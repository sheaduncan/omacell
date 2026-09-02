#!/usr/bin/env python3
"""Validate the §12.1 manifest and optional fixed-host JSON-lines results."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys


EXPECTED = {
    "gui_cold_start_ms",
    "tui_cold_start_ms",
    "csv_100mb_first_paint_ms",
    "csv_100mb_loaded_ms",
    "xlsx_50mb_open_ms",
    "xlsx_50mb_save_ms",
    "recalc_incremental_100k_ms",
    "recalc_full_1m_ms",
    "keystroke_to_paint_ms",
    "scroll_frame_ms",
    "memory_1m_x20_bytes",
    "theme_reload_ms",
    "inline_completion_first_token_ms",
    "local_plan_ms",
    "cloud_plan_ms",
    "ai_batch_cells",
    "workbook_card_columns_1m_ms",
}


def load_manifest(root: Path) -> dict[str, dict[str, object]]:
    path = root / "benchmarks/release-budgets.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        raise ValueError("release budget schema_version must be 1")
    if data.get("regression_tolerance_percent") != 10:
        raise ValueError("release regression tolerance must be 10 percent")
    metrics = data.get("metrics")
    if not isinstance(metrics, list):
        raise ValueError("metrics must be an array")
    by_id: dict[str, dict[str, object]] = {}
    for metric in metrics:
        if not isinstance(metric, dict) or not isinstance(metric.get("id"), str):
            raise ValueError("every metric needs a string id")
        identifier = metric["id"]
        if identifier in by_id:
            raise ValueError(f"duplicate metric: {identifier}")
        direction = metric.get("direction")
        target = metric.get("target")
        budget = metric.get("budget")
        if direction not in {"maximum", "minimum"}:
            raise ValueError(f"{identifier}: invalid direction")
        if not isinstance(target, (int, float)) or not isinstance(budget, (int, float)):
            raise ValueError(f"{identifier}: target and budget must be numeric")
        expected = target * (1.1 if direction == "maximum" else 0.9)
        if abs(budget - expected) > max(abs(expected) * 0.00001, 0.00001):
            raise ValueError(f"{identifier}: budget is not the 10 percent boundary")
        by_id[identifier] = metric
    missing = EXPECTED - set(by_id)
    extra = set(by_id) - EXPECTED
    if missing or extra:
        raise ValueError(f"metric coverage mismatch; missing={sorted(missing)}, extra={sorted(extra)}")
    return by_id


def check_results(path: Path, manifest: dict[str, dict[str, object]]) -> None:
    seen: set[str] = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        result = json.loads(line)
        identifier = result.get("id")
        value = result.get("value")
        if identifier not in manifest or not isinstance(value, (int, float)):
            raise ValueError(f"{path}:{line_number}: unknown metric or non-numeric value")
        metric = manifest[identifier]
        budget = metric["budget"]
        direction = metric["direction"]
        failed = value > budget if direction == "maximum" else value < budget
        if failed:
            raise ValueError(
                f"{identifier}: measured {value} {metric['unit']}, budget {budget} {metric['unit']}"
            )
        seen.add(identifier)
    missing = set(manifest) - seen
    if missing:
        raise ValueError(f"fixed-host result is incomplete: {sorted(missing)}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("results", nargs="?", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    manifest = load_manifest(root)
    if args.results is not None:
        check_results(args.results, manifest)
        print(f"fixed-host results pass: {len(manifest)} metrics")
    else:
        print(f"release budgets valid: {len(manifest)} metrics at 10 percent tolerance")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        print(f"performance gate failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
