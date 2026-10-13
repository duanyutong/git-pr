#!/usr/bin/env -S uv run
"""Collect all-target package coverage under the declared coverage policy."""

import argparse
import os

import unit_coverage


def collect(suite: unit_coverage.Suite) -> None:
    """Measure package acceptance independently of the strict unit profiles."""
    root = unit_coverage.ROOT
    environment = dict(os.environ)
    environment["CARGO_LLVM_COV_TARGET_DIR"] = str(root / "target/package-coverage" / suite.name)
    environment["CARGO_LLVM_COV_BUILD_DIR"] = environment["CARGO_LLVM_COV_TARGET_DIR"]
    environment.pop("LLVM_PROFILE_FILE", None)
    environment.setdefault("RUST_TEST_THREADS", "6")
    unit_coverage.run(["cargo", "llvm-cov", "clean", "--workspace"], environment)
    unit_coverage.run(
        [
            "cargo",
            "llvm-cov",
            "-p",
            suite.package,
            "--all-targets",
            "--all-features",
            "--locked",
            "--no-report",
        ],
        environment,
    )
    directory = root / "coverage"
    directory.mkdir(exist_ok=True)
    report = directory / ("coverage.json" if suite.name == "application" else f"{suite.name}.json")
    html = directory if suite.name == "application" else directory / suite.name
    base = [
        "cargo",
        "llvm-cov",
        "report",
        "-p",
        suite.package,
        "--ignore-filename-regex",
        unit_coverage.exclusion_pattern(suite, unit_only=False),
    ]
    unit_coverage.run([*base, "--json", "--output-path", str(report)], environment)
    unit_coverage.run([*base, "--html", "--output-dir", str(html)], environment)
    unit_coverage.run([*base, "--summary-only"], environment)
    unit_coverage.enforce_package(report, suite)


def main() -> None:
    """Collect the named package using the common inventory and acceptance rules."""
    suites = {suite.name: suite for suite in unit_coverage.suites()}
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=suites)
    arguments = parser.parse_args()
    collect(suites[arguments.suite])


if __name__ == "__main__":
    main()
