#!/usr/bin/env -S uv run
"""Measure the declared unit boundary without repository or external execution."""

import argparse
import dataclasses
import html
import json
import os
import pathlib
import re
import shutil
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
    if not policy:
        raise AssertionError("coverage policy must declare a unit suite")
    result = []
    for name, values in policy.items():
        expected = {field.name for field in dataclasses.fields(Suite)} - {"name"}
        if values.keys() != expected:
            raise AssertionError(f"{name}: invalid policy fields: {values.keys() ^ expected}")
        suite = Suite(
            name=name,
            package=values["package"],
            source_root=values["source_root"],
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
    reports = json.loads(report.read_text())["data"]
    if len(reports) != 1:
        raise AssertionError("expected exactly one unit coverage report")
    data = reports[0]
    files = {}
    for item in data["files"]:
        try:
            filename = (
                pathlib.Path(item["filename"]).resolve().relative_to(ROOT.resolve()).as_posix()
            )
        except ValueError as error:
            raise AssertionError(f"report source outside checkout: {item['filename']}") from error
        if filename in files:
            raise AssertionError(f"duplicate report source: {filename}")
        files[filename] = item
    return files


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
            if (
                type(counts["count"]) is not int
                or type(counts["covered"]) is not int
                or counts["count"] <= 0
                or counts["covered"] != counts["count"]
            ):
                raise AssertionError(f"{filename}: incomplete unit {metric}: {counts}")
            for key in ("count", "covered"):
                totals[metric][key] += counts[key]
    return totals


def exclusion_pattern(suite: Suite) -> str:
    """Exclude declared non-unit sources and tests within this checkout only."""
    root = re.escape(ROOT.resolve().as_posix() + "/")
    sources = re.escape(suite.source_root)
    pattern = rf"^{root}({sources}/(.*/)?tests(/|\.rs$)|tests/|\.validation/|target/)"
    excluded = (*suite.mixed, *suite.boundary, *suite.declarations, *suite.generated)
    if excluded:
        pattern += "|^" + root + "(" + "|".join(re.escape(path) for path in excluded) + ")$"
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


def annotate_reports(directory: pathlib.Path, context: dict) -> None:
    """Keep the compiler, selected tests and source scope with each report format."""
    report = directory / "coverage.json"
    data = json.loads(report.read_text())
    data["unit_scope"] = context
    report.write_text(json.dumps(data) + "\n")
    description = (
        "Rust unit coverage\n"
        f"Compiler: {context['compiler']}\n"
        f"Test selection: {json.dumps(context['tests'])}\n"
        "Production sources:\n" + "".join(f"  {filename}\n" for filename in context["files"])
    )
    summary = directory / "summary.txt"
    summary.write_text(description + "\n" + summary.read_text())
    (directory / "index.html").write_text(
        '<!doctype html><html lang="en"><meta charset="utf-8">'
        "<title>Rust unit coverage</title><h1>Rust unit coverage</h1>"
        f"<pre>{html.escape(description)}</pre>"
        '<p><a href="html/index.html">Detailed unit coverage report</a></p></html>\n'
    )


def collect(suite: Suite) -> dict:
    """Use a private profile directory for a vetted unit-only test suite."""
    environment = dict(os.environ)
    environment["CARGO_LLVM_COV_TARGET_DIR"] = str(ROOT / "target/unit-coverage" / suite.name)
    environment["CARGO_LLVM_COV_BUILD_DIR"] = environment["CARGO_LLVM_COV_TARGET_DIR"]
    environment.pop("LLVM_PROFILE_FILE", None)
    environment.setdefault("RUST_TEST_THREADS", "6")
    compiler = subprocess.check_output(
        ["rustc", "--version"], cwd=ROOT, env=environment, text=True, timeout=30
    ).strip()
    context = {
        "compiler": compiler,
        "tests": test_arguments(suite),
        "files": list(suite.unit),
    }
    directory = ROOT / "coverage/unit" / suite.name
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    run(["cargo", "llvm-cov", "clean", "--workspace"], environment)
    tests = ["cargo", "llvm-cov", "--no-report", *test_arguments(suite)]
    run(tests, environment)
    base = [
        "cargo",
        "llvm-cov",
        "report",
        "-p",
        suite.package,
        "--ignore-filename-regex",
        exclusion_pattern(suite),
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
    annotate_reports(directory, context)
    totals = enforce(report, suite)
    print(f"Rust unit coverage: 100% lines, regions and functions ({len(suite.unit)} modules)")
    return {**context, "metrics": totals}


def main() -> None:
    """Enforce independent unit acceptance for every declared package."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-only", action="store_true", help="run unit tests without coverage")
    arguments = parser.parse_args()
    summary = ROOT / "coverage/unit/summary.json"
    if not arguments.test_only:
        summary.unlink(missing_ok=True)
    declared = suites()
    if arguments.test_only:
        for suite in declared:
            run(["cargo", "test", *test_arguments(suite)], dict(os.environ))
        return
    results = {suite.name: collect(suite) for suite in declared}
    summary.write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
