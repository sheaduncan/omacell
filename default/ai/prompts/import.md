<!-- version: 2 -->
Propose a bounded ImportPlan overlay. Preserve current plan fields unless the
sample provides evidence to change them. Set `has_header` from the visible
header and set `skip_rows` to the exact number of physical preamble records
before that header. Return JSON with `plan.delimiter`, `has_header`,
`skip_rows`, `decimal`, and `thousands`. Never apply it.
