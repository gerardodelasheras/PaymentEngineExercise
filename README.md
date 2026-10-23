# Transaction Processor

A fast, stream-based transaction processor written in Rust. It reads a
chronological CSV of deposits, withdrawals, disputes, resolves, and
chargebacks, applies each row to per-client account state, and writes the final
accounts CSV to standard output.

The implementation is intentionally lock-free at the account level: each worker
owns a subset of clients, and a client is always handled by the same worker.

## Usage

```sh
cargo run -- transactions.csv > accounts.csv
```

The binary takes exactly one positional argument: the input CSV path. Standard
output contains only the accounts CSV; diagnostics go to standard error.

Set `RUST_LOG` to control diagnostic verbosity, including full trace logging:

```sh
RUST_LOG=trace cargo run -- transactions.csv > accounts.csv
```

Logs still go to standard error, so stdout remains valid accounts CSV.

## Architecture

`pipeline::process` is the public entry point. It starts a `WorkerPool`, runs
CSV ingestion on a blocking task, then drains each worker's accounts into one
map for output.

```mermaid
flowchart TD
    Input["transactions.csv"] --> Process["pipeline::process"]
    Process -->|"spawn_blocking"| Pipeline["pipeline::read_and_dispatch"]
    Pipeline --> Parser["transaction::parser::transactions"]
    Parser -->|"one Result per row"| Pipeline
    Pipeline --> Dispatcher["ClientDispatcher"]
    Dispatcher -->|"worker index"| Pool["WorkerPool"]
    Pool -->|"Command::Process"| W1["worker task<br/>TransactionProcessor"]
    Pool -->|"Command::Process"| WN["worker task<br/>TransactionProcessor"]
    W1 --> State1["accounts + entries"]
    WN --> StateN["accounts + entries"]
    Process -->|"producer completes"| Collect["WorkerPool::take_accounts"]
    Collect --> Report["account::report::write_accounts"]
    Report --> Output["stdout: accounts.csv"]
```

Key properties:

- **Single-pass streaming.** `transaction::parser::transactions` yields one
  `Result<Transaction, ProcessingError>` per CSV row. The reader does not load
  the full input into memory.
- **Client affinity.** `ClientDispatcher` assigns a client to a worker the first
  time it sees that client. New clients go to the worker with the fewest
  assigned clients, then the largest remaining queue capacity, then round-robin
  order. Existing clients stay pinned.
- **Bounded backpressure.** Each worker has a bounded `mpsc` queue
  (`QUEUE_CAPACITY = 256`). The producer uses `blocking_send`; when a queue is
  full, the reader waits instead of growing the backlog.
- **Worker-local state.** Each `TransactionProcessor` owns its
  `HashMap<ClientId, Account>` and `HashMap<TransactionId, TransactionEntry>`.
  No account is mutated by more than one worker, so no locks are needed.
- **Fixed-point currency.** Every amount is an `i64` scaled by `10_000`
  (`1.23` is stored as `12300`). No floating point or decimal crate is used in
  the ledger or amount path.
- **Order-independent output.** Row order is unspecified. Worker assignment is
  not visible in stdout.

## Data Flow & Domain Entities

```mermaid
flowchart LR
    subgraph Input["Parsed input row"]
        Tx["Transaction"]
        Tx --> Typ["transaction_type<br/>TransactionType"]
        Tx --> Cid["client<br/>ClientId = u16"]
        Tx --> Tid["transaction_id<br/>TransactionId = u32"]
        Tx --> Amt["amount<br/>Option(Amount)"]
    end

    subgraph State["Worker-local state"]
        Proc["TransactionProcessor::process"]
        Entries["entries<br/>TransactionId -> TransactionEntry"]
        Accounts["accounts<br/>ClientId -> Account"]
    end

    Typ --> Proc
    Cid --> Proc
    Tid --> Proc
    Amt --> Proc
    Proc -->|"deposit / withdrawal"| Entries
    Proc -->|"dispute / resolve / chargeback"| Entries
    Proc -->|"credit / debit / hold / release"| Accounts

    subgraph Output["Output row"]
        Acc["Account"]
        Acc --> O1["client"]
        Acc --> O2["available: Amount"]
        Acc --> O3["held: Amount"]
        Acc --> O4["total(): Amount"]
        Acc --> O5["locked: bool"]
    end

    Accounts --> Acc
```

The core types are:

- `Transaction` — a parsed input row: `transaction_type`, `client`,
  `transaction_id`, and optional `amount`.
- `TransactionType` — `Deposit`, `Withdrawal`, `Dispute`, `Resolve`, or
  `Chargeback`.
- `TransactionEntry` — an accepted transaction in the worker's transaction index;
  `Settled` or `Disputed`.
- `Account` — a client's ledger state: `available`, `held`, and `locked`.
  `Account::total()` is always `available + held`.
- `ProcessingError` — the crate's unified error type for row-level validation,
  ledger errors, I/O, and worker failures.

### Operation effects

| Operation | Amount | Account effect | Transaction entry effect |
|---|---|---|---|
| `deposit` | required | `available += amount` | Insert `Settled` |
| `withdrawal` | required | Reject when `available < amount`; otherwise `available -= amount` | Insert `Settled` |
| `dispute` | absent | `available -= amount`, `held += amount` | `Settled` -> `Disputed` |
| `resolve` | absent | `available += amount`, `held -= amount` | `Disputed` -> `Settled` |
| `chargeback` | absent | `held -= amount`, then `locked = true` | Remove the entry; terminal |

See [Notes](#notes) for documented assumptions and edge-case behavior.

## State Pattern Trade-offs

The transaction index uses an enum-based state carried inline in
`TransactionEntry`:

| Pattern | Memory and collections | Compile-time safety | Runtime transitions | Decision |
|---|---|---|---|---|
| Enum-based `TransactionEntry` | Fixed-size variants stored directly in one `HashMap<TransactionId, TransactionEntry>`; no heap allocation per state | Exhaustive `match`; wrong-state transitions are rejected by the processor at runtime | In-place `insert`/`remove`; `dispute` and `resolve` rewrite the same map slot | Selected |
| Typestate (`TransactionEntry<Settled>`, `TransactionEntry<Disputed>`, ...) | Requires distinct types and reallocation per transition; mixed states cannot live in one homogeneous map without erasure | Illegal states are unrepresentable | Transitions consume and rebuild values; awkward for a shared historical index | Skipped |
| Dynamic dispatch (`Box<dyn State>`) | Heap allocation and pointer chasing per entry | Limited to trait-level guarantees | Flexible, but adds indirection and allocation | Skipped |

The enum approach matches the workload: the input is processed chronologically,
historical transactions must remain in a shared lookup map for disputes, and
transitions happen in place. It keeps entries uniform in size and the worker's
state cache-friendly.

## Specifications

### Input schema

The input CSV has a header row and the columns `type, client, tx, amount`.
Rows are trimmed, and the parser tolerates flexible column counts.

| Column | Type | Rules |
|---|---|---|
| `type` | string | One of `deposit`, `withdrawal`, `dispute`, `resolve`, `chargeback` |
| `client` | `u16` | Client identifier |
| `tx` | `u32` | Transaction identifier |
| `amount` | decimal string | Required for `deposit` and `withdrawal`; empty or absent for the other types. Up to four fraction digits, non-negative, leading dot allowed (`.5` = `0.5`) |

Example:

```csv
type,client,tx,amount
deposit,1,1,1.0
withdrawal,1,2,0.5
```

Rows that cannot be parsed or validated are skipped with a diagnostic. This
includes unknown types, missing or unexpected amounts, invalid UTF-8, invalid
client or transaction IDs, malformed decimals, negatives, more than four
fraction digits, and overflow.

### Output schema

The output is `client,available,held,total,locked`, one row per initialized
client. Row order is unspecified.

| Column | Type | Notes |
|---|---|---|
| `client` | `u16` | Client identifier |
| `available` | decimal | Funds available for withdrawal |
| `held` | decimal | Funds frozen by active disputes |
| `total` | decimal | Always `available + held` |
| `locked` | boolean | `true` after a chargeback |

Amounts are rendered with trailing zeros stripped and at least one decimal
place (`1.2300` -> `1.23`, `0.5000` -> `0.5`, `0` -> `0.0`).

Example:

```csv
client,available,held,total,locked
1,0.5,0.0,0.5,false
```

## Tested Scenarios

### Unit tests

- `src/amount.rs` covers fixed-point parsing and formatting: whole and fractional
  amounts, whitespace trimming, varied fraction lengths, leading-dot fractions,
  negative rejection, too many decimals, malformed input, `i64` overflow,
  checked arithmetic, `Display`, and operator overflow behavior.
- `src/account/account.rs` covers the zeroed default, the
  `total = available + held` invariant, and overflow handling that leaves
  balances unchanged.
- `src/transaction/transaction_processor.rs` covers every state-machine path:
  deposit and withdrawal entries, successful and insufficient withdrawals,
  dispute holds funds (including negative available balances), unknown
  transactions, duplicate disputes, resolve,
  invalid resolve, chargeback, invalid chargeback, locked-account rejection,
  cross-client disputes, and zeroed initialization of unknown clients.
- `src/client/dispatcher.rs` covers assigned-count priority, capacity
  tie-breaking, round-robin tie-breaking, spreading new clients across workers,
  and stable client pinning.
- `src/worker_pool.rs` covers remaining queue capacity, closed dispatch and
  collection channels, and backpressure when a queue is full.

### Integration tests

`tests/integration.rs` includes an independent reference oracle written in plain
`i64` arithmetic and covers:

- 100 seeded random transaction sequences compared against the oracle.
- A 10,000-row generated input compared against the oracle.
- Table-driven edge cases: insufficient funds, dispute/resolve, chargeback
  locking, cross-client disputes, unknown transactions, duplicate disputes,
  resolving a non-disputed transaction, varying decimal precision, and
  leading-dot amounts.
- A header-only input producing only the output header.
- Oversized fields: amount overflow, client IDs beyond `u16`, and transaction
  IDs beyond `u32`.
- Strange characters and non-ASCII digits, plus invalid UTF-8 bytes.
- Malformed rows followed by valid rows, proving the stream continues.

### Optional end-to-end regression tests

Run the full test suites with:

```sh
cargo test
./tests/run_tests.sh
```

`cargo test` runs the unit and integration suites. The optional
`tests/run_tests.sh` script regenerates `tests/accounts*.csv` from the committed
CSV test cases and fails if any output differs from its reference file.

The `tests/` directory includes three committed CSV cases:

- `tests/transactions0.csv` and `tests/transactions1.csv` — 120 rows each, five
  clients, space-padded columns, and all five operation types.
- `tests/transactions2.csv` — 100 rows across five clients, covering
  every operation, dispute/resolve pairs, a chargeback, and a post-lock
  rejection.

Reference outputs live alongside them as `tests/accounts0.csv`,
`tests/accounts1.csv`, and `tests/accounts2.csv`. `./tests/run_tests.sh`
normalizes row order and verifies the rows agree.

To add another case, commit `tests/transactionsN.csv` and its matching
`tests/accountsN.csv`; the runner discovers every `transactions*.csv`
automatically.

## Correctness

- **Exact money.** Amounts are scaled `i64` values; floating point is absent from
  the parser, ledger, and formatter.
- **Distinct identifiers.** `ClientId` is `u16`; `TransactionId` is `u32`. They
  cannot be accidentally interchanged.
- **Recoverable ledger arithmetic.** Balance mutations use checked
  `credit`/`debit`/`hold`/`release`/`chargeback` paths. Overflow becomes a
  `ProcessingError` and the row is skipped. `Amount`'s `Add`/`Sub` operators are
  convenience APIs and panic on overflow, so row processing uses the checked
  methods.
- **Independent verification.** The integration oracle shares no implementation
  with the processor. A disagreement is a real finding, not two copies of the
  same bug.
- **Regression tests.** `./tests/run_tests.sh` normalizes row order, then
  compares the processor's output against committed references with
  `git diff --exit-code`.

## Error Handling

Row-level failures are logged and skipped so the stream can continue. Examples
include `InvalidType`, `InvalidAmount`, `NegativeAmount`, `TooManyDecimals`,
`MissingAmount`, `UnexpectedAmount`, `InsufficientFunds`, `AccountLocked`,
`UnknownTx`, `NotOwner`, `Overflow`, and CSV deserialization errors.

Infrastructure failures fail the run instead of producing silently incomplete
output:

- `WorkerUnavailable` when the producer or a worker channel closes.
- `Io` when writing the accounts CSV fails.
- `Csv` when reading or deserializing the input CSV fails.

`main` prints the error to stderr and exits with a non-zero status.

## Notes

### Assumptions

- **Disputes may overdraw `available`.** The specification requires a dispute to
  decrease `available` by the disputed amount and increase `held`, keeping
  `total` unchanged. It does not define an insufficient-funds failure for
  disputes. This processor follows that literal rule, so `available` may become
  negative while `total = available + held` still holds. For example, a deposit
  of `100`, a withdrawal of `80`, and a dispute of the deposit produce
  `available = -80`, `held = 100`, and `total = 20`.
- **Already-disputed transactions are ignored.** A second dispute on a
  transaction already in `Disputed` state is skipped. The specification only
  explicitly requires ignoring unknown transactions; rejecting a transaction
  that is already under dispute follows conservative account behavior.
- **Cross-client references are ignored.** A dispute, resolve, or chargeback
  must reference a transaction owned by the same client. A mismatched owner is
  skipped with `NotOwner`.
- **Unknown clients may be initialized by rejected rows.** A parsed transaction
  creates a zeroed account for its client before the operation is validated, so
  a client can appear in the final report even when its first operation is
  skipped.
- **Non-monetary rows must not carry an amount.** A `dispute`, `resolve`, or
  `chargeback` with a non-empty `amount` is skipped with `UnexpectedAmount`; the
  specification says these rows omit the amount.
- **Resolved transactions can be disputed again.** `resolve` returns the entry
  to `Settled`, so a later `dispute` can hold the same funds again. The
  specification does not explicitly forbid a second dispute after resolution.
- **Locked accounts reject all later operations.** A successful chargeback sets
  `locked = true`; every later row for that client is skipped with
  `AccountLocked`. This follows the frozen-account behavior in the
  specification.

### Edge behavior

- A chargeback removes the transaction entry; the entry is terminal.
- An unknown transaction or a wrong-state reference is skipped with `UnknownTx`;
  the stream continues.

## Project Layout

```text
src/                          Shared library and binary entry points
  account/                    Account state and reporting
    account.rs                Client account and ledger operations
    report.rs                 Accounts CSV writer
  client/                     Client identity and routing
    dispatcher.rs             Least-loaded client-to-worker assignment
    id.rs                     ClientId alias
  transaction/                Transaction model, parsing, and processing
    parser.rs                 CSV parsing and validation
    transaction.rs            Transaction input row
    transaction_entry.rs      Settled/Disputed lifecycle entry
    transaction_id.rs         TransactionId alias
    transaction_processor.rs  Per-worker account and entry state machine
    transaction_type.rs       Deposit/withdrawal/dispute operation enum
  amount.rs                   Fixed-point i64 currency and formatting
  lib.rs                      Public crate exports
  main.rs                     CLI entry point
  pipeline.rs                 Streaming ingestion, dispatch, and output
  processing_error.rs         Recoverable and infrastructure errors
  worker_pool.rs              Bounded queues and worker lifecycle
tests/                        Integration tests and optional CSV regression cases
  integration.rs              Seeded oracle and edge-case tests
  run_tests.sh                Optional CSV regression runner
  transactions*.csv           Committed transaction inputs
  accounts*.csv               Reference account outputs
```

## AI Assistance

This implementation was developed with AI assistance using:

- **opencode** — agentic coding environment
- **DeepSeek** — language model used for implementation and review
- **Minimax** — language model used for implementation and review
- **intent-driven-development** — change planning and execution workflow
- **rust-skills** — Rust coding guidelines and review rules
