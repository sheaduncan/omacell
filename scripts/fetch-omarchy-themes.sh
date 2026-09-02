#!/usr/bin/env bash
# Copy stock Omarchy theme colors.toml, or verify committed fixtures in CI.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
src="${OMARCHY_THEMES:-/usr/share/omarchy/themes}"
dst="$root/tests/fixtures/omarchy-themes"
if [[ ! -d "$src" ]]; then
  echo "no Omarchy themes at $src" >&2
  exit 1
fi
if [[ ${1:-} == --check ]]; then
  checked=0
  while IFS= read -r fixture; do
    name="$(basename "$(dirname "$fixture")")"
    installed="$src/$name/colors.toml"
    if [[ ! -f "$installed" ]]; then
      echo "committed Omarchy theme is absent from this channel: $name" >&2
      exit 1
    fi
    if ! cmp -s "$fixture" "$installed"; then
      echo "Omarchy theme fixture drifted: $name; run scripts/fetch-omarchy-themes.sh" >&2
      exit 1
    fi
    checked=$((checked + 1))
  done < <(find "$dst" -mindepth 2 -maxdepth 2 -name colors.toml -print | sort)
  echo "verified $checked committed Omarchy theme fixtures"
  exit 0
fi
if [[ $# -ne 0 ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
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
