#!/usr/bin/env python3
"""Regression tests for fixed-host performance collection and validation."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


collector = load_module("collect_perf_results", ROOT / "scripts/collect-perf-results.py")
checker = load_module("check_perf_baselines", ROOT / "scripts/check-perf-baselines.py")


class PerformanceResultsTest(unittest.TestCase):
    def test_complete_fresh_artifact_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            temp = Path(directory)
            criterion = temp / "criterion"
            for relative in collector.CRITERION_METRICS.values():
                path = criterion / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(
                    json.dumps({"mean": {"point_estimate": 1_000_000.0}}),
                    encoding="utf-8",
                )
            log = temp / "measurements.log"
            manifest = checker.load_manifest(ROOT)
            log.write_text(
                "\n".join(
                    collector.MARKER
                    + json.dumps(
                        {"id": identifier, "value": manifest[identifier]["target"]}
                    )
                    for identifier in sorted(collector.LOG_METRICS)
                ),
                encoding="utf-8",
            )
            results = {}
            collector.read_log(log, results)
            collector.read_criterion(criterion, results)
            artifact = temp / "results.json"
            artifact.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "recorded_at": datetime.now(timezone.utc).isoformat(),
                        "commit": "abc123",
                        "run_id": "42",
                        "host": "test",
                        "metrics": list(results.values()),
                    }
                ),
                encoding="utf-8",
            )
            checker.check_results(
                artifact,
                manifest,
                "abc123",
                "42",
                1,
            )

    def test_duplicate_and_stale_artifacts_fail(self) -> None:
        manifest = checker.load_manifest(ROOT)
        safe = []
        for identifier, metric in manifest.items():
            target = metric["target"]
            safe.append({"id": identifier, "value": target, "source": "test"})
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / "results.json"
            base = {
                "schema_version": 1,
                "recorded_at": datetime.now(timezone.utc).isoformat(),
                "commit": "abc123",
                "run_id": "42",
                "host": "test",
                "metrics": safe + [safe[0]],
            }
            artifact.write_text(json.dumps(base), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate metric"):
                checker.check_results(artifact, manifest, "abc123", "42", 1)

            base["metrics"] = safe
            base["recorded_at"] = (
                datetime.now(timezone.utc) - timedelta(hours=2)
            ).isoformat()
            artifact.write_text(json.dumps(base), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "stale"):
                checker.check_results(artifact, manifest, "abc123", "42", 1)


if __name__ == "__main__":
    unittest.main()
