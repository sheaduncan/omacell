# Pivot corpus (WP-24)

JSON cases consumed by `crates/core/tests/pivot.rs`.

Each case supplies a headered source table, a pivot definition, and the
expected output cells as offsets from the destination origin. Numbers compare
within `1e-9`. Refresh cases edit the source and expect a second snapshot.
