#!/usr/bin/env -S uv run
"""Collect application and child-process coverage, then enforce both acceptance gates."""

import os
import pathlib
import shlex
import subprocess
import sys

import unit_coverage

ROOT = pathlib.Path(__file__).resolve().parents[1]


def run(arguments: list[str], environment: dict[str, str]) -> str:
    """Run one measured phase, retaining its output and exit status."""
    return (
        subprocess.run(
            arguments,
            cwd=ROOT,
            env=environment,
            check=True,
            text=True,
            timeout=1200,
        ).stdout
        or ""
    )


def enforce(report: pathlib.Path) -> None:
    """Fail on incomplete inventories or uncovered application behaviour."""
    application_suite = next(suite for suite in unit_coverage.suites() if suite.package == "git-pr")
    unit_coverage.enforce_package(report, application_suite)


def main() -> None:
    """Use cargo-llvm-cov's external-test environment for every Rust process."""
    environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "target/coverage"))
    environment.setdefault("RUST_TEST_THREADS", "6")
    output = subprocess.check_output(
        ["cargo", "llvm-cov", "show-env"],
        cwd=ROOT,
        env=environment,
        text=True,
        timeout=30,
    )
    for assignment in output.splitlines():
        name, value = assignment.split("=", 1)
        environment[name] = shlex.split(value)[0]
    profile = environment["LLVM_PROFILE_FILE"]
    if "%p" not in profile or "%" not in profile:
        raise AssertionError("Child-process profiles must have collision-safe filenames")
    run(["cargo", "llvm-cov", "clean", "--workspace"], environment)
    run(
        [
            sys.executable,
            "scripts/measure.py",
            "coverage-tests",
            "cargo",
            "test",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ],
        environment,
    )
    if sys.platform == "linux":
        run([sys.executable, "scripts/secret_service.py"], environment)
    directory = ROOT / "coverage"
    directory.mkdir(exist_ok=True)
    suite = next(suite for suite in unit_coverage.suites() if suite.package == "git-pr")
    base = [
        "cargo",
        "llvm-cov",
        "report",
        "-p",
        "git-pr",
        "--ignore-filename-regex",
        unit_coverage.exclusion_pattern(suite, unit_only=False),
    ]
    run([*base, "--json", "--output-path", str(directory / "coverage.json")], environment)
    run([*base, "--html", "--output-dir", str(directory)], environment)
    with (directory / "summary.txt").open("w") as summary:
        subprocess.run(base, cwd=ROOT, env=environment, check=True, stdout=summary, timeout=120)
    enforce(directory / "coverage.json")


if __name__ == "__main__":
    main()
