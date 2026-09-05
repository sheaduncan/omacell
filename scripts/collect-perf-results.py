#!/usr/bin/env python3
"""Build one complete, attributable §12.1 result artifact from fresh runs."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import platform
import sys


MARKER = "OMACELL_PERF_RESULT "

# Criterion stores estimates below a sanitized benchmark id. Values are
# nanoseconds and are converted to the units in release-budgets.json.
CRITERION_METRICS = {
    "csv_100mb_first_paint_ms": "csv_product/first_paint_100mb/new/estimates.json",
    "csv_100mb_loaded_ms": "csv_product/full_load_100mb/new/estimates.json",
    "xlsx_50mb_open_ms": "xlsx_open/synthetic_sheet/new/estimates.json",
    "xlsx_50mb_save_ms": "xlsx_save/numeric_86k_x_20/new/estimates.json",
    "recalc_incremental_100k_ms": (
        "recalc/incremental_100k_typical_one_cell/new/estimates.json"
    ),
    "recalc_full_1m_ms": "recalc/full_1m_independent_8t/new/estimates.json",
    "theme_reload_ms": "theme_reload_full_config_and_theme/new/estimates.json",
    "workbook_card_columns_1m_ms": "card_columns_1m_cells/new/estimates.json",
}

LOG_METRICS = {
    "gui_cold_start_ms",
    "tui_cold_start_ms",
    "keystroke_to_paint_ms",
    "scroll_frame_ms",
    "memory_1m_x20_bytes",
    "inline_completion_first_token_ms",
    "local_plan_ms",
    "cloud_plan_ms",
    "ai_batch_cells",
}


def add_result(results: dict[str, dict[str, object]], result: dict[str, object]) -> None:
    identifier = result.get("id")
    value = result.get("value")
    if (
        not isinstance(identifier, str)
        or isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
    ):
        raise ValueError("measurement needs a string id and numeric value")
    if identifier in results:
        raise ValueError(f"duplicate metric: {identifier}")
    results[identifier] = result


def read_log(path: Path, results: dict[str, dict[str, object]]) -> None:
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        marker = line.find(MARKER)
        if marker < 0:
            continue
        result = json.loads(line[marker + len(MARKER) :])
        if result.get("id") not in LOG_METRICS:
            raise ValueError(f"{path}:{line_number}: unknown log metric")
        result["source"] = f"{path.name}:{line_number}"
        add_result(results, result)


def read_criterion(root: Path, results: dict[str, dict[str, object]]) -> None:
    for identifier, relative in CRITERION_METRICS.items():
        path = root / relative
        estimates = json.loads(path.read_text(encoding="utf-8"))
        nanoseconds = estimates["mean"]["point_estimate"]
        if not isinstance(nanoseconds, (int, float)) or nanoseconds <= 0:
            raise ValueError(f"{path}: invalid Criterion mean")
        add_result(
            results,
            {
                "id": identifier,
                "value": nanoseconds / 1_000_000,
                "source": str(path.relative_to(root.parent)),
            },
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--log", required=True, type=Path)
    parser.add_argument("--criterion-root", required=True, type=Path)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    results: dict[str, dict[str, object]] = {}
    read_log(args.log, results)
    read_criterion(args.criterion_root, results)
    expected = set(CRITERION_METRICS) | LOG_METRICS
    if set(results) != expected:
        missing = sorted(expected - set(results))
        extra = sorted(set(results) - expected)
        raise ValueError(f"metric coverage mismatch; missing={missing}, extra={extra}")

    artifact = {
        "schema_version": 1,
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        "commit": args.commit,
        "run_id": args.run_id,
        "host": platform.node(),
        "metrics": [results[key] for key in sorted(results)],
    }
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(results)} fresh measurements to {args.output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, OSError, ValueError, TypeError, json.JSONDecodeError) as error:
        print(f"performance collection failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
