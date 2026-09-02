# Icons

`hicolor/scalable/apps/omacell.svg` is the canonical application artwork.
The PNG application icons are deterministic renders of that file. The
symbolic and MIME icons are maintained as SVG because they intentionally use
different single-color and document silhouettes.

Regenerate the PNGs with:

```sh
for size in 16 24 32 48 64 128 256; do
  rsvg-convert -w "$size" -h "$size" \
    hicolor/scalable/apps/omacell.svg \
    -o "hicolor/${size}x${size}/apps/omacell.png"
done
```
