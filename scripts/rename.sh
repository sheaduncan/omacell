#!/bin/bash
# One-shot repository identity rename. Run only from a clean worktree.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'usage: %s new-product-slug "Display Name"\n' "$0" >&2
  exit 2
fi

new_slug=$1
new_display=$2
if [[ ! $new_slug =~ ^[a-z][a-z0-9-]*$ ]]; then
  printf 'slug must match [a-z][a-z0-9-]*\n' >&2
  exit 2
fi
if [[ ! $new_display =~ ^[A-Za-z0-9._\ -]+$ ]]; then
  printf 'display name contains an unsupported character\n' >&2
  exit 2
fi

root=$(git rev-parse --show-toplevel)
cd "$root"
if [[ -n $(git status --porcelain) ]]; then
  printf 'rename requires a clean worktree\n' >&2
  exit 1
fi
test -f packaging/name.env
test -f crates/core/src/product.rs

# shellcheck disable=SC1091 -- repository-owned identity data
source packaging/name.env
if [[ ! $PRODUCT_NAME =~ ^[a-z][a-z0-9-]*$ ]]; then
  printf 'packaging/name.env has an invalid PRODUCT_NAME\n' >&2
  exit 1
fi
if [[ -z $PRODUCT_DISPLAY_NAME ]]; then
  printf 'packaging/name.env has an empty PRODUCT_DISPLAY_NAME\n' >&2
  exit 1
fi
old_install_hash=$(sha256sum "packaging/${PRODUCT_NAME}.install" | cut -d ' ' -f 1)

new_upper=${new_slug^^}
new_upper=${new_upper//-/_}
while IFS= read -r -d '' path; do
  sed -i \
    -e "s/${PRODUCT_NAME^^}/${new_upper}/g" \
    -e "s/\\<${PRODUCT_DISPLAY_NAME}\\>/${new_display}/g" \
    -e "s/${PRODUCT_NAME}/${new_slug}/g" \
    "$path"
done < <(git grep -Ilz -e "${PRODUCT_NAME^^}" -e "$PRODUCT_DISPLAY_NAME" -e "$PRODUCT_NAME")

while IFS= read -r -d '' path; do
  renamed=${path//$PRODUCT_NAME/$new_slug}
  if [[ $renamed != "$path" ]]; then
    install -d "$(dirname "$renamed")"
    git mv "$path" "$renamed"
  fi
done < <(git ls-files -z "*${PRODUCT_NAME}*")

new_install_hash=$(sha256sum "packaging/${new_slug}.install" | cut -d ' ' -f 1)
sed -i "s/${old_install_hash}/${new_install_hash}/" packaging/PKGBUILD

printf 'Renamed repository identity. Review the diff, run just check, and rebuild generated docs.\n'
