#!/usr/bin/env bash
# Times hydroid on Polar's server (scripts/corpus.sh clones it): N runs, prints each wall time
# and the median, and saves the last JSON report. Usage: polar_time.sh BINARY [N] [OUT.json]
set -euo pipefail
bin="$1"; n="${2:-3}"; out="${3:-/dev/null}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
polar="$root/target/corpus/polar/server"
times=()
for _ in $(seq "$n"); do
  s=$(date +%s%N)
  "$bin" "$polar" --python "$polar/.venv" --format json > "$out" || true
  e=$(date +%s%N)
  times+=($(( (e - s) / 1000000 )))
done
sorted=($(printf '%s\n' "${times[@]}" | sort -n))
echo "runs ${times[*]} ms  median ${sorted[$(( n / 2 ))]} ms"
