#!/usr/bin/env bash
# Copy stock Omarchy theme colors.toml into tests/fixtures/omarchy-themes/.
# Human-run only (needs a local Omarchy install). Not invoked by CI.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
src="${OMARCHY_THEMES:-/usr/share/omarchy/themes}"
dst="$root/tests/fixtures/omarchy-themes"
if [[ ! -d "$src" ]]; then
  echo "no Omarchy themes at $src" >&2
  exit 1
fi
shopt -s nullglob
copied=0
for colors in "$src"/*/colors.toml; do
  name="$(basename "$(dirname "$colors")")"
  mkdir -p "$dst/$name"
  cp -f "$colors" "$dst/$name/colors.toml"
  copied=$((copied + 1))
done
echo "copied $copied colors.toml files into $dst"
