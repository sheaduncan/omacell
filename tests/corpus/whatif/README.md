# Goal Seek corpus (WP-24)

`goalseek.tsv` is consumed by `crates/core/tests/pivot.rs`.

Columns: case name, target A1, goal, input A1, starting input value, formula
in the target cell, whether the solver must converge, and the expected input
when it does. Non-convergence leaves a finite last trial and reports
`converged=false`.
