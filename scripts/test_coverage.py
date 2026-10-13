#!/usr/bin/env -S uv run
"""Regression checks for coverage inventories and acceptance decisions."""

import copy
import dataclasses
import json
import pathlib
import tempfile
import unittest
import unittest.mock

import parity
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


class ParityConfigurationTests(unittest.TestCase):
    """Retain exact setting budgets when a batch includes a declared branch scope."""

    def test_completion_reordering_retains_rows_and_rejects_failed_candidate_listings(self) -> None:
        listing = "\nStack of 2 (newest at the top):\n\nPR two\n\nPR one\n"
        prefix, native = "Updating PR(s)...\n", "GitHub Stack updated: a b\n"
        reference = {"status": 0, "stderr": "", "stdout": prefix + listing + native}
        candidate = dict(reference, stdout=prefix + native + listing)
        expected = parity.completed_listing("native-enabled", reference, reference=True)
        self.assertEqual(
            expected, parity.completed_listing("native-enabled", candidate, reference=False)
        )
        changed = dict(candidate, stdout=candidate["stdout"].replace("PR two", "PR wrong"))
        self.assertNotEqual(
            expected, parity.completed_listing("native-enabled", changed, reference=False)
        )
        missing = dict(candidate, stdout=prefix + native)
        self.assertNotEqual(
            expected, parity.completed_listing("native-enabled", missing, reference=False)
        )
        failed = dict(candidate, status=1)
        with self.assertRaises(AssertionError):
            parity.completed_listing("native-missing", failed, reference=False)
        self.assertEqual(
            parity.completed_listing("native-missing", failed, reference=True)["completed_listing"],
            "",
        )
        with self.assertRaises(AssertionError):
            parity.completed_listing("unexpected-failure", failed, reference=True)

    def test_preview_orientation_preserves_rows_and_detects_unexpected_changes(self) -> None:
        first = "12345678 (topic/1) First commit\n"
        last = "abcdefab  Last commit\n"
        for args, upwards in [([], True), (["--stack-up=false"], False)]:
            reference = {"status": 0, "stdout": first + last + "\nOther output\n", "stderr": ""}
            candidate = dict(
                reference, stdout=(last + first if upwards else first + last) + "\nOther output\n"
            )
            expected = parity.declared_output("orientation", reference, args, reference=True)
            self.assertEqual(
                expected, parity.declared_output("orientation", candidate, args, reference=False)
            )
            for text in [
                candidate["stdout"].replace("First commit", "Wrong commit"),
                "\nOther output\n",
            ]:
                changed = dict(candidate, stdout=text)
                self.assertNotEqual(
                    expected, parity.declared_output("orientation", changed, args, reference=False)
                )

    def test_invalid_range_still_requires_matching_output_and_zero_effects(self) -> None:
        snapshot = {
            "commits": "a\0tree\0\0message\0",
            "refs": "refs/heads/main|a",
            "checkout": "a",
            "working_copy": "",
            "prs": {},
        }
        reference = {
            "initial_snapshot": snapshot,
            "snapshot": [snapshot],
            "outputs": [{"status": 1, "stdout": "", "stderr": "invalid range"}],
            "phases": [{"args": ["a...b"]}],
            "trace": [],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory)
            parity.compare("invalid-range", reference, copy.deepcopy(reference), path)
            for field in ["trace", "outputs", "snapshot"]:
                candidate = copy.deepcopy(reference)
                if field == "trace":
                    candidate[field] = [{"event": "start", "program": "git", "args": ["status"]}]
                elif field == "outputs":
                    candidate[field][0]["status"] = 0
                else:
                    candidate[field] = [copy.deepcopy(snapshot)]
                    candidate[field][0]["commits"] += "b\0tree\0\0unexpected object\0"
                with self.subTest(field=field), self.assertRaises(AssertionError):
                    parity.compare("invalid-range", reference, candidate, path)

    def test_native_override_omission_does_not_hide_duplicate_or_required_reads(self) -> None:
        event = {
            "event": "start",
            "program": "git",
            "args": ["config", "--type=bool", "--get", "git-pr.githubStack"],
        }
        for reference in [False, True]:
            for flag in [[], ["--github-stack"]]:
                for reads in [0, 1, 2]:
                    evidence = {
                        "trace": [event] * reads,
                        "phases": [{"args": flag, "github_stack_setting_reads": reads}],
                    }
                    with self.subTest(reference=reference, flag=flag, reads=reads):
                        if flag and reads > int(reference):
                            with self.assertRaises(AssertionError):
                                parity.publication_contract(evidence, reference=reference)
                        else:
                            contract = parity.publication_contract(evidence, reference=reference)
                            self.assertEqual(sum(contract.values()), 0 if flag else reads)

    def test_expands_destination_scope_without_concealing_existing_reads(self) -> None:
        event = {
            "event": "start",
            "program": "git",
            "args": [
                "config",
                "--null",
                "--get-regexp",
                r"^(user\.name|remote\.pushdefault|branch\..*\.(pushremote|remote))$",
            ],
        }
        self.assertEqual(
            parity.configuration_reads(event),
            [
                ["config", "--get", "user.name"],
                ["config", "--get", "remote.pushDefault"],
                ["config", "--get", "branch.*.pushRemote"],
                ["config", "--get", "branch.*.remote"],
            ],
        )
        self.assertEqual(set(parity.new_git_settings(event)), parity.REMOTE_GIT_SETTINGS)
        self.assertEqual(
            parity.comparable_trace([event]),
            {json.dumps(["git", "config", "--get", "user.name"]): 1},
        )

    def test_rejects_unexpected_scopes_and_duplicate_settings(self) -> None:
        for pattern in [
            r"^()$",
            r"^(remote\.pushdefault||user\.name)$",
            r"^(branch\..*)$",
            r"^(branch\..*\.(pushremote|unknown))$",
            r"^(remote\.pushdefault|remote\.pushdefault)$",
            r"^(unknown\.key)$",
        ]:
            with self.subTest(pattern=pattern), self.assertRaises(AssertionError):
                parity.configuration_reads(
                    {"program": "git", "args": ["config", "--null", "--get-regexp", pattern]}
                )


if __name__ == "__main__":
    unittest.main()
