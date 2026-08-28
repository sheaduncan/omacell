#!/usr/bin/env python3
"""Generate synthetic .xlsx corpus files (WP-09).

Uses the stdlib zipfile so generation does not require openpyxl. When
openpyxl is installed, `gen_openpyxl.py` can rebuild richer files.

Run from the repo root: python3 scripts/corpus-gen/xlsx/gen.py
"""
from __future__ import annotations

import json
import zipfile
from pathlib import Path

NS = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
PKG = "http://schemas.openxmlformats.org/package/2006/relationships"
OD = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
CT_WB = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
CT_WS = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"
CT_SST = "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"
CT_STY = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"
CT_TBL = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "tests" / "corpus" / "xlsx"


def xml(s: str) -> bytes:
    return s.encode("utf-8")


def content_types(*overrides: tuple[str, str]) -> bytes:
    defs = (
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Default Extension="json" ContentType="application/json"/>'
    )
    ovs = "".join(
        f'<Override PartName="{p}" ContentType="{ct}"/>' for p, ct in overrides
    )
    return xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        f"{defs}{ovs}</Types>"
    )


def rels(tag: str, items: list[tuple[str, str, str]]) -> bytes:
    body = "".join(
        f'<Relationship Id="{i}" Type="{t}" Target="{tgt}"/>' for i, t, tgt in items
    )
    return xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<Relationships xmlns="{PKG}">{body}</Relationships>'
    )


def styles_basic() -> bytes:
    return xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<styleSheet xmlns="{NS}">'
        f'<numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts>'
        f'<fonts count="2">'
        f'<font><sz val="11"/><color theme="1"/><name val="Calibri"/></font>'
        f'<font><b/><sz val="11"/><name val="Calibri"/></font>'
        f"</fonts>"
        f'<fills count="3">'
        f'<fill><patternFill patternType="none"/></fill>'
        f'<fill><patternFill patternType="gray125"/></fill>'
        f'<fill><patternFill patternType="solid"><fgColor rgb="FF00AA00"/></patternFill></fill>'
        f"</fills>"
        f'<borders count="1"><border><left/><right/><top/><bottom/></border></borders>'
        f'<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
        f'<cellXfs count="3">'
        f'<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>'
        f'<xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFont="1" applyFill="1"/>'
        f'<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
        f"</cellXfs>"
        f"</styleSheet>"
    )


def sst(*items: str) -> bytes:
    sis = "".join(f'<si><t xml:space="preserve">{t}</t></si>' for t in items)
    return xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<sst xmlns="{NS}" count="{len(items)}" uniqueCount="{len(items)}">{sis}</sst>'
    )


def write_xlsx(path: Path, parts: dict[str, bytes]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as z:
        for name, data in parts.items():
            z.writestr(name, data)


def pack_simple(sheet_xml: bytes, extra: dict[str, bytes] | None = None, extra_ct=(), extra_wb_rels=()) -> dict[str, bytes]:
    parts = {
        "[Content_Types].xml": content_types(
            ("/xl/workbook.xml", CT_WB),
            ("/xl/worksheets/sheet1.xml", CT_WS),
            ("/xl/sharedStrings.xml", CT_SST),
            ("/xl/styles.xml", CT_STY),
            *extra_ct,
        ),
        "_rels/.rels": rels(
            "pkg",
            [("rId1", f"{OD}/officeDocument", "xl/workbook.xml")],
        ),
        "xl/_rels/workbook.xml.rels": rels(
            "wb",
            [
                ("rId1", f"{OD}/worksheet", "worksheets/sheet1.xml"),
                ("rId2", f"{OD}/sharedStrings", "sharedStrings.xml"),
                ("rId3", f"{OD}/styles", "styles.xml"),
                *extra_wb_rels,
            ],
        ),
        "xl/workbook.xml": xml(
            f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            f'<workbook xmlns="{NS}" xmlns:r="{OD}">'
            f'<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>'
            f"</workbook>"
        ),
        "xl/worksheets/sheet1.xml": sheet_xml,
        "xl/sharedStrings.xml": sst("hello", "world"),
        "xl/styles.xml": styles_basic(),
    }
    if extra:
        parts.update(extra)
    return parts


def gen_l1_values() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}">'
        f"<sheetData>"
        f'<row r="1">'
        f'<c r="A1" t="n"><v>1.5</v></c>'
        f'<c r="B1" t="s"><v>0</v></c>'
        f'<c r="C1" t="b"><v>1</v></c>'
        f'<c r="D1" t="e"><v>#DIV/0!</v></c>'
        f'<c r="E1" t="inlineStr"><is><t>inline</t></is></c>'
        f"</row>"
        f'<row r="2">'
        f'<c r="A2" s="2"><v>44927</v></c>'
        f"</row>"
        f"</sheetData></worksheet>"
    )
    write_xlsx(OUT / "l1_values.xlsx", pack_simple(sheet))
    (OUT / "l1_values.json").write_text(
        json.dumps(
            {
                "sheets": ["Sheet1"],
                "cells": {
                    "A1": {"kind": "number", "n": 1.5},
                    "B1": {"kind": "text", "t": "hello"},
                    "C1": {"kind": "bool", "b": True},
                    "D1": {"kind": "error", "e": "#DIV/0!"},
                    "E1": {"kind": "text", "t": "inline"},
                },
            },
            indent=2,
        )
        + "\n"
    )


def gen_l1_formulas() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}">'
        f"<sheetData>"
        f'<row r="1">'
        f'<c r="A1"><v>1</v></c>'
        f'<c r="B1"><v>2</v></c>'
        f'<c r="C1"><f>A1+B1</f><v>3</v></c>'
        f"</row>"
        f'<row r="2">'
        f'<c r="A2"><f t="shared" ref="A2:A3" si="0">A1+10</f><v>11</v></c>'
        f"</row>"
        f'<row r="3">'
        f'<c r="A3"><f t="shared" si="0"/><v>12</v></c>'
        f"</row>"
        f"</sheetData></worksheet>"
    )
    write_xlsx(OUT / "l1_formulas.xlsx", pack_simple(sheet))
    (OUT / "l1_formulas.json").write_text(
        json.dumps(
            {
                "sheets": ["Sheet1"],
                "formulas": {"C1": "=A1+B1", "A2": "=A1+10", "A3": "=A2+10"},
            },
            indent=2,
        )
        + "\n"
    )


def gen_l2_merges_freeze() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}">'
        f'<sheetViews><sheetView workbookViewId="0" zoomScale="150">'
        f'<pane xSplit="1" ySplit="1" topLeftCell="B2" activePane="bottomRight" state="frozen"/>'
        f"</sheetView></sheetViews>"
        f"<sheetData>"
        f'<row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row>'
        f"</sheetData>"
        f'<mergeCells count="1"><mergeCell ref="A1:B1"/></mergeCells>'
        f"</worksheet>"
    )
    write_xlsx(OUT / "l2_merges_freeze.xlsx", pack_simple(sheet))
    (OUT / "l2_merges_freeze.json").write_text(
        json.dumps(
            {"merges": ["A1:B1"], "freeze": {"rows": 1, "cols": 1}, "zoom": 1.5},
            indent=2,
        )
        + "\n"
    )


def gen_l2_names() -> None:
    parts = pack_simple(
        xml(
            f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            f'<worksheet xmlns="{NS}"><sheetData>'
            f'<row r="1"><c r="A1"><v>10</v></c></row>'
            f"</sheetData></worksheet>"
        )
    )
    parts["xl/workbook.xml"] = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<workbook xmlns="{NS}" xmlns:r="{OD}">'
        f'<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>'
        f"<definedNames>"
        f'<definedName name="Rate">Sheet1!$A$1</definedName>'
        f"</definedNames>"
        f"</workbook>"
    )
    write_xlsx(OUT / "l2_names.xlsx", parts)
    (OUT / "l2_names.json").write_text(
        json.dumps({"names": [{"name": "Rate", "a1": "A1"}]}, indent=2) + "\n"
    )


def gen_l2_hyperlinks() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}" xmlns:r="{OD}">'
        f"<sheetData>"
        f'<row r="1"><c r="A1" t="s"><v>0</v></c></row>'
        f"</sheetData>"
        f'<hyperlinks><hyperlink ref="A1" r:id="rId1" display="example"/></hyperlinks>'
        f"</worksheet>"
    )
    parts = pack_simple(sheet)
    parts["xl/worksheets/_rels/sheet1.xml.rels"] = rels(
        "ws",
        [("rId1", f"{OD}/hyperlink", "https://example.com")],
    )
    # TargetMode external
    parts["xl/worksheets/_rels/sheet1.xml.rels"] = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<Relationships xmlns="{PKG}">'
        f'<Relationship Id="rId1" Type="{OD}/hyperlink" Target="https://example.com" TargetMode="External"/>'
        f"</Relationships>"
    )
    write_xlsx(OUT / "l2_hyperlinks.xlsx", parts)
    (OUT / "l2_hyperlinks.json").write_text(
        json.dumps({"hyperlinks": {"A1": "https://example.com"}}, indent=2) + "\n"
    )


def gen_l2_table() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}" xmlns:r="{OD}">'
        f"<sheetData>"
        f'<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>'
        f'<row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>2</v></c></row>'
        f"</sheetData>"
        f'<tableParts count="1"><tablePart r:id="rId1"/></tableParts>'
        f"</worksheet>"
    )
    parts = pack_simple(
        sheet,
        extra={
            "xl/tables/table1.xml": xml(
                f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                f'<table xmlns="{NS}" id="1" name="Sales" displayName="Sales" ref="A1:B2" headerRowCount="1">'
                f"<tableColumns count=\"2\">"
                f'<tableColumn id="1" name="hello"/>'
                f'<tableColumn id="2" name="world"/>'
                f"</tableColumns>"
                f"</table>"
            ),
            "xl/worksheets/_rels/sheet1.xml.rels": rels(
                "ws", [("rId1", f"{OD}/table", "../tables/table1.xml")]
            ),
        },
        extra_ct=(("/xl/tables/table1.xml", CT_TBL),),
    )
    write_xlsx(OUT / "l2_table.xlsx", parts)
    (OUT / "l2_table.json").write_text(
        json.dumps({"tables": [{"name": "Sales", "ref": "A1:B2"}]}, indent=2) + "\n"
    )


def gen_l2_comments() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}" xmlns:r="{OD}">'
        f"<sheetData>"
        f'<row r="1"><c r="A1"><v>1</v></c></row>'
        f"</sheetData></worksheet>"
    )
    comments = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<comments xmlns="{NS}">'
        f"<authors><author>Ada</author></authors>"
        f"<commentList>"
        f'<comment ref="A1" authorId="0"><text><t>check this</t></text></comment>'
        f"</commentList></comments>"
    )
    parts = pack_simple(
        sheet,
        extra={
            "xl/comments1.xml": comments,
            "xl/worksheets/_rels/sheet1.xml.rels": rels(
                "ws", [("rId1", f"{OD}/comments", "../comments1.xml")]
            ),
        },
        extra_ct=(
            (
                "/xl/comments1.xml",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml",
            ),
        ),
    )
    write_xlsx(OUT / "l2_comments.xlsx", parts)
    (OUT / "l2_comments.json").write_text(
        json.dumps({"notes": {"A1": {"author": "Ada", "text": "check this"}}}, indent=2)
        + "\n"
    )


def gen_omacell_part() -> None:
    parts = pack_simple(
        xml(
            f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            f'<worksheet xmlns="{NS}"><sheetData/></worksheet>'
        ),
        extra={"xl/omacell/meta.json": b'{"hello":true}'},
        extra_ct=(("/xl/omacell/meta.json", "application/json"),),
    )
    write_xlsx(OUT / "omacell_part.xlsx", parts)
    (OUT / "omacell_part.json").write_text(
        json.dumps({"custom_parts": ["xl/omacell/meta.json"]}, indent=2) + "\n"
    )


def gen_l2_print() -> None:
    sheet = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}">'
        f"<sheetData>"
        f'<row r="1"><c r="A1"><v>1</v></c></row>'
        f"</sheetData>"
        f'<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>'
        f'<pageSetup orientation="landscape" paperSize="9"/>'
        f"</worksheet>"
    )
    write_xlsx(OUT / "l2_print.xlsx", pack_simple(sheet))
    (OUT / "l2_print.json").write_text(
        json.dumps({"print": True}, indent=2) + "\n"
    )


def gen_hidden_sheet() -> None:
    parts = pack_simple(
        xml(
            f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            f'<worksheet xmlns="{NS}"><sheetData>'
            f'<row r="1"><c r="A1"><v>1</v></c></row>'
            f"</sheetData></worksheet>"
        )
    )
    parts["xl/workbook.xml"] = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<workbook xmlns="{NS}" xmlns:r="{OD}">'
        f"<sheets>"
        f'<sheet name="Visible" sheetId="1" r:id="rId1"/>'
        f'<sheet name="Hidden" sheetId="2" r:id="rId4" state="hidden"/>'
        f"</sheets></workbook>"
    )
    parts["xl/_rels/workbook.xml.rels"] = rels(
        "wb",
        [
            ("rId1", f"{OD}/worksheet", "worksheets/sheet1.xml"),
            ("rId4", f"{OD}/worksheet", "worksheets/sheet2.xml"),
            ("rId2", f"{OD}/sharedStrings", "sharedStrings.xml"),
            ("rId3", f"{OD}/styles", "styles.xml"),
        ],
    )
    parts["xl/worksheets/sheet2.xml"] = xml(
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<worksheet xmlns="{NS}"><sheetData>'
        f'<row r="1"><c r="A1"><v>9</v></c></row>'
        f"</sheetData></worksheet>"
    )
    ct = content_types(
        ("/xl/workbook.xml", CT_WB),
        ("/xl/worksheets/sheet1.xml", CT_WS),
        ("/xl/worksheets/sheet2.xml", CT_WS),
        ("/xl/sharedStrings.xml", CT_SST),
        ("/xl/styles.xml", CT_STY),
    )
    parts["[Content_Types].xml"] = ct
    write_xlsx(OUT / "l2_hidden_sheet.xlsx", parts)
    (OUT / "l2_hidden_sheet.json").write_text(
        json.dumps(
            {"sheets": [{"name": "Visible", "hidden": False}, {"name": "Hidden", "hidden": True}]},
            indent=2,
        )
        + "\n"
    )


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    gen_l1_values()
    gen_l1_formulas()
    gen_l2_merges_freeze()
    gen_l2_names()
    gen_l2_hyperlinks()
    gen_l2_table()
    gen_l2_comments()
    gen_omacell_part()
    gen_l2_print()
    gen_hidden_sheet()
    print(f"wrote corpus under {OUT}")


if __name__ == "__main__":
    main()
