#!/usr/bin/env -S uv run
"""Execute generated contracts, known-defect sensitivity and critical mutations."""

import argparse
import dataclasses
import datetime
import hashlib
import json
import math
import os
import pathlib
import re
import shutil
import signal
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "target/validation/assurance"
MUTANTS_VERSION = "27.1.0"
DEFAULT_SEED = "5138697615354058066"


@dataclasses.dataclass(frozen=True)
class Change:
    """An exact, reviewed reintroduction of a previously corrected decision."""

    path: str
    original: str
    replacement: str
    occurrences: int = 1


@dataclasses.dataclass(frozen=True)
class Defect:
    """A defective decision and its independently asserted regression."""

    name: str
    test: str
    changes: tuple[Change, ...]


DEFECTS = (
    Defect(
        "fetch-instead-of-push-destination",
        "tests::assurance::every_partial_push_destination_prefix_remains_uncertain_until_all_destinations_match",
        (
            Change(
                "src/git/mod.rs",
                '"--push",\n                        "--all",\n                        "--",',
                '"--",',
            ),
        ),
    ),
    Defect(
        "single-character-remote-ref",
        "tests::regressions::existing_single_character_remote_ref_is_published_without_rewriting",
        (
            Change(
                "src/model/trailers.rs",
                "if value.is_empty() {",
                'if value.is_empty() || (key.eq_ignore_ascii_case("remote-ref") && value.chars().count() < 2) {',
            ),
        ),
    ),
    Defect(
        "pipe-containing-ref-inventory",
        "tests::regressions::pipe_branch_is_synchronised_without_moving_the_checked_out_branch",
        (Change("src/git/mod.rs", ".rsplit_once('|')", ".split_once('|')"),),
    ),
    Defect(
        "single-character-trailer",
        "tests::regressions::single_character_tags_preserve_the_published_branch_and_label",
        (
            Change(
                "src/model/trailers.rs",
                "if value.is_empty() {",
                'if value.chars().count() < 2 && !key.eq_ignore_ascii_case("remote-ref") {',
            ),
        ),
    ),
    Defect(
        "ambiguous-short-trunk",
        "tests::regressions::trunk_discovery_ignores_shadowing_local_branches_and_tags",
        (
            Change(
                "src/config/mod.rs",
                '"symbolic-ref",\n            &format!("refs/remotes/{}/HEAD", repo.remote),',
                '"symbolic-ref",\n            "--short",\n            &format!("refs/remotes/{}/HEAD", repo.remote),',
            ),
            Change(
                "src/config/mod.rs",
                '.strip_prefix(&format!("refs/remotes/{}/", repo.remote))',
                '.strip_prefix(&format!("{}/", repo.remote))',
            ),
        ),
    ),
    Defect(
        "duplicate-trailer-precedence",
        "tests::unit::properties::duplicate_trailer_winners_survive_rewriting",
        (
            Change(
                "src/model/trailers.rs",
                "attrs.push((key, value));",
                "attrs.insert(0, (key, value));",
            ),
        ),
    ),
)


def execute(
    command: list[str],
    log: pathlib.Path,
    *,
    cwd: pathlib.Path = ROOT,
    environment: dict[str, str] | None = None,
    timeout: int = 1200,
) -> dict:
    """Retain bounded process evidence without interpreting a timeout as success."""
    started = time.monotonic()
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w") as stream:
        with subprocess.Popen(
            command,
            cwd=cwd,
            env=environment,
            stdout=stream,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        ) as process:
            try:
                status: int | str = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
                finally:
                    # The direct child may exit before a descendant which
                    # ignored SIGTERM; its process group still needs cleanup.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                status = "timeout"
    result = {
        "command": command,
        "status": status,
        "seconds": time.monotonic() - started,
        "log": str(log.relative_to(ROOT)),
    }
    print(json.dumps(result), flush=True)
    return result


def require_success(result: dict) -> None:
    """Keep failed or inconclusive commands visible to the calling gate."""
    if result["status"] != 0:
        raise AssertionError(f"command failed: {result}; inspect its retained log")


def properties(report: dict) -> None:
    """Run ordinary and externally expanded cases with a recorded input seed."""
    environment = os.environ.copy()
    environment.setdefault("PROPTEST_CASES", "128")
    environment.setdefault("PROPTEST_RNG_SEED", DEFAULT_SEED)
    environment.setdefault("GIT_PR_ASSURANCE_CASES", "8")
    for key in ("PROPTEST_CASES", "GIT_PR_ASSURANCE_CASES"):
        if int(environment[key]) <= 0:
            raise ValueError(f"{key} must be positive")
    report["inputs"] = {
        key: environment[key]
        for key in ("PROPTEST_CASES", "PROPTEST_RNG_SEED", "GIT_PR_ASSURANCE_CASES")
    }
    for name, test_filter in (
        ("properties", "tests::unit::properties::"),
        ("assurance", "::assurance::"),
    ):
        result = execute(
            ["cargo", "test", "--locked", "-p", "git-pr", "--lib", "--", test_filter],
            EVIDENCE / f"{name}.log",
            environment=environment,
        )
        report["commands"].append(result)
        require_success(result)
        if "running 0 tests" in (ROOT / result["log"]).read_text():
            raise AssertionError(f"empty behavioural suite: {test_filter}")


def snapshot(destination: pathlib.Path) -> dict[str, str]:
    """Copy explicit build inputs without developer Git state or unrelated files."""
    files = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT
    ).split(b"\0")
    digests = {}
    for name in sorted(set(os.fsdecode(name) for name in files if name)):
        relative = pathlib.PurePosixPath(name)
        if relative.is_absolute() or ".." in relative.parts or relative.parts[0] == ".git":
            raise AssertionError(f"unsafe snapshot path: {name}")
        if not (
            name
            in {
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                "build.rs",
                "src/cli/usage.txt",
                "src/cli/flags.txt",
                "src/commands/editor.sh",
            }
            or relative.parts[0] == "src"
            and relative.suffix == ".rs"
        ):
            continue
        source = ROOT / name
        if not source.resolve().is_relative_to(ROOT.resolve()) or any(
            ROOT.joinpath(*relative.parts[:index]).is_symlink()
            for index in range(1, len(relative.parts) + 1)
        ):
            raise AssertionError(f"snapshot requires an ordinary source file: {name}")
        if not source.exists():
            continue
        if not source.is_file():
            raise AssertionError(f"snapshot requires an ordinary source file: {name}")
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        digests[name] = hashlib.sha256(target.read_bytes()).hexdigest()
    return digests


def archive(gate: str) -> None:
    """Retain previous evidence before starting another campaign of the same kind."""
    names = {
        "properties": ("properties.json", "properties.log", "assurance.log"),
        "sensitivity": ("sensitivity.json", "sensitivity"),
        "mutants": ("mutants.json", "mutants"),
    }[gate]
    previous = [EVIDENCE / name for name in names if (EVIDENCE / name).exists()]
    if not previous:
        return
    history = ROOT / "target/validation/history/assurance"
    history.mkdir(parents=True, exist_ok=True)
    destination = pathlib.Path(tempfile.mkdtemp(prefix=f"{gate}-", dir=history))
    for path in previous:
        shutil.move(path, destination / path.name)


def source_identity() -> dict[str, str]:
    """Record the source and scope used by each campaign before execution."""
    paths = [*ROOT.glob("src/**/*.rs"), ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    paths.extend([ROOT / ".cargo/mutants.toml", pathlib.Path(__file__).resolve()])
    return {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(paths)
    }


def apply_changes(directory: pathlib.Path, changes: tuple[Change, ...]) -> dict[str, str]:
    """Reject stale mutation anchors instead of silently testing an unchanged tree."""
    original = {}
    for change in changes:
        path = directory / change.path
        text = path.read_text()
        original.setdefault(change.path, text)
        if text.count(change.original) != change.occurrences:
            raise AssertionError(f"stale sensitivity anchor: {change}")
        path.write_text(text.replace(change.original, change.replacement))
    return original


def detected(status: int | str, output: str, test: str) -> bool:
    """Require an executed assertion failure, rather than a compiler or runner failure."""
    return (
        status == 101 and f"test {test} ... FAILED" in output and "test result: FAILED." in output
    )


def validate_selectors(output: str) -> None:
    """Reject absent or ambiguous exact regressions against Cargo's compiled inventory."""
    tests = [line.removesuffix(": test") for line in output.splitlines() if line.endswith(": test")]
    selectors = [defect.test for defect in DEFECTS]
    if len(selectors) != len(set(selectors)):
        raise AssertionError("duplicate sensitivity selectors")
    for selector in selectors:
        if tests.count(selector) != 1:
            raise AssertionError(f"sensitivity selector must identify exactly one test: {selector}")


def sensitivity(report: dict) -> None:
    """Require a passing baseline and actual assertion failure for each historical defect."""
    directory = EVIDENCE / "sensitivity"
    directory.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(ROOT / "target/assurance-sensitivity")
    environment["CARGO_INCREMENTAL"] = "0"
    environment["GIT_CEILING_DIRECTORIES"] = str(directory.resolve())
    with tempfile.TemporaryDirectory(prefix="source-", dir=directory) as temporary:
        source = pathlib.Path(temporary)
        report["source_sha256"] = snapshot(source)
        report["defects"] = []
        inventory = execute(
            ["cargo", "test", "--locked", "-p", "git-pr", "--lib", "--", "--list"],
            directory / "inventory.log",
            cwd=source,
            environment=environment,
        )
        report["commands"].append(inventory)
        require_success(inventory)
        validate_selectors((ROOT / inventory["log"]).read_text())
        for defect in DEFECTS:
            command = [
                "cargo",
                "test",
                "--locked",
                "-p",
                "git-pr",
                "--lib",
                "--",
                defect.test,
                "--exact",
                "--nocapture",
            ]
            baseline = execute(
                command,
                directory / f"{defect.name}-baseline.log",
                cwd=source,
                environment=environment,
            )
            report["commands"].append(baseline)
            require_success(baseline)
            baseline_text = (ROOT / baseline["log"]).read_text()
            if f"test {defect.test} ... ok" not in baseline_text:
                raise AssertionError(f"baseline did not execute its regression: {defect.test}")
            originals = apply_changes(source, defect.changes)
            try:
                mutant = execute(
                    command,
                    directory / f"{defect.name}-mutant.log",
                    cwd=source,
                    environment=environment,
                )
                report["commands"].append(mutant)
                output = (ROOT / mutant["log"]).read_text()
                caught = detected(mutant["status"], output, defect.test)
                report["defects"].append(
                    {
                        "name": defect.name,
                        "test": defect.test,
                        "caught": caught,
                        "changes": [dataclasses.asdict(change) for change in defect.changes],
                    }
                )
                if not caught:
                    raise AssertionError(
                        f"defect was not detected by its assertions: {defect.name}"
                    )
            finally:
                for filename, text in originals.items():
                    (source / filename).write_text(text)


def mutation_identity(value: object) -> str:
    """Compare complete tool identities, excluding the inventory-only rendered diff."""
    fields = ("name", "package", "file", "function", "span", "replacement", "genre")
    if not isinstance(value, dict) or not set(fields).issubset(value):
        raise AssertionError("malformed mutation identity")
    if any(
        not isinstance(value[key], str) or not value[key] for key in ("name", "package", "file")
    ):
        raise AssertionError("malformed mutation name, package or source path")
    source = pathlib.PurePosixPath(value["file"])
    if not source.parts or source.is_absolute() or ".." in source.parts:
        raise AssertionError("unsafe mutation source path")
    span = value["span"]
    if not isinstance(span, dict) or set(span) != {"start", "end"}:
        raise AssertionError("malformed mutation source span")
    positions = []
    for edge in ("start", "end"):
        position = span[edge]
        if not isinstance(position, dict) or set(position) != {"line", "column"}:
            raise AssertionError("malformed mutation source position")
        if any(type(position[key]) is not int or position[key] <= 0 for key in position):
            raise AssertionError("malformed mutation source position")
        positions.append((position["line"], position["column"]))
    if positions[1] < positions[0] or not isinstance(value["replacement"], str):
        raise AssertionError("malformed mutation replacement or source span")
    function = value["function"]
    if function is not None and (
        not isinstance(function, dict)
        or not isinstance(function.get("function_name"), str)
        or not function["function_name"]
        or not isinstance(function.get("return_type"), str)
        or not isinstance(function.get("span"), dict)
    ):
        raise AssertionError("malformed mutation function")
    if not isinstance(value["genre"], str) or value["genre"] not in {
        "FnValue",
        "BinaryOperator",
        "UnaryOperator",
        "MatchArm",
        "MatchArmGuard",
        "StructField",
    }:
        raise AssertionError("unknown mutation operator")
    return json.dumps({key: value[key] for key in fields}, sort_keys=True)


def mutation_log(directory: pathlib.Path, outcome: dict) -> str:
    """Reject escaped or indirect log paths before inspecting execution evidence."""
    name = outcome.get("log_path")
    if not isinstance(name, str) or not name:
        raise AssertionError("missing mutation log path")
    relative = pathlib.PurePosixPath(name)
    path = directory / name
    if (
        relative.is_absolute()
        or ".." in relative.parts
        or not path.resolve().is_relative_to(directory.resolve())
        or any(
            directory.joinpath(*relative.parts[:index]).is_symlink()
            for index in range(1, len(relative.parts) + 1)
        )
        or not path.is_file()
    ):
        raise AssertionError(f"unsafe or missing mutation log: {name}")
    return re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", path.read_text())


def validate_mutation_phases(outcome: dict, directory: pathlib.Path) -> None:
    """Require actual build and assertion evidence rather than trusting summary labels."""
    phases = outcome.get("phase_results")
    if not isinstance(phases, list) or not all(isinstance(phase, dict) for phase in phases):
        raise AssertionError("missing mutation phase evidence")
    unviable = outcome["summary"] == "Unviable"
    if [phase.get("phase") for phase in phases] != (["Build"] if unviable else ["Build", "Test"]):
        raise AssertionError("incomplete or unexpected mutation phases")
    for phase in phases:
        duration = phase.get("duration")
        if type(duration) not in (int, float) or not math.isfinite(duration) or duration < 0:
            raise AssertionError("invalid mutation phase duration")
        argv = phase.get("argv")
        if not isinstance(argv, list) or not argv or not all(isinstance(arg, str) for arg in argv):
            raise AssertionError("missing mutation phase command")
        if (
            len(argv) < 2
            or pathlib.Path(argv[0]).name != "cargo"
            or argv[1] != "test"
            or ("--no-run" in argv) != (phase["phase"] == "Build")
        ):
            raise AssertionError("unexpected mutation phase command")
    if unviable:
        status = phases[0].get("process_status")
        if (
            not isinstance(status, dict)
            or set(status) != {"Failure"}
            or type(status["Failure"]) is not int
            or status["Failure"] == 0
        ):
            raise AssertionError("unviable mutant did not have a failed build")
        mutation_log(directory, outcome)
        return
    if phases[0].get("process_status") != "Success":
        raise AssertionError("mutation build did not succeed")
    log = mutation_log(directory, outcome)
    if outcome["scenario"] == "Baseline":
        if phases[1].get("process_status") != "Success" or not re.search(
            r"(?m)^test result: ok\. [1-9]\d* passed; 0 failed;", log
        ):
            raise AssertionError("mutation baseline did not execute a nonempty passing suite")
    elif (
        phases[1].get("process_status") != {"Failure": 101}
        or not re.search(r"(?m)^test result: FAILED\. \d+ passed; [1-9]\d* failed;", log)
        or not re.search(r"(?m)^test [^\r\n]+ \.\.\. FAILED\r?$", log)
    ):
        raise AssertionError("caught mutant lacks executed assertion failure evidence")


def validate_mutants(data: dict, inventory: list, directory: pathlib.Path) -> dict:
    """Require a complete inventory, passing baseline and explained executable outcomes."""
    if not isinstance(data, dict) or data.get("cargo_mutants_version") != MUTANTS_VERSION:
        raise AssertionError("missing or incorrect mutation report version")
    try:
        started = datetime.datetime.fromisoformat(data["start_time"])
        ended = datetime.datetime.fromisoformat(data["end_time"])
    except (KeyError, TypeError, ValueError) as error:
        raise AssertionError("missing or invalid mutation completion times") from error
    if started.tzinfo is None or ended.tzinfo is None or ended < started:
        raise AssertionError("invalid mutation completion times")
    if not isinstance(inventory, list) or not inventory:
        raise AssertionError("empty mutation scope")
    planned = [mutation_identity(mutant) for mutant in inventory]
    if len(set(planned)) != len(planned):
        raise AssertionError("duplicate mutation inventory identities")
    outcomes = data.get("outcomes")
    if not isinstance(outcomes, list) or not all(isinstance(outcome, dict) for outcome in outcomes):
        raise AssertionError("malformed mutation outcomes")
    baseline = [outcome for outcome in outcomes if outcome.get("scenario") == "Baseline"]
    if (
        len(baseline) != 1
        or outcomes[0] is not baseline[0]
        or baseline[0].get("summary") != "Success"
    ):
        raise AssertionError("missing, duplicated or failing mutation baseline")
    mutants = [outcome for outcome in outcomes if outcome.get("scenario") != "Baseline"]
    completed = []
    for outcome in mutants:
        scenario = outcome.get("scenario")
        if not isinstance(scenario, dict) or set(scenario) != {"Mutant"}:
            raise AssertionError("malformed mutation scenario")
        completed.append(mutation_identity(scenario["Mutant"]))
    if len(set(completed)) != len(completed) or set(completed) != set(planned):
        raise AssertionError("mutation outcomes do not complete the exact planned inventory")
    for key, expected in {
        "total_mutants": len(mutants),
        "caught": sum(outcome.get("summary") == "CaughtMutant" for outcome in mutants),
        "unviable": sum(outcome.get("summary") == "Unviable" for outcome in mutants),
        "missed": sum(outcome.get("summary") == "MissedMutant" for outcome in mutants),
        "timeout": sum(outcome.get("summary") == "Timeout" for outcome in mutants),
        "success": sum(outcome.get("summary") == "Success" for outcome in mutants),
    }.items():
        if type(data.get(key)) is not int or data[key] != expected:
            raise AssertionError(f"inconsistent mutation aggregate: {key}")
    unresolved = [
        outcome for outcome in mutants if outcome.get("summary") not in ("CaughtMutant", "Unviable")
    ]
    if unresolved:
        raise AssertionError(f"unresolved mutation outcomes: {unresolved}")
    for outcome in outcomes:
        validate_mutation_phases(outcome, directory)
    if not data["caught"]:
        raise AssertionError("mutation campaign detected no executable mutations")
    return {
        "planned": len(planned),
        "examined": len(mutants),
        "caught": data["caught"],
        "unviable": data["unviable"],
        "unresolved": [],
    }


def mutants(report: dict) -> None:
    """Execute the declared critical scope, optionally intersected with a Git diff."""
    version = subprocess.check_output(["cargo", "mutants", "--version"], text=True).strip()
    if version != f"cargo-mutants {MUTANTS_VERSION}":
        raise AssertionError(f"unexpected mutation tool: {version}")
    report["tool"] = version
    output = EVIDENCE / "mutants"
    output.mkdir(parents=True, exist_ok=True)
    command = ["cargo", "mutants", "--no-shuffle", "--jobs", "2", "--output", str(output)]
    resolved = ""
    base = os.environ.get("GIT_PR_MUTATION_BASE", "")
    if base:
        resolved = subprocess.check_output(
            ["git", "rev-parse", "--verify", f"{base}^{{commit}}"], cwd=ROOT, text=True
        ).strip()
        patch = output / "changes.diff"
        with patch.open("w") as stream:
            subprocess.run(
                ["git", "diff", "--no-ext-diff", resolved, "--", "src"],
                cwd=ROOT,
                stdout=stream,
                check=True,
            )
        command.extend(["--in-diff", str(patch)])
        report["diff_base"] = resolved
    result = execute(command, output / "campaign.log", timeout=5400)
    report["commands"].append(result)
    directory = output / "mutants.out"
    inventory_path = directory / "mutants.json"
    if not inventory_path.is_file():
        raise AssertionError(f"mutation campaign produced no inventory: {result}")
    inventory = json.loads(inventory_path.read_text())
    if not isinstance(inventory, list):
        raise AssertionError("malformed mutation inventory")
    for mutant in inventory:
        mutation_identity(mutant)
    report["scope"] = {
        "inventory": str(inventory_path.relative_to(ROOT)),
        "inventory_sha256": hashlib.sha256(inventory_path.read_bytes()).hexdigest(),
        "planned_mutations": len(inventory),
        "changed_lines_only": bool(base),
        "files": sorted({mutant["file"] for mutant in inventory}),
        "functions": sorted(
            {
                mutant["function"]["function_name"]
                for mutant in inventory
                if mutant["function"] is not None
            }
        ),
        "operators": sorted({mutant["genre"] for mutant in inventory}),
    }
    outcomes = directory / "outcomes.json"
    # The pinned tool writes no outcomes file when an in-diff inventory is empty.
    if base and not inventory:
        require_success(result)
        if (
            outcomes.exists()
            or "No mutants found under the active filters"
            not in (output / "campaign.log").read_text()
        ):
            raise AssertionError("empty changed scope lacks fresh nonexecution evidence")
        report["mutation"] = {
            "status": "no changed mutations",
            "diff_base": resolved,
            "planned": 0,
            "examined": 0,
        }
        return
    if not outcomes.is_file():
        raise AssertionError(f"mutation campaign produced no outcomes: {result}")
    data = json.loads(outcomes.read_text())
    report["mutation"] = validate_mutants(data, inventory, directory)
    require_success(result)


def main() -> None:
    """Persist failure evidence even when a gate cannot complete."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gate", choices=("properties", "sensitivity", "mutants"))
    args = parser.parse_args()
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    archive(args.gate)
    report = {
        "gate": args.gate,
        "commands": [],
        "passed": False,
        "source_sha256": source_identity(),
    }
    try:
        {"properties": properties, "sensitivity": sensitivity, "mutants": mutants}[args.gate](
            report
        )
        report["passed"] = True
    except (AssertionError, ValueError, OSError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
        raise
    finally:
        (EVIDENCE / f"{args.gate}.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
