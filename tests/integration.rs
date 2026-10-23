//! Integration tests: a reference oracle, a seeded CSV driver, end-to-end
//! processor runs, and table-driven edge cases. All amounts use generated values
//! that differ from the specification examples.

use std::collections::HashMap;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Run the processor over an in-memory CSV string and return its normalized
/// stdout.
async fn run_processor(input: &str) -> String {
    run_processor_bytes(input.as_bytes()).await
}

/// Run the processor over raw bytes and return its normalized stdout, so inputs
/// can include invalid UTF-8 that a `&str` could not represent.
async fn run_processor_bytes(input: &[u8]) -> String {
    let mut out = Vec::new();
    transaction_processor::pipeline::process(std::io::Cursor::new(input.to_vec()), &mut out)
        .await
        .expect("processor should not fail");
    normalize_account_rows(&String::from_utf8(out).expect("output is utf-8"))
}

/// Sort account rows by client ID so assertions do not depend on output order.
fn normalize_account_rows(output: &str) -> String {
    let mut lines = output.lines();
    let header = lines.next().unwrap_or_default();
    let mut rows: Vec<&str> = lines.collect();
    rows.sort_unstable_by_key(|row| {
        row.split(',')
            .next()
            .and_then(|client| client.parse::<u16>().ok())
    });

    let mut normalized = String::from(header);
    normalized.push('\n');
    for row in rows {
        normalized.push_str(row);
        normalized.push('\n');
    }
    normalized
}

/// Dispute lifecycle state for the oracle's transaction index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Settled,
    UnderDispute,
    Chargebacked,
}

/// Independent reference model. Parses the CSV by hand and folds over the
/// five operations using plain `i64` arithmetic. Returns
/// `client -> (available, held, locked)`.
fn oracle(csv: &str) -> HashMap<u16, (i64, i64, bool)> {
    let mut accounts: HashMap<u16, (i64, i64, bool)> = HashMap::new();
    let mut txs: HashMap<u32, (u16, i64, State)> = HashMap::new(); // (client, amount, state)

    for line in csv.lines().skip(1) {
        let f: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if f.len() < 3 {
            continue;
        }
        let kind = f[0];
        let client: u16 = match f[1].parse() {
            Ok(c) => c,
            Err(_) => continue,
        };
        let tx: u32 = match f[2].parse() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let amount_str = f.get(3).copied().unwrap_or("");

        // Validate and parse the amount per type, skipping the row (without
        // initializing an account) on any failure or unknown kind.
        let amount: Option<i64> = match kind {
            "deposit" | "withdrawal" => match parse_amount(amount_str) {
                Some(a) => Some(a),
                None => continue,
            },
            "dispute" | "resolve" | "chargeback" => {
                if !amount_str.is_empty() {
                    continue;
                }
                None
            }
            _ => continue,
        };

        let acct = accounts.entry(client).or_insert((0, 0, false));
        if acct.2 {
            continue; // locked accounts reject everything
        }

        match kind {
            "deposit" => {
                let a = amount.unwrap();
                acct.0 += a;
                txs.insert(tx, (client, a, State::Settled));
            }
            "withdrawal" => {
                let a = amount.unwrap();
                if acct.0 < a {
                    continue;
                }
                acct.0 -= a;
                txs.insert(tx, (client, a, State::Settled));
            }
            "dispute" => {
                if let Some(&(c, a, st)) = txs.get(&tx) {
                    if c == client && st == State::Settled {
                        acct.0 -= a;
                        acct.1 += a;
                        txs.get_mut(&tx).unwrap().2 = State::UnderDispute;
                    }
                }
            }
            "resolve" => {
                if let Some(&(c, a, st)) = txs.get(&tx) {
                    if c == client && st == State::UnderDispute {
                        acct.1 -= a;
                        acct.0 += a;
                        txs.get_mut(&tx).unwrap().2 = State::Settled;
                    }
                }
            }
            "chargeback" => {
                if let Some(&(c, a, st)) = txs.get(&tx) {
                    if c == client && st == State::UnderDispute {
                        acct.1 -= a;
                        txs.get_mut(&tx).unwrap().2 = State::Chargebacked;
                        acct.2 = true;
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    accounts
}

/// Decimal parser used by the oracle (independent of the production parser).
/// Rejects negatives and more than four fraction digits.
fn parse_amount(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('-') {
        return None;
    }
    let (int_s, frac_s) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let int_v: i64 = if int_s.is_empty() {
        // Accept a leading decimal point (".5" == "0.5"); require fraction digits.
        match frac_s {
            Some(f) if !f.is_empty() => 0,
            _ => return None,
        }
    } else {
        int_s.parse().ok()?
    };
    let mut v = int_v.checked_mul(10_000)?;
    if let Some(fr) = frac_s {
        if fr.len() > 4 {
            return None;
        }
        if !fr.is_empty() {
            if !fr.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let fv: i64 = fr.parse().ok()?;
            v = v.checked_add(fv.checked_mul(10_i64.pow((4 - fr.len()) as u32))?)?;
        }
    }
    Some(v)
}

/// Parse a processor output line's amount (may be negative).
fn parse_output_amount(s: &str) -> i64 {
    if let Some(rest) = s.strip_prefix('-') {
        -parse_amount(rest).expect("negative output amount parses")
    } else {
        parse_amount(s).expect("output amount parses")
    }
}

/// Parse the processor's CSV output into `client -> (available, held, total, locked)`.
fn parse_processor_output(out: &str) -> HashMap<u16, (i64, i64, i64, bool)> {
    let mut map = HashMap::new();
    for line in out.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 5 {
            continue;
        }
        let client: u16 = f[0].parse().expect("client parses");
        let available = parse_output_amount(f[1]);
        let held = parse_output_amount(f[2]);
        let total = parse_output_amount(f[3]);
        let locked = f[4] == "true";
        map.insert(client, (available, held, total, locked));
    }
    map
}

/// Generate a random, well-formed transaction CSV for a fixed seed.
fn generate_csv(seed: u64) -> String {
    let mut rng = StdRng::seed_from_u64(seed);

    let mut rows: Vec<String> = Vec::new();
    let mut settled: Vec<(u16, u32)> = Vec::new();
    let mut disputed: Vec<(u16, u32)> = Vec::new();
    let mut tx_counter: u32 = 1;

    let n = rng.gen_range(50..300);
    for _ in 0..n {
        let roll: f64 = rng.gen();
        if roll < 0.65 {
            // deposit or withdrawal
            let client = rng.gen_range(0u16..64);
            let kind = if roll < 0.35 { "deposit" } else { "withdrawal" };
            let amount = random_amount(&mut rng);
            rows.push(format!("{kind},{client},{tx_counter},{amount}"));
            settled.push((client, tx_counter));
            tx_counter += 1;
        } else if roll < 0.80 {
            if !settled.is_empty() {
                let idx = rng.gen_range(0..settled.len());
                let (client, tx) = settled.remove(idx);
                rows.push(format!("dispute,{client},{tx},"));
                disputed.push((client, tx));
            }
        } else if roll < 0.92 {
            if !disputed.is_empty() {
                let idx = rng.gen_range(0..disputed.len());
                let (client, tx) = disputed.remove(idx);
                rows.push(format!("resolve,{client},{tx},"));
                settled.push((client, tx));
            }
        } else if !disputed.is_empty() {
            let idx = rng.gen_range(0..disputed.len());
            let (client, tx) = disputed.remove(idx);
            rows.push(format!("chargeback,{client},{tx},"));
        }
    }

    let mut csv = String::from("type,client,tx,amount\n");
    for row in rows {
        csv.push_str(&row);
        csv.push('\n');
    }
    csv
}

/// Produce a random decimal amount string with 0..=4 fraction digits.
fn random_amount(rng: &mut StdRng) -> String {
    let int: u32 = rng.gen_range(0..1000);
    let frac_digits: u32 = rng.gen_range(0..5);
    if frac_digits == 0 {
        return int.to_string();
    }
    let frac: u32 = rng.gen_range(0..10_u32.pow(frac_digits));
    let frac_full = format!("{frac:04}");
    let frac_str = &frac_full[(4 - frac_digits as usize)..];
    format!("{int}.{frac_str}")
}

#[tokio::test]
async fn generated_sequences_match_oracle() {
    for seed in 0..100u64 {
        let csv = generate_csv(seed);
        let expected = oracle(&csv);
        let out = run_processor(&csv).await;
        let actual = parse_processor_output(&out);

        assert_eq!(
            actual.len(),
            expected.len(),
            "seed {seed}: client count differs"
        );
        for (&client, &(avail, held, total, locked)) in &actual {
            let &(e_avail, e_held, e_locked) = expected
                .get(&client)
                .unwrap_or_else(|| panic!("seed {seed}: unexpected client {client}"));
            assert_eq!(avail, e_avail, "seed {seed}: client {client} available");
            assert_eq!(held, e_held, "seed {seed}: client {client} held");
            assert_eq!(
                total,
                e_avail + e_held,
                "seed {seed}: client {client} total"
            );
            assert_eq!(locked, e_locked, "seed {seed}: client {client} locked");
        }
    }
}

#[tokio::test]
async fn invalid_row_is_skipped_and_stream_continues() {
    let input = concat!(
        "type,client,tx,amount\n",
        "bogus,1,1,5.00\n",
        "deposit,1,2,12.34\n",
    );
    let out = run_processor(input).await;

    assert_eq!(
        out,
        concat!(
            "client,available,held,total,locked\n",
            "1,12.34,0.0,12.34,false\n",
        )
    );
}

#[tokio::test]
async fn large_generated_input_matches_oracle() {
    let mut csv = String::from("type,client,tx,amount\n");
    for tx in 0..10_000u32 {
        let client = (tx % 64) as u16;
        let amount = (tx % 97) + 1;
        csv.push_str(&format!("deposit,{client},{tx},{amount}\n"));
    }

    let expected = oracle(&csv);
    let out = run_processor(&csv).await;
    let actual = parse_processor_output(&out);

    assert_eq!(actual.len(), expected.len());
    for (&client, &(available, held, total, locked)) in &actual {
        let &(expected_available, expected_held, expected_locked) = expected
            .get(&client)
            .unwrap_or_else(|| panic!("unexpected client {client}"));
        assert_eq!(available, expected_available, "client {client} available");
        assert_eq!(held, expected_held, "client {client} held");
        assert_eq!(
            total,
            expected_available + expected_held,
            "client {client} total"
        );
        assert_eq!(locked, expected_locked, "client {client} locked");
    }
}

#[tokio::test]
async fn edge_cases_produce_expected_output() {
    let cases: Vec<(&str, &str)> = vec![
        (
            // Insufficient funds are skipped.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "withdrawal,     1,     2,    200.00\n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,100.0,0.0,100.0,false\n",
            ),
        ),
        (
            // Dispute then resolve restores balances.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "dispute   ,     1,     1,          \n",
                "resolve   ,     1,     1,          \n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,100.0,0.0,100.0,false\n",
            ),
        ),
        (
            // Chargeback withdraws held funds, locks, and rejects further rows.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "dispute   ,     1,     1,          \n",
                "chargeback,     1,     1,          \n",
                "deposit   ,     1,     2,     50.00\n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,0.0,0.0,0.0,true\n",
            ),
        ),
        (
            // Cross-client dispute is skipped; both clients appear.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "dispute   ,     2,     1,          \n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,100.0,0.0,100.0,false\n",
                "2,0.0,0.0,0.0,false\n",
            ),
        ),
        (
            // Dispute of an unknown transaction is skipped.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "dispute   ,     1,    99,          \n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,100.0,0.0,100.0,false\n",
            ),
        ),
        (
            // A second dispute on an already-disputed transaction is skipped.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "dispute   ,     1,     1,          \n",
                "dispute   ,     1,     1,          \n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,0.0,100.0,100.0,false\n",
            ),
        ),
        (
            // Resolve of a non-disputed transaction is skipped.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,    100.00\n",
                "resolve   ,     1,     1,          \n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,100.0,0.0,100.0,false\n",
            ),
        ),
        (
            // Round and varied decimals parse and format correctly.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,       100\n",
                "deposit   ,     1,     2,      25.5\n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,125.5,0.0,125.5,false\n",
            ),
        ),
        (
            // A fractional amount without a leading zero is accepted.
            concat!(
                "type      ,client,    tx,    amount\n",
                "deposit   ,     1,     1,        .5\n",
            ),
            concat!(
                "client,available,held,total,locked\n",
                "1,0.5,0.0,0.5,false\n",
            ),
        ),
    ];

    for (i, (input, expected)) in cases.iter().enumerate() {
        let out = run_processor(input).await;
        assert_eq!(&out, expected, "edge case {i} failed");
    }
}

#[tokio::test]
async fn header_is_written_even_with_no_data_rows() {
    let out = run_processor("type,client,tx,amount\n").await;
    assert_eq!(out, "client,available,held,total,locked\n");
}

#[tokio::test]
async fn oversized_fields_are_skipped() {
    let cases = [
        // Amount overflows the fixed-point i64 representation.
        "type,client,tx,amount\ndeposit,1,1,9223372036854775807\n",
        // Client id exceeds u16.
        "type,client,tx,amount\ndeposit,70000,1,100.00\n",
        // Transaction id exceeds u32.
        "type,client,tx,amount\ndeposit,1,5000000000,100.00\n",
    ];

    for input in cases {
        let out = run_processor(input).await;
        assert_eq!(out, "client,available,held,total,locked\n");
    }
}

#[tokio::test]
async fn strange_characters_are_skipped() {
    let cases = [
        // Unknown type with a non-ASCII character.
        "type,client,tx,amount\ndepos\u{00e9}t,1,1,100.00\n",
        // Amount containing a currency symbol.
        "type,client,tx,amount\ndeposit,1,1,1\u{20ac}0\n",
        // Amount using full-width digits.
        "type,client,tx,amount\ndeposit,1,1,\u{ff11}\u{ff10}.00\n",
        // Amount containing a replacement character (mojibake).
        "type,client,tx,amount\ndeposit,1,1,1\u{fffd}0\n",
        // Client id that is not numeric.
        "type,client,tx,amount\ndeposit,abc,1,100.00\n",
    ];

    for input in cases {
        let out = run_processor(input).await;
        assert_eq!(out, "client,available,held,total,locked\n");
    }
}

#[tokio::test]
async fn invalid_utf8_is_skipped() {
    let cases: &[&[u8]] = &[
        // Invalid UTF-8 bytes inside the amount field.
        b"type,client,tx,amount\ndeposit,1,1,100\xff\xfe.00\n",
        // Invalid UTF-8 bytes inside the type field.
        b"type,client,tx,amount\ndep\xffsit,1,1,100.00\n",
    ];

    for input in cases {
        let out = run_processor_bytes(input).await;
        assert_eq!(out, "client,available,held,total,locked\n");
    }
}
