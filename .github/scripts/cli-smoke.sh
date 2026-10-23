#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: cli-smoke.sh <qlt-binary>" >&2
  exit 2
fi

binary_path="$1"
binary="$(cd "$(dirname "$binary_path")" && pwd)/$(basename "$binary_path")"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
package_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)"
binary_version="$("$binary" --version)"
if [[ "$binary_version" != "$package_version" ]]; then
  echo "binary reports $binary_version, expected Cargo package version $package_version" >&2
  exit 1
fi
if [[ -n "${RELEASE_TAG:-}" ]]; then
  tag_version="${RELEASE_TAG#v}"
  if [[ "$tag_version" != "$package_version" ]]; then
    echo "release tag $RELEASE_TAG does not match Cargo package version $package_version" >&2
    exit 1
  fi
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Check a nested JSONL value through the CLI's actual CSV output.
cat >"$work/nested.jsonl" <<'JSONL'
{"id":17,"event":{"user":"alice","active":true}}
JSONL
"$binary" load "$work/nested.jsonl" - flatten - show >"$work/nested.csv"
grep -F 'event.user' "$work/nested.csv" >/dev/null
grep -F 'alice' "$work/nested.csv" >/dev/null

# Carry the CSV through Parquet and back to the CLI's CSV output.
"$binary" load "$work/nested.jsonl" - flatten - dump -o "$work/roundtrip.csv"
"$binary" load "$work/roundtrip.csv" - dumpcache -o "$work/roundtrip.parquet"
"$binary" load "$work/roundtrip.parquet" - show >"$work/from-parquet.csv"
grep -F 'event.user' "$work/from-parquet.csv" >/dev/null
grep -F 'alice' "$work/from-parquet.csv" >/dev/null

# Exercise YAML workflow loading, strict datetime conversion, and file output.
cat >"$work/dates.csv" <<'CSV'
id,when
1,2024-01-02T03:04:05Z
CSV
cat >"$work/datetime.yaml" <<'YAML'
version: 2
stages:
  - name: output
    steps:
      - load:
          paths: [dates.csv]
      - cast:
          column: when
          type: datetime
          strict: true
      - dump:
          output: dates-out.csv
YAML
(cd "$work" && "$binary" run datetime.yaml)
grep -F 'when' "$work/dates-out.csv" >/dev/null
grep -F '2024-01-02' "$work/dates-out.csv" >/dev/null

# A strict conversion failure must fail before writing machine output.
cat >"$work/invalid.csv" <<'CSV'
value
not-an-integer
CSV
if "$binary" load "$work/invalid.csv" - cast value int - show >"$work/failure.out" 2>"$work/failure.err"; then
  echo "invalid strict conversion unexpectedly succeeded" >&2
  exit 1
fi
if [[ -s "$work/failure.out" ]]; then
  echo "invalid strict conversion wrote partial output" >&2
  cat "$work/failure.out" >&2
  exit 1
fi
grep -i 'conversion' "$work/failure.err" >/dev/null
