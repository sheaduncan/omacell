#!/usr/bin/env python3
"""Headless LibreOffice cross-check for function corpus rows.

Skips cleanly when `soffice` / LibreOffice is not installed. Network is never
used. Intended for WP-05a/b/c corpora under tests/corpus/functions/.

Usage:
    python3 scripts/lo-crosscheck.py [tsv ...]
"""

from __future__ import annotations

import csv
import html
import shutil
import subprocess
import sys
import tempfile
import zipfile
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
        if len(parts) < 3 or len(parts) > 5 or not parts[0].startswith("=") or not parts[2].strip():
            raise ValueError(f"{path}: malformed corpus row: {line!r}")
        formula, expected, note = parts[0], parts[1], parts[2]
        rows.append((formula, expected, note))
    return rows


# Post-2007 names LibreOffice's XLSX importer only maps with `_xlfn.`.
_MODERN_CALLEES = {
    "ACOT",
    "ACOTH",
    "AGGREGATE",
    "COMBINA",
    "COT",
    "COTH",
    "CSC",
    "CSCH",
    "FORECAST.LINEAR",
    "IFNA",
    "IFS",
    "ISFORMULA",
    "ISOMITTED",
    "MAXIFS",
    "MINIFS",
    "PERMUTATIONA",
    "SEC",
    "SECH",
    "SWITCH",
    "XOR",
    "ARRAYTOTEXT",
    "CONCAT",
    "DAYS",
    "ISOWEEKNUM",
    "NUMBERVALUE",
    "REGEXEXTRACT",
    "REGEXREPLACE",
    "REGEXTEST",
    "SEQUENCE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TEXTBEFORE",
    "TEXTAFTER",
    "UNICHAR",
    "UNICODE",
    "VALUETOTEXT",
}

_LO_ERR = {
    "Err:501": "#DIV/0!",
    "Err:502": "#VALUE!",
    "Err:503": "#NUM!",
    "Err:504": "#VALUE!",
    "Err:519": "#N/A",
    "Err:521": "#NULL!",
    "Err:525": "#NAME?",
    "Err:532": "#N/A",
    "Err:538": "#N/A",
    "Err:539": "#N/A",
}


def _xlfn_prefix(expression: str) -> str:
    # OOXML stores post-2007 function names with the compatibility prefix.
    head = expression.split("(", 1)[0].upper()
    if "." in head or head in _MODERN_CALLEES:
        return f"_xlfn.{expression}"
    return expression


def _numeric_close(left: str, right: str) -> bool:
    try:
        a = float(left)
        b = float(right)
    except ValueError:
        return False
    if a == b:
        return True
    scale = max(abs(a), abs(b), 1e-9)
    return abs(a - b) / scale < 1e-8


def values_match(expected: str, actual: str) -> bool:
    if actual == expected:
        return True
    mapped = _LO_ERR.get(actual, actual)
    if mapped == expected:
        return True
    # LibreOffice CSV uses Err:502 for several Excel error kinds.
    if actual == "Err:502" and expected in {"#VALUE!", "#NUM!", "#DIV/0!"}:
        return True
    if actual == "Err:504" and expected in {"#N/A", "#VALUE!", "#NUM!"}:
        return True
    if expected in {"TRUE", "FALSE"} and actual.lower() == expected.lower():
        return True
    return _numeric_close(expected, mapped)


def write_workbook(path: Path, rows: list[tuple[str, str, str]]) -> None:
    sheet_rows = []
    for index, (formula, _expected, _note) in enumerate(rows, start=1):
        expression = formula.removeprefix("=")
        expression = _xlfn_prefix(expression)
        escaped = html.escape(expression, quote=False)
        sheet_rows.append(
            f'<row r="{index}"><c r="A{index}"><f>{escaped}</f><v/></c></row>'
        )
    sheet_xml = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>{}</sheetData>
</worksheet>
""".format("".join(sheet_rows))
    parts = {
        "[Content_Types].xml": """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>
""",
        "_rels/.rels": """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>
""",
        "xl/workbook.xml": """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Corpus" sheetId="1" r:id="rId1"/></sheets>
  <calcPr calcMode="auto" fullCalcOnLoad="1" forceFullCalc="1"/>
</workbook>
""",
        "xl/_rels/workbook.xml.rels": """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>
""",
        "xl/worksheets/sheet1.xml": sheet_xml,
    }
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            archive.writestr(name, content)


def evaluate(
    soffice: str, rows: list[tuple[str, str, str]], directory: Path
) -> list[str]:
    workbook = directory / "omacell-function-corpus.xlsx"
    output = directory / "omacell-function-corpus.csv"
    profile = directory / "lo-profile"
    profile.mkdir()
    write_workbook(workbook, rows)
    completed = subprocess.run(
        [
            soffice,
            "--headless",
            f"-env:UserInstallation={profile.resolve().as_uri()}",
            "--convert-to",
            "csv",
            "--outdir",
            str(directory),
            str(workbook),
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=60,
    )
    if completed.returncode != 0 or not output.is_file():
        detail = " | ".join(
            part
            for part in [completed.stdout.strip(), completed.stderr.strip()]
            if part
        )
        raise RuntimeError(
            f"LibreOffice conversion failed (exit {completed.returncode}, "
            f"output={output.is_file()}): {detail}"
        )
    with output.open(newline="", encoding="utf-8-sig") as handle:
        csv_rows = list(csv.reader(handle))
    if len(csv_rows) < len(rows):
        raise RuntimeError(
            f"LibreOffice returned {len(csv_rows)} rows for {len(rows)} formulas"
        )
    return [csv_rows[index][0].strip() if csv_rows[index] else "" for index in range(len(rows))]


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
    try:
        indexed_rows: list[tuple[Path, str, str, str]] = []
        for path in files:
            rows = parse_tsv(path)
            indexed_rows.extend((path, formula, expected, note) for formula, expected, note in rows)
        with tempfile.TemporaryDirectory(prefix="omacell-lo-") as temp:
            got = evaluate(
                soffice,
                [(formula, expected, note) for _, formula, expected, note in indexed_rows],
                Path(temp),
            )
    except (OSError, RuntimeError, subprocess.TimeoutExpired, ValueError) as exc:
        print(f"lo-crosscheck: {exc}", file=sys.stderr)
        return 1

    failures = 0
    known = 0
    for (path, formula, expected, note), actual in zip(indexed_rows, got, strict=True):
        if values_match(expected, actual):
            continue
        if "known difference" in note.lower():
            known += 1
            print(f"KNOWN {path.name}: {formula}: Omacell={expected!r}, LO={actual!r}")
            continue
        failures += 1
        print(
            f"FAIL {path.name}: {formula}: expected {expected!r}, LO={actual!r} ({note})",
            file=sys.stderr,
        )
    print(
        f"lo-crosscheck: {len(indexed_rows)} evaluated, "
        f"{known} known difference(s), {failures} unexplained difference(s)."
    )
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
