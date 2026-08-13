#!/usr/bin/env bash
set -euo pipefail

RUNS="${QLT_BENCH_RUNS:-3}"
RELEASE_DIR="target/release"
BIN="$RELEASE_DIR/qlt"
BENCH_TMP="$(mktemp -d "${TMPDIR:-/tmp}/qlt-bench.XXXXXX")"
trap 'rm -rf "$BENCH_TMP"' EXIT

if ! [[ "$RUNS" =~ ^[0-9]+$ ]] || [ "$RUNS" -lt 1 ]; then
  echo "QLT_BENCH_RUNS must be a positive integer" >&2
  exit 1
fi

cargo build --release "$@"

if [ ! -x "$BIN" ]; then
  echo "Expected release binary at $BIN" >&2
  exit 1
fi

size_bytes="$(stat -c%s "$BIN")"
build_label="${*:-default}"

measure_cmd() {
  local label="$1"
  shift

  local total_ns=0
  local run=1
  while [ "$run" -le "$RUNS" ]; do
    local start_ns
    local end_ns
    start_ns="$(date +%s%N)"
    local -a command=("$@")
    if [ "$label" = "dump" ]; then
      command+=(--output "$BENCH_TMP/dump-$run.csv")
    fi
    "$BIN" "${command[@]}" >/dev/null 2>/dev/null
    end_ns="$(date +%s%N)"
    total_ns=$((total_ns + end_ns - start_ns))
    run=$((run + 1))
  done

  local avg_ns=$((total_ns / RUNS))
  local avg_ms=$((avg_ns / 1000000))
  printf '| %s | %s |\n' "$label" "$avg_ms"
}

echo "# qlt baseline"
echo
echo "- build: \`cargo build --release ${build_label}\`"
echo "- runs per command: $RUNS"
echo "- binary: \`$BIN\`"
echo "- size_bytes: $size_bytes"
echo
echo "| benchmark | avg_ms |"
echo "| --- | ---: |"
measure_cmd "load_show" load tests/fixtures/sample.csv - show
measure_cmd "select_show" load tests/fixtures/sample.csv - select EventId,Level - show
measure_cmd "grep_show" load tests/fixtures/sample.csv - grep Information - show
measure_cmd "bucket_show" load tests/fixtures/sample.csv - cast TimeCreated datetime - bucket TimeCreated 1h - show
measure_cmd "dump" load tests/fixtures/sample.csv - dump
measure_cmd "run" run tests/fixtures/run-bench.yaml
