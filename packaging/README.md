# Packaging

`PKGBUILD` builds the source archive; `PKGBUILD-bin` installs the architecture-
specific release bundle. Both install the same `/usr` tree and depend on
Carlito and Liberation fonts so Calibri and Arial workbooks retain compatible
metrics without LibreOffice. LibreOffice is neither a runtime nor packaging
dependency.

The binary recipe consumes release-tree `.tar.zst` bundles; those are inputs
that `makepkg` wraps as a real Arch package, not files intended for direct
`pacman -U`. The source recipe disables makepkg's C/C++ LTO injection because
GCC `-flto=auto` makes `ring`'s bundled native objects unavailable to rust-lld;
the workspace's Rust thin-LTO release profile remains enabled. The checked-in
recipes use an explicit checksum placeholder because `0.0.0` is not a public
release. The release workflow substitutes the tag archive or bundle SHA-256,
runs `makepkg --printsrcinfo`, builds both packages in clean Arch containers,
and publishes the resulting recipes with the release assets.
For a local source smoke build, `scripts/arch-package-smoke.sh` creates a source
archive and supplies its `file://` URL and checksum through the documented
`OMACELL_SOURCE_URL` and `OMACELL_SOURCE_SHA256` inputs.

Installed paths:

- `/usr/bin/omacell`
- `/usr/share/omacell/default/` (configuration, keymaps, theme template,
  prompts, and agent skill)
- `/usr/share/omacell/i18n/` (Fluent localization resources)
- `/usr/share/applications/omacell.desktop`
- `/usr/share/mime/packages/omacell.xml`
- `/usr/share/icons/hicolor/`
- shell completions, `omacell(1)`, and the HTML manual

The install hook updates only system caches and prints the optional
`omacell setup omarchy` next step. It never writes into a user's home or
`/usr/share/omarchy`.
