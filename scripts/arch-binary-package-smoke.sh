#!/bin/bash
# Build a release tree, wrap it with PKGBUILD-bin, install it, and smoke-test it.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
image=${PKGBUILD_ARCH_IMAGE:-archlinux:base-devel}
docker run --rm --volume "$root:/src:ro" "$image" bash -euo pipefail -c '
  pacman -Syu --noconfirm --needed cargo rust mdbook python zstd \
    fontconfig libxkbcommon mesa shared-mime-info ttf-carlito ttf-liberation wayland \
    gtk-update-icon-cache
  useradd --create-home build
  install -d -o build -g build /home/build/source /home/build/package /home/build/release-tree/usr
  tar -C /src --exclude=.git --exclude=target --exclude=.direnv -cf - . |
    tar -C /home/build/source -xf -
  chown -R build:build /home/build
  su build -c "
    cd /home/build/source
    export CARGO_TARGET_DIR=/home/build/source/target
    cargo fetch --locked
    cargo build --frozen --release -p omacell-cli -p omacell-xls-worker
    cargo test --frozen -p omacell-cli --test dist generate_completions_and_man
    cargo run --frozen -p omacell-cli --example generate-docs -- docs/book/cli-reference.md
    python scripts/generate-docs.py --write
    mdbook build
  "
  release=/home/build/release-tree/usr
  install -Dm755 /home/build/source/target/release/omacell "$release/bin/omacell"
  install -Dm755 /home/build/source/target/release/omacell-xls-worker \
    "$release/lib/omacell/omacell-xls-worker"
  install -Dm644 /home/build/source/LICENSE "$release/share/licenses/omacell/LICENSE"
  install -Dm644 /home/build/source/packaging/omacell.desktop \
    "$release/share/applications/omacell.desktop"
  install -Dm644 /home/build/source/packaging/omacell.xml \
    "$release/share/mime/packages/omacell.xml"
  install -d "$release/share/omacell" "$release/share/icons" \
    "$release/share/doc/omacell/manual"
  cp -a /home/build/source/default "$release/share/omacell/default"
  cp -a /home/build/source/i18n "$release/share/omacell/i18n"
  cp -a /home/build/source/packaging/icons/hicolor "$release/share/icons/"
  install -Dm644 /home/build/source/target/dist/omacell.1 \
    "$release/share/man/man1/omacell.1"
  install -Dm644 /home/build/source/target/dist/omacell.bash \
    "$release/share/bash-completion/completions/omacell"
  install -Dm644 /home/build/source/target/dist/omacell.fish \
    "$release/share/fish/vendor_completions.d/omacell.fish"
  install -Dm644 /home/build/source/target/dist/_omacell \
    "$release/share/zsh/site-functions/_omacell"
  cp -a /home/build/source/book/. "$release/share/doc/omacell/manual/"
  tar --zstd -cf /home/build/omacell-0.0.0-x86_64.tar.zst -C /home/build/release-tree .
  install -o build -g build /src/packaging/PKGBUILD-bin /src/packaging/omacell.install \
    /home/build/package/
  chown build:build /home/build/omacell-0.0.0-x86_64.tar.zst
  checksum=$(sha256sum /home/build/omacell-0.0.0-x86_64.tar.zst | cut -d " " -f 1)
  su build -c "
    cd /home/build/package
    PKGBUILD_BIN_X86_64_URL=file:///home/build/omacell-0.0.0-x86_64.tar.zst \\
      PKGBUILD_BIN_X86_64_SHA256=$checksum makepkg --noconfirm --cleanbuild -p PKGBUILD-bin
  "
  package_file=$(find /home/build/package -maxdepth 1 -name "omacell-bin-0.0.0-1-*.pkg.tar.zst" -print -quit)
  test -n "$package_file"
  bsdtar -tf "$package_file" "usr/share/licenses/omacell/LICENSE" >/dev/null
  bsdtar -tf "$package_file" "usr/share/doc/omacell/manual/index.html" >/dev/null
  pacman -U --noconfirm "$package_file"
  omacell --version
  test -x /usr/lib/omacell/omacell-xls-worker
  test -f /usr/share/omacell/default/agents/skills/omacell/SKILL.md
  printf "%s\n" "smoke: binary package installed release tree"
'
