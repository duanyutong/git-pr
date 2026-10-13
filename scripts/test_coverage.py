#!/usr/bin/env -S uv run
"""Regression checks for coverage inventories and acceptance decisions."""

import dataclasses
import json
import pathlib
import tempfile
import unittest
import unittest.mock

import unit_coverage


class CoveragePolicyTests(unittest.TestCase):
    """Reject changes which could conceal uncovered production sources."""

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        patch = unittest.mock.patch.object(unit_coverage, "ROOT", self.root)
        patch.start()
        self.addCleanup(patch.stop)
        self.unit = ("src/first/policy.rs", "src/second/policy.rs")
        self.adapter = "src/adapter.rs"
        for name in (*self.unit, self.adapter):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fn example() {}\n")
        self.policy = self.root / "coverage-policy.toml"
        self.policy.write_text(
            '[application]\npackage = "example"\nsource_root = "src"\nminimum_lines = 95\n'
            'target = ["--lib"]\ntest_filter = ""\ntest_skip = ["::repositories::"]\n'
            f'unit = {json.dumps(self.unit)}\nmixed = ["{self.adapter}"]\n'
            "boundary = []\ndeclarations = []\ngenerated = []\n"
        )
        self.suite = unit_coverage.suites()[0]

    def report(self, names: tuple[str, ...], *, covered: int = 95) -> pathlib.Path:
        """Write a report with colliding basenames and explicit aggregate counts."""
        data = {
            "files": [
                {
                    "filename": str(self.root / name),
                    "summary": {
                        metric: {"count": 1, "covered": 1} for metric in unit_coverage.METRICS
                    },
                }
                for name in names
            ],
            "totals": {"lines": {"count": 100, "covered": covered, "percent": 95.0}},
        }
        path = self.root / "report.json"
        path.write_text(json.dumps({"data": [data]}))
        return path

    def test_rejects_new_or_missing_production_sources(self) -> None:
        extra = self.root / "src/new.rs"
        extra.write_text("fn new() {}\n")
        with self.assertRaisesRegex(AssertionError, "unclassified"):
            unit_coverage.suites()
        extra.unlink()
        (self.root / self.adapter).unlink()
        with self.assertRaisesRegex(AssertionError, "missing"):
            unit_coverage.suites()

    def test_rejects_duplicate_classifications_and_unknown_fields(self) -> None:
        original = self.policy.read_text()
        self.policy.write_text(original.replace("boundary = []", f'boundary = ["{self.adapter}"]'))
        with self.assertRaisesRegex(AssertionError, "duplicate"):
            unit_coverage.suites()
        self.policy.write_text(original + "unknown_lines = 95\n")
        with self.assertRaisesRegex(AssertionError, "invalid policy fields"):
            unit_coverage.suites()

    def test_ignores_conventional_test_implementation_paths(self) -> None:
        for name in ["src/tests/unit.rs", "src/first/tests.rs"]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch()
        self.assertEqual(unit_coverage.suites(), (self.suite,))

    def test_complete_report_distinguishes_identical_basenames(self) -> None:
        totals = unit_coverage.enforce(self.report(self.unit), self.suite)
        self.assertEqual(totals["lines"], {"count": 2, "covered": 2})

    def test_rejects_missing_or_external_unit_report_entries(self) -> None:
        for names in [self.unit[:1], (*self.unit, self.adapter)]:
            with (
                self.subTest(names=names),
                self.assertRaisesRegex(AssertionError, "report boundary"),
            ):
                unit_coverage.enforce(self.report(names), self.suite)

    def test_rejects_uncovered_or_empty_unit_metrics(self) -> None:
        for metric in unit_coverage.METRICS:
            for count in [0, 2]:
                with self.subTest(metric=metric, count=count):
                    path = self.report(self.unit)
                    data = json.loads(path.read_text())
                    data["data"][0]["files"][0]["summary"][metric] = {"count": count, "covered": 0}
                    path.write_text(json.dumps(data))
                    with self.assertRaisesRegex(AssertionError, "incomplete unit"):
                        unit_coverage.enforce(path, self.suite)

    def test_package_requires_adapters_and_uses_integer_thresholds(self) -> None:
        with self.assertRaisesRegex(AssertionError, "package boundary"):
            unit_coverage.enforce_package(self.report(self.unit), self.suite)
        names = (*self.unit, self.adapter)
        for minimum in (95, 100):
            with self.subTest(minimum=minimum):
                suite = dataclasses.replace(self.suite, minimum_lines=minimum)
                path = self.report(names, covered=minimum)
                unit_coverage.enforce_package(path, suite)
                data = json.loads(path.read_text())
                data["data"][0]["totals"]["lines"] = {
                    "count": 100000,
                    "covered": minimum * 1000 - 1,
                    "percent": float(minimum),
                }
                path.write_text(json.dumps(data))
                with self.assertRaisesRegex(AssertionError, f"below {minimum}%"):
                    unit_coverage.enforce_package(path, suite)

    def test_declared_selection_excludes_repository_execution(self) -> None:
        arguments = unit_coverage.test_arguments(self.suite)
        self.assertEqual(arguments[-3:], ["--", "--skip", "::repositories::"])
        filtered = dataclasses.replace(self.suite, test_filter="tests::unit::", test_skip=())
        self.assertEqual(unit_coverage.test_arguments(filtered)[-2:], ["--", "tests::unit::"])

    def test_branch_gate_requires_complete_nonempty_integer_outcomes(self) -> None:
        complete = {"count": 1434, "covered": 1434, "notcovered": 0, "percent": 100.0}
        unit_coverage.enforce_branches({"branches": complete})
        for changes in [
            {"count": 0, "covered": 0},
            {"covered": 1433, "notcovered": 1},
            {"covered": 1433},
            {"notcovered": 1},
            {"covered": 1435},
            {"count": 1434.0},
            {"notcovered": False},
        ]:
            with (
                self.subTest(changes=changes),
                self.assertRaisesRegex(AssertionError, "incomplete unit branch"),
            ):
                unit_coverage.enforce_branches({"branches": complete | changes})


if __name__ == "__main__":
    unittest.main()
