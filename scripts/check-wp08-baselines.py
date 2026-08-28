#!/usr/bin/env python3
"""Validate the committed WP-08 CSV performance baselines."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise ValueError(message)


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    path = root / "benchmarks/wp08-baselines.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1 or data.get("work_package") != "WP-08":
        fail("expected schema_version 1 for WP-08")

    entries = data.get("criterion")
    if not isinstance(entries, list) or not entries:
        fail("criterion baselines must be a non-empty list")
    ids: set[str] = set()
    for entry in entries:
        identifier = entry.get("id")
        estimate = entry.get("estimate_ns")
        if not isinstance(identifier, str) or not identifier:
            fail("every criterion baseline needs an id")
        if identifier in ids:
            fail(f"duplicate criterion baseline {identifier}")
        ids.add(identifier)
        if not isinstance(estimate, (int, float)) or estimate <= 0:
            fail(f"{identifier}: estimate_ns must be positive")
        interval = entry.get("confidence_interval_ns")
        if interval is not None and not interval[0] <= estimate <= interval[1]:
            fail(f"{identifier}: estimate is outside its confidence interval")
        throughput = entry.get("throughput_bytes_per_second")
        minimum = entry.get("minimum_bytes_per_second")
        if not isinstance(throughput, (int, float)) or throughput <= 0:
            fail(f"{identifier}: throughput must be positive")
        if minimum is not None and throughput < minimum:
            fail(f"{identifier}: throughput is below its product budget")
        source = entry.get("source")
        if not isinstance(source, str) or not (root / source).is_file():
            fail(f"{identifier}: source report does not exist")

    memory = data.get("memory")
    if not isinstance(memory, list) or not memory:
        fail("memory baselines must be a non-empty list")
    for entry in memory:
        identifier = entry.get("id")
        total = entry.get("bytes_total")
        maximum = entry.get("maximum_bytes_total")
        if not isinstance(identifier, str) or not identifier:
            fail("every memory baseline needs an id")
        if not isinstance(total, (int, float)) or total < 0:
            fail(f"{identifier}: bytes_total must be non-negative")
        if maximum is not None and total > maximum:
            fail(f"{identifier}: memory exceeds its budget")
        source = entry.get("source")
        if not isinstance(source, str) or not (root / source).is_file():
            fail(f"{identifier}: source report does not exist")

    print(f"WP-08 baselines valid: {len(entries)} criterion, {len(memory)} memory")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, TypeError, ValueError) as error:
        print(f"WP-08 baseline validation failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
