# Pivot corpus (WP-24 / WP-24a)

JSON cases consumed by `crates/core/tests/pivot.rs`.

Each case supplies a headered source table, a pivot definition, and the
expected output cells as offsets from the destination origin. Numbers compare
within `1e-9`. Refresh cases edit the source and expect a second snapshot.

Optional `layout` is `compact` (default), `outline`, or `tabular`. Compact
nested row fields indent two spaces per depth and emit a group header row when
an outer key changes. Outline uses one column per row field and blanks repeated
outer labels. Tabular repeats outer labels.

Optional `calc_fields` are `[{ "name", "formula" }]` evaluated over source
columns (`'Amount'*0.1`).
