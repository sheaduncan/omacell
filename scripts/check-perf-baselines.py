#!/usr/bin/env python3
"""Validate the §12.1 manifest and an optional fresh fixed-host artifact."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
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


def check_results(
    path: Path,
    manifest: dict[str, dict[str, object]],
    expect_commit: str | None,
    expect_run_id: str | None,
    max_age_hours: float,
) -> None:
    artifact = json.loads(path.read_text(encoding="utf-8"))
    if artifact.get("schema_version") != 1:
        raise ValueError("fixed-host result schema_version must be 1")
    if expect_commit is not None and artifact.get("commit") != expect_commit:
        raise ValueError("fixed-host result commit does not match the checked-out commit")
    if expect_run_id is not None and artifact.get("run_id") != expect_run_id:
        raise ValueError("fixed-host result run_id does not match this workflow run")
    recorded = artifact.get("recorded_at")
    if not isinstance(recorded, str):
        raise ValueError("fixed-host result recorded_at is missing")
    recorded_at = datetime.fromisoformat(recorded.replace("Z", "+00:00"))
    if recorded_at.tzinfo is None:
        raise ValueError("fixed-host result recorded_at must include a timezone")
    age_hours = (datetime.now(timezone.utc) - recorded_at).total_seconds() / 3_600
    if age_hours < -0.25 or age_hours > max_age_hours:
        raise ValueError(f"fixed-host result is stale or future-dated: {age_hours:.2f} hours")
    results = artifact.get("metrics")
    if not isinstance(results, list):
        raise ValueError("fixed-host result metrics must be an array")

    seen: set[str] = set()
    for index, result in enumerate(results, 1):
        if not isinstance(result, dict):
            raise ValueError(f"{path}: metric {index} is not an object")
        identifier = result.get("id")
        value = result.get("value")
        if (
            identifier not in manifest
            or isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
        ):
            raise ValueError(f"{path}: metric {index} is unknown or non-numeric")
        if not isinstance(result.get("source"), str) or not result["source"]:
            raise ValueError(f"{identifier}: measurement source is missing")
        if identifier in seen:
            raise ValueError(f"duplicate metric: {identifier}")
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
    parser.add_argument("--expect-commit")
    parser.add_argument("--expect-run-id")
    parser.add_argument("--max-age-hours", type=float, default=24.0)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    manifest = load_manifest(root)
    if args.results is not None:
        if args.max_age_hours <= 0:
            raise ValueError("--max-age-hours must be positive")
        check_results(
            args.results,
            manifest,
            args.expect_commit,
            args.expect_run_id,
            args.max_age_hours,
        )
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
