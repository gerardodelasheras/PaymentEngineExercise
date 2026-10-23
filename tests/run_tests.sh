#!/usr/bin/env bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/.." && pwd)"

for input in "$DIR"/transactions*.csv; do
  stem="$(basename "$input" .csv)"
  n="${stem#transactions}"
  actual="$DIR/accounts${n}.csv"
  output="$(cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -- "$input")"
  header="${output%%$'\n'*}"
  {
    printf '%s\n' "$header"
    printf '%s\n' "$output" | tail -n +2 | sort -t, -k1,1n
  } > "$actual"
  rel="${actual#"$ROOT"/}"
  git -C "$ROOT" diff --exit-code --quiet -- "$rel" || {
    echo "tests/run_tests.sh: accounts${n}.csv drifted from committed reference" >&2
    git -C "$ROOT" diff --stat -- "$rel" >&2
    exit 1
  }
done

echo "tests/run_tests.sh: all test accounts outputs match the committed references"
