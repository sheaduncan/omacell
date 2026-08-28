#!/usr/bin/env python3
"""Generate or verify the deterministic Gate G1 LibreOffice sample."""

from __future__ import annotations

import argparse
import random
import sys
from dataclasses import dataclass
from pathlib import Path


DEFAULT_SEED = 20260828
FUNCTION_ROWS = 40
EVAL_ROWS = 10


@dataclass(frozen=True)
class CorpusRow:
    formula: str
    expected: str
    note: str
    source: str
    kind: str


def read_pool(directory: Path, kind: str) -> list[CorpusRow]:
    rows: list[CorpusRow] = []
    for path in sorted(directory.glob("*.tsv")):
        for line_number, line in enumerate(
            path.read_text(encoding="utf-8").splitlines(), start=1
        ):
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            parts = stripped.split("\t")
            if len(parts) < 3 or not parts[0].startswith("=") or not parts[2]:
                raise ValueError(f"{path}:{line_number}: malformed corpus row")
            rows.append(
                CorpusRow(
                    formula=parts[0],
                    expected=parts[1],
                    note=parts[2],
                    source=path.name,
                    kind=kind,
                )
            )
    return rows


def sample_rows(root: Path, seed: int) -> list[CorpusRow]:
    functions = read_pool(root / "tests/corpus/functions", "function")
    eval_rows = read_pool(root / "tests/corpus/eval", "eval")
    if len(functions) < FUNCTION_ROWS or len(eval_rows) < EVAL_ROWS:
        raise ValueError("corpus is smaller than the G1 sample")
    rng = random.Random(seed)
    selected = rng.sample(functions, FUNCTION_ROWS) + rng.sample(eval_rows, EVAL_ROWS)
    rng.shuffle(selected)
    return selected


def render(rows: list[CorpusRow], seed: int) -> str:
    lines = [f"# G1 spot-check sample seed={seed}"]
    lines.extend(
        f"{row.formula}\t{row.expected}\t[{row.kind}:{row.source}] {row.note}"
        for row in rows
    )
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument(
        "--check",
        type=Path,
        help="fail unless this committed fixture matches the generated sample",
    )
    args = parser.parse_args(argv[1:])
    root = Path(__file__).resolve().parents[1]
    generated = render(sample_rows(root, args.seed), args.seed)
    if args.check is None:
        sys.stdout.write(generated)
        return 0
    committed = args.check.read_text(encoding="utf-8")
    if committed != generated:
        print(f"G1 sample drift: regenerate {args.check}", file=sys.stderr)
        return 1
    print(f"G1 sample matches {args.check}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
