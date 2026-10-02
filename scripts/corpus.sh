#!/usr/bin/env bash
# Runs hydroid (release) on real async codebases and prints one summary line each.
# Usage: scripts/corpus.sh [workdir] [name...]   (default workdir: target/corpus; default: all)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="${1:-$root/target/corpus}"
shift || true
mkdir -p "$work"
cargo build --release -q -p hydroid --manifest-path "$root/Cargo.toml"
hydroid="$root/target/release/hydroid"

# name | git url | branch | project dir (inside the clone) | environment dir | install command (run there)
corpus=(
  "template|https://github.com/fastapi/full-stack-fastapi-template.git|master|backend|.|uv sync -q"
  "polar|https://github.com/polarsource/polar.git|main|server|server|uv sync -q"
  "mealie|https://github.com/mealie-recipes/mealie.git|mealie-next|.|.|uv sync -q --no-install-package python-ldap"
  "lnbits|https://github.com/lnbits/lnbits.git|dev|.|.|uv sync -q"
  "dispatch|https://github.com/Netflix/dispatch.git|main|.|.|uv venv -q -p 3.12 && uv pip install -q -e ."
  "litellm|https://github.com/BerriAI/litellm.git|main|.|.|uv sync -q --extra proxy"
  "open-webui|https://github.com/open-webui/open-webui.git|main|backend|backend|uv venv -q -p 3.12 && uv pip install -q -r requirements-slim.txt"
)

for entry in "${corpus[@]}"; do
  IFS='|' read -r name url rev project envdir install <<<"$entry"
  if [ $# -gt 0 ] && [[ ! " $* " =~ " $name " ]]; then
    continue
  fi
  clone="$work/$name"
  [ -d "$clone" ] || git clone -q --depth 1 --branch "$rev" "$url" "$clone"
  [ -d "$clone/$envdir/.venv" ] || (cd "$clone/$envdir" && VIRTUAL_ENV=.venv bash -c "$install")
  printf '%-11s ' "$name"
  "$hydroid" "$clone/$project" --python "$clone/$envdir/.venv" --no-cache | tail -n 1 || true
done
