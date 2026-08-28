#!/usr/bin/env python3
"""Validate the committed Gate G1 performance baseline manifest."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise ValueError(message)


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    path = root / "benchmarks/g1-baselines.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1 or data.get("gate") != "G1":
        fail("expected schema_version 1 for gate G1")
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
        maximum = entry.get("maximum_ns")
        if maximum is not None and estimate > maximum:
            fail(f"{identifier}: estimate exceeds its product budget")
        interval = entry.get("confidence_interval_ns")
        if interval is not None and not interval[0] <= estimate <= interval[1]:
            fail(f"{identifier}: estimate is outside its confidence interval")
        throughput = entry.get("throughput_elements_per_second")
        minimum = entry.get("minimum_elements_per_second")
        if minimum is not None and (throughput is None or throughput < minimum):
            fail(f"{identifier}: throughput is below its product budget")
        source = entry.get("source")
        if not isinstance(source, str) or not (root / source).is_file():
            fail(f"{identifier}: source report does not exist")
    memory = data.get("memory")
    if not isinstance(memory, list) or not memory:
        fail("memory baselines must be a non-empty list")
    for entry in memory:
        identifier = entry.get("id")
        if not isinstance(identifier, str) or not identifier:
            fail("every memory baseline needs an id")
        per_cell = entry.get("bytes_per_cell")
        per_cell_max = entry.get("maximum_bytes_per_cell")
        if per_cell_max is not None and per_cell > per_cell_max:
            fail(f"{identifier}: bytes per cell exceeds its product budget")
        total = entry.get("bytes_total")
        total_max = entry.get("maximum_bytes_total")
        if total_max is not None and total > total_max:
            fail(f"{identifier}: total bytes exceeds its product budget")
        source = entry.get("source")
        if not isinstance(source, str) or not (root / source).is_file():
            fail(f"{identifier}: source report does not exist")
    print(f"G1 baselines valid: {len(entries)} criterion, {len(memory)} memory")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, TypeError, ValueError) as error:
        print(f"G1 baseline validation failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
