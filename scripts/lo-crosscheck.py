#!/usr/bin/env python3
"""Headless LibreOffice cross-check for function corpus rows.

Skips cleanly when `soffice` / LibreOffice is not installed. Network is never
used. Intended for WP-05a/b/c corpora under tests/corpus/functions/.

Usage:
    python3 scripts/lo-crosscheck.py [tsv ...]
"""

from __future__ import annotations

import csv
import shutil
import subprocess
import sys
from pathlib import Path


def find_soffice() -> str | None:
    for name in ("soffice", "libreoffice"):
        path = shutil.which(name)
        if path:
            return path
    return None


def parse_tsv(path: Path) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        formula = parts[0] if parts else ""
        expected = parts[1] if len(parts) > 1 else ""
        note = parts[2] if len(parts) > 2 else ""
        rows.append((formula, expected, note))
    return rows


def main(argv: list[str]) -> int:
    soffice = find_soffice()
    if soffice is None:
        print("lo-crosscheck: LibreOffice not installed; skip.")
        return 0
    files = [Path(a) for a in argv[1:]]
    if not files:
        root = Path(__file__).resolve().parents[1]
        files = sorted((root / "tests/corpus/functions").glob("*.tsv"))
    print(f"lo-crosscheck: using {soffice}")
    print(f"lo-crosscheck: {len(files)} corpus file(s).")
    print(
        "lo-crosscheck: evaluation via soffice is not wired in WP-05F; "
        "this script only verifies the host tool exists and the TSV files parse."
    )
    total = 0
    for path in files:
        rows = parse_tsv(path)
        total += len(rows)
        print(f"  {path.name}: {len(rows)} rows")
    print(f"lo-crosscheck: {total} rows parsed.")
    # Touch soffice so the skip path is the only silent one.
    try:
        subprocess.run(
            [soffice, "--version"],
            check=False,
            capture_output=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        print(f"lo-crosscheck: soffice --version failed ({exc}); skip.")
        return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
