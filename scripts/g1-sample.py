#!/usr/bin/env python3
"""Generate a fresh sample or verify the frozen Gate G1 evidence."""

from __future__ import annotations

import argparse
import hashlib
import random
import re
import sys
from dataclasses import dataclass
from pathlib import Path


DEFAULT_SEED = 20260828
FUNCTION_ROWS = 40
EVAL_ROWS = 10
FROZEN_FIXTURE_SHA256 = "196921f91f75812237ed969cbf87002f8c7507a99074ccf66e2187383c38b5f7"
SOURCE_RE = re.compile(r"^\[(function|eval):([A-Za-z0-9_.-]+\.tsv)\] .+")


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


def verify_frozen_sample(root: Path, path: Path, seed: int) -> None:
    """Validate the immutable sample without resampling the evolving corpus."""
    if seed != DEFAULT_SEED:
        raise ValueError("the frozen G1 fixture only supports seed 20260828")
    raw = path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != FROZEN_FIXTURE_SHA256:
        raise ValueError(
            f"frozen G1 fixture digest changed: expected {FROZEN_FIXTURE_SHA256}, "
            f"got {digest}"
        )

    lines = raw.decode("utf-8").splitlines()
    expected_header = f"# G1 spot-check sample seed={seed}"
    if not lines or lines[0] != expected_header:
        raise ValueError(f"frozen G1 fixture must start with {expected_header!r}")

    counts = {"function": 0, "eval": 0}
    seen: set[tuple[str, str, str]] = set()
    for line_number, line in enumerate(lines[1:], start=2):
        parts = line.split("\t")
        if len(parts) != 3 or not parts[0].startswith("=") or not parts[2]:
            raise ValueError(f"{path}:{line_number}: malformed frozen sample row")
        match = SOURCE_RE.fullmatch(parts[2])
        if match is None:
            raise ValueError(f"{path}:{line_number}: malformed source annotation")
        kind, source = match.groups()
        counts[kind] += 1
        identity = (kind, source, parts[0])
        if identity in seen:
            raise ValueError(f"{path}:{line_number}: duplicate sampled formula")
        seen.add(identity)

        suite = "functions" if kind == "function" else "eval"
        source_path = root / "tests/corpus" / suite / source
        if not source_path.is_file():
            raise ValueError(f"{path}:{line_number}: missing source corpus {source_path}")
        live_formulas = {
            source_line.split("\t", 1)[0]
            for source_line in source_path.read_text(encoding="utf-8").splitlines()
            if source_line.startswith("=")
        }
        if parts[0] not in live_formulas:
            raise ValueError(
                f"{path}:{line_number}: sampled formula is absent from {source_path}"
            )

    if counts != {"function": FUNCTION_ROWS, "eval": EVAL_ROWS}:
        raise ValueError(
            "frozen G1 fixture must contain exactly "
            f"{FUNCTION_ROWS} function and {EVAL_ROWS} eval rows; got {counts}"
        )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument(
        "--check",
        type=Path,
        help="validate the immutable committed sample and its corpus provenance",
    )
    args = parser.parse_args(argv[1:])
    root = Path(__file__).resolve().parents[1]
    if args.check is None:
        sys.stdout.write(render(sample_rows(root, args.seed), args.seed))
        return 0
    try:
        verify_frozen_sample(root, args.check, args.seed)
    except (OSError, UnicodeError, ValueError) as exc:
        print(f"G1 sample invalid: {exc}", file=sys.stderr)
        return 1
    print(f"G1 frozen sample is valid: {args.check}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
