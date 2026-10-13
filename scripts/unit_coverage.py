#!/usr/bin/env -S uv run
"""Measure the declared unit boundary without repository or external execution."""

import argparse
import dataclasses
import json
import os
import pathlib
import re
import subprocess
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
METRICS = ("lines", "regions", "functions")


@dataclasses.dataclass(frozen=True)
class Suite:
    """An explicit production inventory and its unit-only test selection."""

    name: str
    package: str
    source_root: str
    minimum_lines: int
    target: tuple[str, ...]
    test_filter: str
    test_skip: tuple[str, ...]
    unit: tuple[str, ...]
    mixed: tuple[str, ...]
    boundary: tuple[str, ...]
    declarations: tuple[str, ...]
    generated: tuple[str, ...]


def suites() -> tuple[Suite, ...]:
    """Load the sole coverage policy and reject omitted or duplicate modules."""
    policy = tomllib.loads((ROOT / "coverage-policy.toml").read_text())
    result = []
    for name, values in policy.items():
        expected = {field.name for field in dataclasses.fields(Suite)} - {"name"}
        if values.keys() != expected:
            raise AssertionError(f"{name}: invalid policy fields: {values.keys() ^ expected}")
        suite = Suite(
            name=name,
            package=values["package"],
            source_root=values["source_root"],
            minimum_lines=values["minimum_lines"],
            target=tuple(values["target"]),
            test_filter=values["test_filter"],
            test_skip=tuple(values["test_skip"]),
            unit=tuple(values["unit"]),
            mixed=tuple(values["mixed"]),
            boundary=tuple(values["boundary"]),
            declarations=tuple(values["declarations"]),
            generated=tuple(values["generated"]),
        )
        classified = (
            *suite.unit,
            *suite.mixed,
            *suite.boundary,
            *suite.declarations,
            *suite.generated,
        )
        actual = {
            str(path.relative_to(ROOT))
            for path in (ROOT / suite.source_root).rglob("*.rs")
            if "tests" not in path.relative_to(ROOT / suite.source_root).parts
            and path.name != "tests.rs"
        }
        if len(classified) != len(set(classified)):
            raise AssertionError(f"{name}: duplicate module classifications")
        if set(classified) != actual:
            raise AssertionError(
                f"{name}: unclassified={actual - set(classified)}; "
                f"missing={set(classified) - actual}"
            )
        if not suite.unit:
            raise AssertionError(f"{name}: the unit boundary must not be empty")
        result.append(suite)
    return tuple(result)


def report_files(report: pathlib.Path) -> dict:
    """Identify coverage entries by their complete repository-relative path."""
    data = json.loads(report.read_text())["data"][0]
    return {
        str(pathlib.Path(item["filename"]).resolve().relative_to(ROOT.resolve())): item
        for item in data["files"]
    }


def enforce(report: pathlib.Path, suite: Suite) -> dict:
    """Require every declared production file to have zero uncovered code."""
    files = report_files(report)
    if set(files) != set(suite.unit):
        raise AssertionError(
            f"{suite.name}: report boundary differs: "
            f"missing={set(suite.unit) - set(files)}; extra={set(files) - set(suite.unit)}"
        )
    totals = {metric: {"count": 0, "covered": 0} for metric in METRICS}
    for filename in suite.unit:
        for metric in METRICS:
            counts = files[filename]["summary"][metric]
            if counts["count"] == 0 or counts["covered"] != counts["count"]:
                raise AssertionError(f"{filename}: incomplete unit {metric}: {counts}")
            for key in ("count", "covered"):
                totals[metric][key] += counts[key]
    return totals


def enforce_package(report: pathlib.Path, suite: Suite) -> None:
    """Require the package minimum and retain complete coverage of unit policy."""
    files = report_files(report)
    required = {*suite.unit, *suite.mixed, *suite.boundary}
    allowed = required | set(suite.declarations) | set(suite.generated)
    if not required <= files.keys() or not files.keys() <= allowed:
        raise AssertionError(
            f"{suite.name}: package boundary differs: "
            f"missing={required - files.keys()}; extra={files.keys() - allowed}"
        )
    counts = json.loads(report.read_text())["data"][0]["totals"]["lines"]
    if counts["count"] == 0 or counts["covered"] * 100 < counts["count"] * suite.minimum_lines:
        raise AssertionError(f"{suite.name}: package lines below {suite.minimum_lines}%: {counts}")
    for filename in suite.unit:
        for metric in METRICS:
            counts = files[filename]["summary"][metric]
            if counts["count"] == 0 or counts["covered"] != counts["count"]:
                raise AssertionError(f"{filename}: incomplete package {metric}: {counts}")


def enforce_branches(totals: dict) -> None:
    """Require nonempty, complete unit branch outcomes from the pinned compiler."""
    counts = totals["branches"]
    if (
        any(type(counts[key]) is not int for key in ("count", "covered", "notcovered"))
        or counts["count"] <= 0
        or counts["covered"] != counts["count"]
        or counts["notcovered"] != 0
    ):
        raise AssertionError(f"incomplete unit branch outcomes: {counts}")


def exclusion_pattern(suite: Suite, *, unit_only: bool) -> str:
    """Exclude test implementations and, for the unit gate, non-unit sources."""
    pattern = r"/tests/|/target/validation/|/tests\.rs$"
    if unit_only:
        excluded = (*suite.mixed, *suite.boundary, *suite.declarations, *suite.generated)
        if excluded:
            pattern += "|(" + "|".join(re.escape("/" + path) for path in excluded) + ")$"
    return pattern


def test_arguments(suite: Suite) -> list[str]:
    """Select the declared suite, excluding repository tests where specified."""
    arguments = ["-p", suite.package, *suite.target, "--locked"]
    filters = [suite.test_filter] if suite.test_filter else []
    for skipped in suite.test_skip:
        filters.extend(["--skip", skipped])
    if filters:
        arguments.extend(["--", *filters])
    return arguments


def run(arguments: list[str], environment: dict[str, str]) -> None:
    """Run a bounded collection command and preserve its diagnostics."""
    subprocess.run(arguments, cwd=ROOT, env=environment, check=True, timeout=1200)


def collect(suite: Suite) -> dict:
    """Use a private profile directory for a vetted unit-only test suite."""
    environment = dict(os.environ)
    environment["CARGO_LLVM_COV_TARGET_DIR"] = str(ROOT / "target/unit-coverage" / suite.name)
    environment["CARGO_LLVM_COV_BUILD_DIR"] = environment["CARGO_LLVM_COV_TARGET_DIR"]
    environment.pop("LLVM_PROFILE_FILE", None)
    environment.setdefault("RUST_TEST_THREADS", "6")
    run(["cargo", "llvm-cov", "clean", "--workspace"], environment)
    tests = ["cargo", "llvm-cov", "--no-report", *test_arguments(suite)]
    run(tests, environment)
    directory = ROOT / "coverage/unit" / suite.name
    directory.mkdir(parents=True, exist_ok=True)
    base = [
        "cargo",
        "llvm-cov",
        "report",
        "-p",
        suite.package,
        "--ignore-filename-regex",
        exclusion_pattern(suite, unit_only=True),
    ]
    report = directory / "coverage.json"
    run([*base, "--json", "--output-path", str(report)], environment)
    run([*base, "--html", "--output-dir", str(directory)], environment)
    with (directory / "summary.txt").open("w") as output:
        subprocess.run(
            [*base, "--show-missing-lines"],
            cwd=ROOT,
            env=environment,
            check=True,
            stdout=output,
            timeout=120,
        )
    totals = enforce(report, suite)
    print(f"{suite.name}: {len(suite.unit)} modules; 100% unit lines, regions and functions")
    return {"files": list(suite.unit), "metrics": totals}


def main() -> None:
    """Enforce independent unit acceptance for every declared package."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-only", action="store_true", help="run unit tests without coverage")
    arguments = parser.parse_args()
    declared = suites()
    if arguments.test_only:
        for suite in declared:
            run(["cargo", "test", *test_arguments(suite)], dict(os.environ))
        return
    (ROOT / "coverage/unit/summary.json").unlink(missing_ok=True)
    results = {suite.name: collect(suite) for suite in declared}
    (ROOT / "coverage/unit/summary.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
