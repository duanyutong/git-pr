#!/usr/bin/env -S uv run
"""Regression checks for coverage inventories and acceptance decisions."""

import dataclasses
import io
import json
import os
import pathlib
import re
import shutil
import subprocess
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
            '[application]\npackage = "example"\nsource_root = "src"\n'
            'target = ["--lib"]\ntest_filter = "::unit::"\ntest_skip = []\n'
            f'unit = {json.dumps(self.unit)}\nmixed = ["{self.adapter}"]\n'
            "boundary = []\ndeclarations = []\ngenerated = []\n"
        )
        self.suite = unit_coverage.suites()[0]

    def report(self, names: tuple[str, ...]) -> pathlib.Path:
        """Write production entries with colliding basenames and exact counts."""
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
        for field in ["unknown_lines", "minimum_lines"]:
            self.policy.write_text(original + f"{field} = 95\n")
            with (
                self.subTest(field=field),
                self.assertRaisesRegex(AssertionError, "invalid policy fields"),
            ):
                unit_coverage.suites()

    def test_rejects_empty_unit_scope(self) -> None:
        self.policy.write_text(
            self.policy.read_text()
            .replace(f"unit = {json.dumps(self.unit)}", "unit = []")
            .replace("boundary = []", f"boundary = {json.dumps(self.unit)}")
        )
        with self.assertRaisesRegex(AssertionError, "unit boundary must not be empty"):
            unit_coverage.suites()

    def test_rejects_empty_policy(self) -> None:
        self.policy.write_text("")
        with self.assertRaisesRegex(AssertionError, "must declare a unit suite"):
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
            for count, covered in [(0, 0), (2, 0), (100000, 99999)]:
                with self.subTest(metric=metric, count=count, covered=covered):
                    path = self.report(self.unit)
                    data = json.loads(path.read_text())
                    data["data"][0]["files"][0]["summary"][metric] = {
                        "count": count,
                        "covered": covered,
                        "percent": 100.0,
                    }
                    path.write_text(json.dumps(data))
                    with self.assertRaisesRegex(AssertionError, "incomplete unit"):
                        unit_coverage.enforce(path, self.suite)

    def test_rejects_invalid_counts(self) -> None:
        for metric in unit_coverage.METRICS:
            for count, covered in [(-1, -1), (1, 2), (True, True), (1.0, 1), (1, 1.0)]:
                with self.subTest(metric=metric, count=count, covered=covered):
                    path = self.report(self.unit)
                    data = json.loads(path.read_text())
                    data["data"][0]["files"][0]["summary"][metric] = {
                        "count": count,
                        "covered": covered,
                    }
                    path.write_text(json.dumps(data))
                    with self.assertRaisesRegex(AssertionError, "incomplete unit"):
                        unit_coverage.enforce(path, self.suite)

    def test_rejects_duplicate_and_foreign_report_sources(self) -> None:
        path = self.report((*self.unit, self.unit[0]))
        with self.assertRaisesRegex(AssertionError, "duplicate report source"):
            unit_coverage.enforce(path, self.suite)
        path = self.report((*self.unit, "../foreign.rs"))
        with self.assertRaisesRegex(AssertionError, "outside checkout"):
            unit_coverage.enforce(path, self.suite)

    def test_rejects_empty_or_multiple_report_datasets(self) -> None:
        for count in [0, 2]:
            path = self.report(self.unit)
            data = json.loads(path.read_text())
            data["data"] *= count
            path.write_text(json.dumps(data))
            with (
                self.subTest(count=count),
                self.assertRaisesRegex(AssertionError, "exactly one"),
            ):
                unit_coverage.enforce(path, self.suite)

    def test_accepts_production_generics_instantiated_with_test_doubles(self) -> None:
        path = self.report(self.unit)
        data = json.loads(path.read_text())
        data["data"][0]["functions"] = [
            {
                "name": "production_policy::<tests::FakeBoundary>",
                "filenames": [str(self.root / self.unit[0])],
                "count": 1,
            }
        ]
        for item in data["data"][0]["files"]:
            item["summary"]["instantiations"] = {"count": 2, "covered": 1}
        path.write_text(json.dumps(data))
        self.assertEqual(
            unit_coverage.enforce(path, self.suite)["functions"], {"count": 2, "covered": 2}
        )

    def test_exclusions_do_not_match_checkout_ancestors(self) -> None:
        root = (self.root / "tests" / "project.with-regex+characters").resolve()
        with unittest.mock.patch.object(unit_coverage, "ROOT", root):
            pattern = unit_coverage.exclusion_pattern(self.suite)
        for name in [self.adapter, "src/tests/unit.rs", "src/first/tests.rs", "tests/cli.rs"]:
            with self.subTest(name=name):
                self.assertIsNotNone(re.search(pattern, str(root / name)))
        for name in [*self.unit, "src/adapter.rs.extra", "src/testsuite.rs"]:
            with self.subTest(name=name):
                self.assertIsNone(re.search(pattern, str(root / name)))
        self.assertIsNone(re.search(pattern, str(self.root / self.adapter)))

    def test_failed_policy_validation_removes_stale_success(self) -> None:
        summary = self.root / "coverage/unit/summary.json"
        summary.parent.mkdir(parents=True)
        summary.write_text('{"previous": "success"}\n')
        (self.root / "src/new.rs").touch()
        with (
            unittest.mock.patch("sys.argv", ["unit_coverage.py"]),
            self.assertRaisesRegex(AssertionError, "unclassified"),
        ):
            unit_coverage.main()
        self.assertFalse(summary.exists())

    def test_failed_collection_removes_stale_success(self) -> None:
        summary = self.root / "coverage/unit/summary.json"
        summary.parent.mkdir(parents=True)
        summary.write_text('{"previous": "success"}\n')
        with (
            unittest.mock.patch("sys.argv", ["unit_coverage.py"]),
            unittest.mock.patch.object(
                unit_coverage, "collect", side_effect=RuntimeError("failed")
            ),
            self.assertRaisesRegex(RuntimeError, "failed"),
        ):
            unit_coverage.main()
        self.assertFalse(summary.exists())

    def test_declared_selection_excludes_repository_execution(self) -> None:
        arguments = unit_coverage.test_arguments(self.suite)
        self.assertEqual(arguments[-2:], ["--", "::unit::"])
        skipped = dataclasses.replace(self.suite, test_filter="", test_skip=("::repositories::",))
        self.assertEqual(
            unit_coverage.test_arguments(skipped)[-3:], ["--", "--skip", "::repositories::"]
        )

    def test_integration_profiles_cannot_complete_unit_coverage(self) -> None:
        profiles = self.root / "target/unit-coverage" / self.suite.name
        profiles.mkdir(parents=True)
        contaminated = profiles / "integration.profraw"
        contaminated.write_text("complete integration coverage")
        external = self.root / "integration.profraw"
        external.write_text("external integration coverage")
        complete = self.report(self.unit).read_text()
        data = json.loads(complete)
        data["data"][0]["files"][0]["summary"]["lines"]["covered"] = 0
        incomplete = json.dumps(data)

        def execute(arguments: list[str], environment: dict[str, str]) -> None:
            self.assertEqual(environment["CARGO_LLVM_COV_TARGET_DIR"], str(profiles))
            self.assertEqual(environment["CARGO_LLVM_COV_BUILD_DIR"], str(profiles))
            self.assertNotIn("LLVM_PROFILE_FILE", environment)
            if arguments[2] == "clean":
                shutil.rmtree(profiles)
            elif "--no-report" in arguments:
                self.assertEqual(arguments[3:], unit_coverage.test_arguments(self.suite))
            elif "--json" in arguments:
                output = pathlib.Path(arguments[arguments.index("--output-path") + 1])
                output.write_text(complete if contaminated.exists() else incomplete)

        def text_report(arguments: list[str], *, stdout: io.TextIOBase, **_: object) -> None:
            stdout.write("unit report\n")

        with (
            unittest.mock.patch.dict(os.environ, {"LLVM_PROFILE_FILE": str(external)}),
            unittest.mock.patch.object(unit_coverage, "run", side_effect=execute),
            unittest.mock.patch.object(
                unit_coverage.subprocess, "check_output", return_value="rustc 1.99.1"
            ),
            unittest.mock.patch.object(unit_coverage.subprocess, "run", side_effect=text_report),
            self.assertRaisesRegex(AssertionError, "incomplete unit lines"),
        ):
            unit_coverage.collect(self.suite)
        self.assertFalse(contaminated.exists())
        self.assertEqual(external.read_text(), "external integration coverage")

    def test_retained_reports_identify_compiler_selection_and_source_scope(self) -> None:
        directory = self.root / "reports"
        directory.mkdir()
        (directory / "coverage.json").write_text(self.report(self.unit).read_text())
        (directory / "summary.txt").write_text("Unit coverage table\n")
        context = {
            "compiler": "rustc 1.99.1",
            "tests": unit_coverage.test_arguments(self.suite),
            "files": list(self.unit),
        }
        unit_coverage.annotate_reports(directory, context)
        self.assertEqual(
            json.loads((directory / "coverage.json").read_text())["unit_scope"], context
        )
        for artifact in ["summary.txt", "index.html"]:
            text = (directory / artifact).read_text()
            self.assertIn(context["compiler"], text)
            self.assertIn("::unit::", text)
            for filename in self.unit:
                self.assertIn(filename, text)
        self.assertIn("Unit coverage table", (directory / "summary.txt").read_text())
        self.assertIn('href="html/index.html"', (directory / "index.html").read_text())


class VersioningGuardTests(unittest.TestCase):
    """Exercise the actual recipe against covered and uncovered disposable scripts."""

    def test_guard_retains_reports_and_displays_details_only_on_failure(self) -> None:
        repository = pathlib.Path(__file__).resolve().parents[1]
        for complete in [True, False]:
            with self.subTest(complete=complete), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                (root / "scripts").mkdir()
                (root / "scripts/version.py").write_text(
                    'def selected():\n    return "covered"\n\ndef unused():\n    return "unused"\n'
                )
                (root / "scripts/test_version.py").write_text(
                    "import version\nversion.selected()\n"
                    + ("version.unused()\n" if complete else "")
                )
                configuration = root / ".coveragerc"
                configuration.write_text(
                    "[run]\ninclude = */scripts/version.py\ndata_file = coverage/.versioning\n"
                    "parallel = True\n[report]\nshow_missing = True\nprecision = 2\n"
                )
                result = subprocess.run(
                    [
                        "just",
                        "--justfile",
                        str(repository / "justfile"),
                        "--working-directory",
                        str(root),
                        "coverage-versioning",
                    ],
                    env=dict(
                        os.environ,
                        UV_PROJECT=str(repository),
                        COVERAGE_RCFILE=str(configuration),
                    ),
                    capture_output=True,
                    text=True,
                    timeout=60,
                    check=False,
                )
                output = result.stdout + result.stderr
                if complete:
                    self.assertEqual(result.returncode, 0, output)
                    self.assertIn("Versioning sanity guard: passed", output)
                    self.assertNotIn("TOTAL", output)
                else:
                    self.assertNotEqual(result.returncode, 0, output)
                    self.assertIn("TOTAL", output)
                    self.assertIn("Missing", output)
                    self.assertNotIn("Versioning sanity guard: passed", output)
                for artifact in [
                    "versioning.json",
                    "versioning/index.html",
                    "versioning-summary.txt",
                ]:
                    self.assertTrue((root / "coverage" / artifact).is_file(), output)


if __name__ == "__main__":
    unittest.main()
