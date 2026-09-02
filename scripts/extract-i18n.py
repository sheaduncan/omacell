#!/usr/bin/env python3
"""Check Fluent ids and reject direct GUI widget string literals."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "crates/gui/src"
BUNDLE = ROOT / "i18n/en-US/omacell.ftl"
TR = re.compile(r'\btr\("([a-z][a-z0-9-]*)"\)')
DIRECT_WIDGET = re.compile(
    r'(?:ui\.(?:label|button|heading|menu_button)|RichText::new|Window::new|\.hint_text)\("'
)


def bundle_ids() -> set[str]:
    ids: set[str] = set()
    for line_number, line in enumerate(BUNDLE.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or stripped.startswith("."):
            continue
        if "=" not in line:
            raise ValueError(f"{BUNDLE}:{line_number}: expected Fluent message assignment")
        identifier = line.split("=", 1)[0].strip()
        if not re.fullmatch(r"[a-z][a-z0-9-]*", identifier):
            raise ValueError(f"{BUNDLE}:{line_number}: invalid message id {identifier!r}")
        if identifier in ids:
            raise ValueError(f"{BUNDLE}:{line_number}: duplicate message id {identifier}")
        ids.add(identifier)
    return ids


def source_ids() -> tuple[set[str], list[str]]:
    ids: set[str] = set()
    hard_coded: list[str] = []
    for path in sorted(SOURCE.glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        ids.update(TR.findall(text))
        for line_number, line in enumerate(text.splitlines(), 1):
            if DIRECT_WIDGET.search(line):
                hard_coded.append(f"{path.relative_to(ROOT)}:{line_number}: {line.strip()}")
    return ids, hard_coded


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", required=True)
    parser.parse_args()
    messages = bundle_ids()
    referenced, hard_coded = source_ids()
    missing = referenced - messages
    unused = messages - referenced
    if missing:
        print(f"missing Fluent messages: {sorted(missing)}", file=sys.stderr)
    if unused:
        print(f"unused Fluent messages: {sorted(unused)}", file=sys.stderr)
    if hard_coded:
        print("hard-coded GUI widget strings:", file=sys.stderr)
        print("\n".join(hard_coded), file=sys.stderr)
    return int(bool(missing or unused or hard_coded))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(f"i18n extraction failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
