#!/bin/bash
# Build and install the source PKGBUILD in a clean Arch container.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
image=${OMACELL_ARCH_IMAGE:-archlinux:base-devel}
docker run --rm --volume "$root:/src:ro" "$image" bash -euo pipefail -c '
  pacman -Syu --noconfirm --needed cargo rust mdbook desktop-file-utils libxml2 python \
    fontconfig libxkbcommon mesa shared-mime-info ttf-carlito ttf-liberation wayland \
    gtk-update-icon-cache
  useradd --create-home build
  install -d -o build -g build /home/build/omacell-0.0.0 /home/build/package
  tar -C /src --exclude=.git --exclude=target --exclude=.direnv -cf - . |
    tar -C /home/build/omacell-0.0.0 -xf -
  tar -C /home/build -czf /home/build/omacell-0.0.0.tar.gz omacell-0.0.0
  install -o build -g build /src/packaging/PKGBUILD /src/packaging/omacell.install /home/build/package/
  chown -R build:build /home/build
  checksum=$(sha256sum /home/build/omacell-0.0.0.tar.gz | cut -d " " -f 1)
  su build -c "cd /home/build/package && OMACELL_SOURCE_URL=file:///home/build/omacell-0.0.0.tar.gz OMACELL_SOURCE_SHA256=$checksum makepkg --noconfirm"
  pacman -U --noconfirm /home/build/package/omacell-0.0.0-1-*.pkg.tar.zst
  omacell --version
  omacell fn list --json >/dev/null
  desktop-file-validate /usr/share/applications/omacell.desktop
  test -f /usr/share/omacell/default/agents/skills/omacell/SKILL.md
  test -f /usr/share/omacell/i18n/en-US/omacell.ftl
  test -f /usr/share/doc/omacell/manual/index.html
'
