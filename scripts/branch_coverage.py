#!/usr/bin/env -S uv run
"""Require complete nightly unit branch coverage in isolated profile storage."""

import json
import os
import pathlib
import subprocess

import assurance
import unit_coverage

NIGHTLY = "nightly-2026-10-14"


def main() -> None:
    """Retain branch evidence and require every measured outcome to be covered."""
    environment = dict(os.environ)
    environment["RUSTUP_TOOLCHAIN"] = NIGHTLY
    environment["CARGO_LLVM_COV_TARGET_DIR"] = str(assurance.ROOT / "target/branch-coverage")
    environment["CARGO_LLVM_COV_BUILD_DIR"] = environment["CARGO_LLVM_COV_TARGET_DIR"]
    environment.pop("LLVM_PROFILE_FILE", None)
    assurance.archive("branches")
    output = assurance.EVIDENCE / "branches"
    output.mkdir(parents=True, exist_ok=True)
    report = {
        "toolchain": NIGHTLY,
        "scope": "application policy, unit execution only; complete branch outcomes required",
        "commands": [],
        "passed": False,
    }
    suite = next(suite for suite in unit_coverage.suites() if suite.package == "git-pr")
    cargo = ["rustup", "run", NIGHTLY, "cargo"]
    base = [
        *cargo,
        "llvm-cov",
        "report",
        "-p",
        "git-pr",
        "--ignore-filename-regex",
        unit_coverage.exclusion_pattern(suite, unit_only=True),
    ]
    commands = (
        [*cargo, "llvm-cov", "clean", "--workspace"],
        [*cargo, "llvm-cov", "--branch", "--no-report", *unit_coverage.test_arguments(suite)],
        [*base, "--json", "--output-path", str(output / "coverage.json")],
        [*base, "--html", "--output-dir", str(output / "html")],
    )
    try:
        nightly_cargo = pathlib.Path(
            subprocess.check_output(
                ["rustup", "which", "--toolchain", NIGHTLY, "cargo"], text=True
            ).strip()
        )
        environment["PATH"] = str(nightly_cargo.parent) + os.pathsep + environment["PATH"]
        environment["RUSTC"] = str(nightly_cargo.with_name("rustc"))
        report["compiler"] = subprocess.check_output(
            ["rustup", "run", NIGHTLY, "rustc", "-Vv"], env=environment, text=True
        ).strip()
        for index, command in enumerate(commands):
            result = assurance.execute(command, output / f"{index}.log", environment=environment)
            report["commands"].append(result)
            assurance.require_success(result)
        data = json.loads((output / "coverage.json").read_text())
        report["totals"] = data["data"][0]["totals"]
        unit_coverage.enforce_branches(report["totals"])
        report["passed"] = True
    finally:
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
