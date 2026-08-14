#!/usr/bin/env bash
set -euo pipefail

RUNS="${QLT_BENCH_RUNS:-3}"
RELEASE_DIR="target/release"
BIN="$RELEASE_DIR/qlt"
BENCH_TMP="$(mktemp -d "${TMPDIR:-/tmp}/qlt-bench.XXXXXX")"
ACTIVE_COMMAND_PID=""
ACTIVE_SAMPLER_PID=""

cleanup() {
  set +e
  if [ -n "$ACTIVE_COMMAND_PID" ]; then kill "$ACTIVE_COMMAND_PID" 2>/dev/null; wait "$ACTIVE_COMMAND_PID" 2>/dev/null; fi
  if [ -n "$ACTIVE_SAMPLER_PID" ]; then kill "$ACTIVE_SAMPLER_PID" 2>/dev/null; wait "$ACTIVE_SAMPLER_PID" 2>/dev/null; fi
  rm -rf "$BENCH_TMP"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

if ! [[ "$RUNS" =~ ^[0-9]+$ ]] || [ "$RUNS" -lt 1 ]; then
  echo "QLT_BENCH_RUNS must be a positive integer" >&2
  exit 1
fi

build_metrics="$BENCH_TMP/build-time.txt"
/usr/bin/time -f '%e %M' -o "$build_metrics" cargo build --release "$@"
read -r build_seconds build_max_rss_kb < "$build_metrics"
build_ms="$(awk -v seconds="$build_seconds" 'BEGIN { printf "%d", seconds * 1000 }')"

if [ ! -x "$BIN" ]; then
  echo "Expected release binary at $BIN" >&2
  exit 1
fi

size_bytes="$(stat -c%s "$BIN")"
build_label="${*:-default}"

managed_temp_bytes() {
  find "$BENCH_TMP" -mindepth 1 -maxdepth 1 -name '.qlt-*' \
    -exec du -sb -- {} + 2>/dev/null | awk '{sum += $1} END {print sum + 0}'
}

measure_cmd() {
  local label="$1"
  shift

  local total_ms=0
  local max_rss_kb=0
  local total_disk_bytes=0
  local max_temp_bytes=0
  local run=1
  while [ "$run" -le "$RUNS" ]; do
    local -a command=("$@")
    local output_path=""
    if [ "$label" = "dump" ]; then
      output_path="$BENCH_TMP/dump-$run.csv"
      rm -f -- "$output_path"
      command+=(--output "$output_path")
    fi
    local metrics_file="$BENCH_TMP/time-$label-$run.txt"
    TMPDIR="$BENCH_TMP" /usr/bin/time -f '%e %M' -o "$metrics_file" \
      "$BIN" "${command[@]}" >/dev/null 2>/dev/null &
    local command_pid=$!
    ACTIVE_COMMAND_PID="$command_pid"
    local temp_metrics="$BENCH_TMP/temp-$label-$run.txt"
    (
      peak=0
      while kill -0 "$command_pid" 2>/dev/null; do
        current="$(managed_temp_bytes)"
        if [ "$current" -gt "$peak" ]; then peak="$current"; fi
        sleep 0.01
      done
      current="$(managed_temp_bytes)"
      if [ "$current" -gt "$peak" ]; then peak="$current"; fi
      printf '%s\n' "$peak" > "$temp_metrics"
    ) &
    local sampler_pid=$!
    ACTIVE_SAMPLER_PID="$sampler_pid"
    local command_status=0
    if wait "$command_pid"; then command_status=0; else command_status=$?; fi
    wait "$sampler_pid" || true
    ACTIVE_COMMAND_PID=""
    ACTIVE_SAMPLER_PID=""
    if [ "$command_status" -ne 0 ]; then
      return "$command_status"
    fi
    local elapsed_seconds rss_kb
    read -r elapsed_seconds rss_kb < "$metrics_file"
    local temp_bytes
    read -r temp_bytes < "$temp_metrics"
    local elapsed_ms
    elapsed_ms="$(awk -v seconds="$elapsed_seconds" 'BEGIN { printf "%d", seconds * 1000 }')"
    total_ms=$((total_ms + elapsed_ms))
    if [ "$rss_kb" -gt "$max_rss_kb" ]; then
      max_rss_kb="$rss_kb"
    fi
    if [ "$temp_bytes" -gt "$max_temp_bytes" ]; then
      max_temp_bytes="$temp_bytes"
    fi
    if [ -n "$output_path" ] && [ -e "$output_path" ]; then
      total_disk_bytes=$((total_disk_bytes + $(stat -c%s -- "$output_path")))
    fi
    run=$((run + 1))
  done

  local avg_ms=$((total_ms / RUNS))
  local avg_disk_bytes=$((total_disk_bytes / RUNS))
  local residual_temp_files
  residual_temp_files="$(find "$BENCH_TMP" -maxdepth 1 -type f -name '.qlt-*' | wc -l)"
  printf '| %s | %s | %s | %s | %s | %s |\n' \
    "$label" "$avg_ms" "$max_rss_kb" "$max_temp_bytes" "$avg_disk_bytes" "$residual_temp_files"
}

echo "# qlt baseline"
echo
echo "- build: \`cargo build --release ${build_label}\`"
echo "- build_time_ms: $build_ms"
echo "- build_max_rss_kb: $build_max_rss_kb"
echo "- runs per command: $RUNS"
echo "- binary: \`$BIN\`"
echo "- size_bytes: $size_bytes"
echo "- timing: \`/usr/bin/time\` wall seconds converted to milliseconds; RSS is maximum"
echo "- peak_temp_bytes: peak sum of execution-owned .qlt-* spool/staging entries sampled during each command; small fixtures may report 0"
echo "- published_output_bytes: average published dump size; execution-owned temporary files are checked for cleanup"
echo
echo "| benchmark | avg_ms | max_rss_kb | peak_temp_bytes | published_output_bytes | residual_temp_files |"
echo "| --- | ---: | ---: | ---: | ---: | ---: |"
measure_cmd "load_show" load tests/fixtures/sample.csv - show
measure_cmd "select_show" load tests/fixtures/sample.csv - select EventId,Level - show
measure_cmd "grep_show" load tests/fixtures/sample.csv - grep Information - show
measure_cmd "bucket_show" load tests/fixtures/sample.csv - cast TimeCreated datetime - bucket TimeCreated 1h - show
measure_cmd "dump" load tests/fixtures/sample.csv - dump
measure_cmd "run" run tests/fixtures/run-bench.yaml
