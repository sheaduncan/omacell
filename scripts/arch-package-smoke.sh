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
  su build -c "cd /home/build/package && PKGBUILD_SOURCE_URL=file:///home/build/omacell-0.0.0.tar.gz PKGBUILD_SOURCE_SHA256=$checksum makepkg --noconfirm"
  package_file=$(find /home/build/package -maxdepth 1 -name "omacell-0.0.0-1-*.pkg.tar.zst" -print -quit)
  test -n "$package_file"
  bsdtar -tf "$package_file" | grep -qx "usr/share/doc/omacell/manual/index.html"
  printf "%s\n" "smoke: manual in package archive"
  pacman -U --noconfirm "$package_file"
  printf "%s\n" "smoke: installed package"
  omacell --version
  printf "%s\n" "smoke: version command"
  omacell fn list --json >/dev/null
  printf "%s\n" "smoke: function catalog"
  desktop-file-validate /usr/share/applications/omacell.desktop
  printf "%s\n" "smoke: desktop entry"
  test -f /usr/share/omacell/default/agents/skills/omacell/SKILL.md
  printf "%s\n" "smoke: packaged skill"
  test -f /usr/share/omacell/i18n/en-US/omacell.ftl
  printf "%s\n" "smoke: locale catalog"
'
