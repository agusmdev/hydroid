#!/usr/bin/env bash
# Runs hydroid (release) on pinned real FastAPI codebases and prints one summary line each.
# Usage: scripts/corpus.sh [workdir]   (default: target/corpus)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="${1:-$root/target/corpus}"
mkdir -p "$work"
cargo build --release -q -p hydroid --manifest-path "$root/Cargo.toml"
hydroid="$root/target/release/hydroid"

# name | git url | commit | project dir (inside the clone) | uv project dir
corpus=(
  "template|https://github.com/fastapi/full-stack-fastapi-template.git|master|backend|."
  "polar|https://github.com/polarsource/polar.git|main|server|server"
)

for entry in "${corpus[@]}"; do
  IFS='|' read -r name url rev project uvdir <<<"$entry"
  clone="$work/$name"
  [ -d "$clone" ] || git clone -q --depth 1 --branch "$rev" "$url" "$clone"
  (cd "$clone/$uvdir" && uv sync -q)
  printf '%-10s ' "$name"
  "$hydroid" "$clone/$project" --python "$clone/$uvdir/.venv" | tail -n 1 || true
done
