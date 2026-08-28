set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# fmt, clippy, tests, docs — the gate every PR must pass
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    cargo doc --workspace --no-deps

# all tests including integration
test:
    cargo test --workspace

# unit tests only (libs and bins, no integration tests)
test-fast:
    cargo test --workspace --lib --bins

# criterion benches (packages add these later)
bench:
    cargo bench --workspace

# run a cargo-fuzz target (requires nightly + cargo-fuzz)
fuzz target:
    cargo +nightly fuzz run {{target}}

lint:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check

fmt:
    cargo fmt

# verify corpus directories exist and README stubs name their WP
corpus-verify:
    cargo test --workspace --test repo_lint
    test -d tests/corpus/formulas
    test -d tests/corpus/eval
    test -d tests/corpus/functions
    test -d tests/corpus/numfmt
    test -d tests/corpus/csv
    test -d tests/corpus/xlsx
    test -d tests/corpus/omc
    test -d tests/corpus/themes
    test -d tests/corpus/evals

# reproduce the committed Gate G1 sample, baselines, and LibreOffice check
g1-verify:
    python3 scripts/g1-sample.py --check tests/fixtures/g1/spotcheck-20260828.tsv
    python3 scripts/check-g1-baselines.py
    python3 scripts/lo-crosscheck.py tests/fixtures/g1/spotcheck-20260828.tsv

# Validate the durable WP-08 CSV performance and memory baseline record.
wp08-baseline-verify:
    python3 scripts/check-wp08-baselines.py

# record criterion baselines (packages that touch §12.1 call this)
perf-baseline:
    cargo bench --workspace -- --save-baseline default
