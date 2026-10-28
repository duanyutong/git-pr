#!/usr/bin/env -S uv run
"""Verify that assurance gates reject absent, inconclusive or stale evidence."""

import copy
import os
import pathlib
import signal
import sys
import tempfile
import time
import unittest
import unittest.mock

import assurance


class SensitivitySelectorTests(unittest.TestCase):
    """Every sensitivity baseline shall execute its intended compiled regression."""

    def test_inventory_requires_each_exact_selector_once(self) -> None:
        inventory = "\n".join(f"{defect.test}: test" for defect in assurance.DEFECTS)
        assurance.validate_selectors(inventory)
        for output in [
            "",
            inventory.replace(assurance.DEFECTS[0].test, "renamed"),
            inventory + f"\n{assurance.DEFECTS[0].test}: test",
        ]:
            with self.subTest(output=output), self.assertRaisesRegex(AssertionError, "exactly one"):
                assurance.validate_selectors(output)

    def test_duplicate_selectors_are_rejected(self) -> None:
        with (
            unittest.mock.patch.object(assurance, "DEFECTS", (assurance.DEFECTS[0],) * 2),
            self.assertRaisesRegex(AssertionError, "duplicate"),
        ):
            assurance.validate_selectors(f"{assurance.DEFECTS[0].test}: test")


class MutationVerdictTests(unittest.TestCase):
    """A passing campaign requires executable detections and a passing baseline."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = pathlib.Path(self.temporary.name)

    @staticmethod
    def identity(index: int) -> dict:
        span = {"start": {"line": index + 1, "column": 1}, "end": {"line": index + 1, "column": 5}}
        return {
            "name": f"src/lib.rs:{index + 1}:1: replace decision with false",
            "package": "git-pr",
            "file": "src/lib.rs",
            "function": {"function_name": "decision", "return_type": "-> bool", "span": span},
            "span": span,
            "replacement": "false",
            "genre": "FnValue",
            "diff": "-true\n+false",
        }

    def outcomes(self, *summaries: str, baseline: str | None = "Success") -> tuple[dict, list]:
        inventory = [self.identity(index) for index in range(len(summaries))]
        values = []
        for index, summary in enumerate((baseline, *summaries)):
            if summary is None:
                continue
            path = f"logs/{index}.log"
            log = self.directory / path
            log.parent.mkdir(exist_ok=True)
            phases = [
                {
                    "phase": "Build",
                    "duration": 0.1,
                    "process_status": "Success",
                    "argv": ["cargo", "test", "--no-run"],
                }
            ]
            if summary == "Unviable":
                phases[0]["process_status"] = {"Failure": 101}
                log.write_text("error: could not compile\n")
            else:
                caught = summary == "CaughtMutant"
                phases.append(
                    {
                        "phase": "Test",
                        "duration": 0.2,
                        "process_status": {"Failure": 101} if caught else "Success",
                        "argv": ["cargo", "test", "--lib"],
                    }
                )
                log.write_text(
                    "test tests::contract ... FAILED\ntest result: FAILED. 166 passed; 1 failed; 0 ignored;\n"
                    if caught
                    else "test tests::contract ... ok\ntest result: ok. 167 passed; 0 failed; 0 ignored;\n"
                )
            scenario = (
                "Baseline"
                if index == 0
                else {
                    "Mutant": {
                        key: value for key, value in inventory[index - 1].items() if key != "diff"
                    }
                }
            )
            values.append(
                {
                    "scenario": scenario,
                    "summary": summary,
                    "phase_results": phases,
                    "log_path": path,
                }
            )
        data = {
            "cargo_mutants_version": assurance.MUTANTS_VERSION,
            "start_time": "2026-10-15T01:00:00Z",
            "end_time": "2026-10-15T01:01:00Z",
            "outcomes": values,
            "total_mutants": len(summaries),
            "caught": summaries.count("CaughtMutant"),
            "unviable": summaries.count("Unviable"),
            "missed": summaries.count("MissedMutant"),
            "timeout": summaries.count("Timeout"),
            "success": summaries.count("Success"),
        }
        return data, inventory

    def validate(self, data: dict, inventory: list) -> dict:
        return assurance.validate_mutants(data, inventory, self.directory)

    def test_keeps_build_failures_separate_from_detections(self) -> None:
        summary = self.validate(*self.outcomes("CaughtMutant", "Unviable"))
        self.assertEqual(
            summary, {"planned": 2, "examined": 2, "caught": 1, "unviable": 1, "unresolved": []}
        )
        with self.assertRaisesRegex(AssertionError, "no executable"):
            self.validate(*self.outcomes("Unviable"))

    def test_rejects_absent_or_failed_baselines_and_empty_scopes(self) -> None:
        for baseline in (None, "Failure", "Timeout"):
            with (
                self.subTest(baseline=baseline),
                self.assertRaisesRegex(AssertionError, "baseline"),
            ):
                self.validate(*self.outcomes("CaughtMutant", baseline=baseline))
        with self.assertRaisesRegex(AssertionError, "empty mutation"):
            self.validate(*self.outcomes())

    def test_rejects_survivors_timeouts_and_unrecognised_outcomes(self) -> None:
        for summary in ("MissedMutant", "Timeout", "Failure", "Success", "Unknown"):
            with (
                self.subTest(summary=summary),
                self.assertRaisesRegex(AssertionError, "unresolved"),
            ):
                self.validate(*self.outcomes("CaughtMutant", summary))

    def test_rejects_incomplete_or_inconsistent_campaign_metadata(self) -> None:
        for field, value in (
            ("cargo_mutants_version", "wrong"),
            ("end_time", None),
            ("end_time", "2026-10-14T00:00:00Z"),
            ("start_time", "2026-10-15T00:00:00"),
            ("total_mutants", 99),
            ("caught", 0),
            ("unviable", True),
        ):
            data, inventory = self.outcomes("CaughtMutant")
            data[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(AssertionError):
                self.validate(data, inventory)

    def test_requires_exact_inventory_completion_and_unique_identities(self) -> None:
        for variation in (
            "missing",
            "duplicate",
            "unplanned",
            "inventory duplicate",
            "malformed scenario",
        ):
            data, inventory = self.outcomes("CaughtMutant", "Unviable")
            if variation == "missing":
                data["outcomes"].pop()
            elif variation == "duplicate":
                data["outcomes"][2]["scenario"] = copy.deepcopy(data["outcomes"][1]["scenario"])
            elif variation == "unplanned":
                data["outcomes"][1]["scenario"]["Mutant"]["replacement"] = "unplanned"
            elif variation == "inventory duplicate":
                inventory[1] = copy.deepcopy(inventory[0])
            else:
                data["outcomes"][1]["scenario"] = "Mutant"
            with self.subTest(variation=variation), self.assertRaises(AssertionError):
                self.validate(data, inventory)

    def test_rejects_malformed_identities_before_scope_reporting(self) -> None:
        for field, value in (
            ("function", {}),
            ("genre", []),
            ("span", {}),
            ("file", "../escape.rs"),
            ("replacement", None),
            ("name", ""),
        ):
            identity = self.identity(0)
            identity[field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                assurance.mutation_identity(identity)

    def test_requires_successful_build_and_real_nonempty_assertion_execution(self) -> None:
        for variation in (
            "absent phases",
            "baseline no test",
            "zero tests",
            "baseline runner failure",
            "mutant compile failure",
            "runner exit",
            "no assertion",
            "invalid duration",
            "missing command",
            "wrong phase command",
        ):
            data, inventory = self.outcomes("CaughtMutant")
            baseline, mutant = data["outcomes"]
            if variation == "absent phases":
                mutant.pop("phase_results")
            elif variation == "baseline no test":
                baseline["phase_results"].pop()
            elif variation == "zero tests":
                (self.directory / baseline["log_path"]).write_text(
                    "test result: ok. 0 passed; 0 failed;\n"
                )
            elif variation == "baseline runner failure":
                baseline["phase_results"][1]["process_status"] = {"Failure": 1}
            elif variation == "mutant compile failure":
                mutant["phase_results"][0]["process_status"] = {"Failure": 101}
            elif variation == "runner exit":
                mutant["phase_results"][1]["process_status"] = {"Failure": 1}
            elif variation == "no assertion":
                (self.directory / mutant["log_path"]).write_text(
                    "error: test runner exhausted resources\n"
                )
            elif variation == "invalid duration":
                mutant["phase_results"][1]["duration"] = float("nan")
            elif variation == "wrong phase command":
                mutant["phase_results"][1]["argv"] = ["cargo", "test", "--no-run"]
            else:
                mutant["phase_results"][1]["argv"] = []
            with self.subTest(variation=variation), self.assertRaises(AssertionError):
                self.validate(data, inventory)

    def test_rejects_missing_escaped_and_symlinked_assertion_logs(self) -> None:
        for name in (
            "../outside.log",
            str(self.directory / "logs/1.log"),
            "missing.log",
            "linked/1.log",
        ):
            data, inventory = self.outcomes("CaughtMutant")
            link = self.directory / "linked"
            if not link.exists():
                link.symlink_to(self.directory / "logs", target_is_directory=True)
            data["outcomes"][1]["log_path"] = name
            with self.subTest(name=name), self.assertRaisesRegex(AssertionError, "mutation log"):
                self.validate(data, inventory)


class ChangedMutationScopeTests(unittest.TestCase):
    """An empty changed-line intersection is evidence of nonexecution, not a full gate."""

    def test_empty_diff_requires_fresh_successful_explicit_nonexecution_evidence(self) -> None:
        for variation in (
            "valid",
            "full scope",
            "missing notice",
            "stale outcomes",
            "failed command",
        ):
            with self.subTest(variation=variation), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                evidence = root / "target/validation/assurance"
                report: dict = {"commands": []}

                def execute(
                    _command: list,
                    log: pathlib.Path,
                    *,
                    variation: str = variation,
                    **_kwargs: object,
                ) -> dict:
                    directory = log.parent / "mutants.out"
                    directory.mkdir(parents=True)
                    (directory / "mutants.json").write_text("[]\n")
                    log.write_text(
                        ""
                        if variation == "missing notice"
                        else "No mutants found under the active filters\n"
                    )
                    if variation == "stale outcomes":
                        (directory / "outcomes.json").write_text("{}")
                    return {"status": 1 if variation == "failed command" else 0}

                with (
                    unittest.mock.patch.object(assurance, "ROOT", root),
                    unittest.mock.patch.object(assurance, "EVIDENCE", evidence),
                    unittest.mock.patch.object(assurance, "execute", side_effect=execute),
                    unittest.mock.patch.object(assurance.subprocess, "run"),
                    unittest.mock.patch.object(
                        assurance.subprocess,
                        "check_output",
                        side_effect=[f"cargo-mutants {assurance.MUTANTS_VERSION}", "a" * 40],
                    ),
                    unittest.mock.patch.dict(
                        os.environ,
                        {"GIT_PR_MUTATION_BASE": "" if variation == "full scope" else "base"},
                    ),
                ):
                    if variation == "valid":
                        assurance.mutants(report)
                        self.assertEqual(report["mutation"]["status"], "no changed mutations")
                        self.assertEqual(report["mutation"]["examined"], 0)
                        self.assertEqual(report["scope"]["planned_mutations"], 0)
                        self.assertTrue(report["scope"]["changed_lines_only"])
                    else:
                        with self.assertRaises(AssertionError):
                            assurance.mutants(report)


class SensitivityTests(unittest.TestCase):
    """A known defect must cause its exact regression assertion to fail."""

    def test_requires_assertions_from_the_named_test(self) -> None:
        output = "test tests::regression ... FAILED\ntest result: FAILED."
        self.assertTrue(assurance.detected(101, output, "tests::regression"))
        for status, text, test in (
            (0, output, "tests::regression"),
            ("timeout", output, "tests::regression"),
            (101, "error: could not compile", "tests::regression"),
            (101, "running 0 tests", "tests::regression"),
            (101, output, "tests::different"),
            (101, "test tests::regression ... FAILED", "tests::regression"),
        ):
            with self.subTest(status=status, text=text, test=test):
                self.assertFalse(assurance.detected(status, text, test))

    def test_exact_anchors_reject_renamed_or_duplicated_decisions(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            source = root / "source.rs"
            change = assurance.Change("source.rs", "before", "after")
            for text in ("missing", "before before"):
                source.write_text(text)
                with self.assertRaisesRegex(AssertionError, "stale sensitivity anchor"):
                    assurance.apply_changes(root, (change,))
                self.assertEqual(source.read_text(), text)
            source.write_text("before")
            originals = assurance.apply_changes(root, (change,))
            self.assertEqual(originals, {"source.rs": "before"})
            self.assertEqual(source.read_text(), "after")

    def test_snapshot_excludes_git_metadata_and_rejects_escaping_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            source = root / "source"
            (source / "src").mkdir(parents=True)
            (source / "src/example.rs").write_bytes(b"source\r\n")
            (source / ".env").write_text("DEVELOPER_TOKEN=must-not-copy\n")
            for asset in (
                "build.rs",
                "src/cli/usage.txt",
                "src/cli/flags.txt",
                "src/commands/editor.sh",
            ):
                (source / asset).parent.mkdir(parents=True, exist_ok=True)
                (source / asset).write_text("build input\n")
            destination = root / "copy"
            destination.mkdir()
            with (
                unittest.mock.patch.object(assurance, "ROOT", source),
                unittest.mock.patch.object(assurance.subprocess, "check_output") as listing,
            ):
                listing.return_value = b"src/example.rs\0src/example.rs\0src/deleted.rs\0.env\0build.rs\0src/cli/usage.txt\0src/cli/flags.txt\0src/commands/editor.sh\0"
                digests = assurance.snapshot(destination)
                self.assertEqual(
                    set(digests),
                    {
                        "src/example.rs",
                        "build.rs",
                        "src/cli/usage.txt",
                        "src/cli/flags.txt",
                        "src/commands/editor.sh",
                    },
                )
                self.assertEqual((destination / "src/example.rs").read_bytes(), b"source\r\n")
                self.assertFalse((destination / ".env").exists())
                self.assertFalse((destination / "src/deleted.rs").exists())
                for path in (b"../escape\0", b"/absolute\0", b".git/config\0"):
                    listing.return_value = path
                    with self.subTest(path=path), self.assertRaisesRegex(AssertionError, "unsafe"):
                        assurance.snapshot(destination)
                link = source / "src/link.rs"
                link.symlink_to(source / "src/example.rs")
                listing.return_value = b"src/link.rs\0"
                with self.assertRaisesRegex(AssertionError, "ordinary source"):
                    assurance.snapshot(destination)
                link.unlink()
                link.symlink_to(source / "src/missing.rs")
                with self.assertRaisesRegex(AssertionError, "ordinary source"):
                    assurance.snapshot(destination)
                outside = root / "outside"
                outside.mkdir()
                (outside / "secret.rs").write_text("must-not-copy")
                (source / "src/linked").symlink_to(outside, target_is_directory=True)
                listing.return_value = b"src/linked/secret.rs\0"
                with self.assertRaisesRegex(AssertionError, "ordinary source"):
                    assurance.snapshot(destination)

    def test_new_campaign_archives_old_evidence_without_changing_other_gates(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            evidence = root / "target/validation/assurance"
            evidence.mkdir(parents=True)
            for name in (
                "properties.json",
                "properties.log",
                "assurance.log",
                "terminal-screen.log",
                "mutants.json",
            ):
                (evidence / name).write_text(name)
            with (
                unittest.mock.patch.object(assurance, "ROOT", root),
                unittest.mock.patch.object(assurance, "EVIDENCE", evidence),
            ):
                assurance.archive("properties")
                assurance.archive("properties")
            archives = list((root / "target/validation/history/assurance").iterdir())
            self.assertEqual(len(archives), 1)
            self.assertEqual(
                sorted(path.name for path in archives[0].iterdir()),
                ["assurance.log", "properties.json", "properties.log", "terminal-screen.log"],
            )
            self.assertEqual((archives[0] / "properties.log").read_text(), "properties.log")
            self.assertEqual(list(evidence.iterdir()), [evidence / "mutants.json"])


class ProcessTimeoutTests(unittest.TestCase):
    """Timeout evidence must include the termination of surviving descendants."""

    @unittest.skipUnless(os.name == "posix", "process-group cleanup requires POSIX signals")
    def test_timeout_kills_descendant_which_ignores_sigterm_after_parent_exits(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            heartbeat = root / "heartbeat"
            pidfile = root / "child.pid"
            child = (
                "import os,pathlib,signal,time; "
                "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                f"pathlib.Path({str(pidfile)!r}).write_text(str(os.getpid())); "
                f"heartbeat=pathlib.Path({str(heartbeat)!r}); "
                "\nwhile True:\n heartbeat.write_text(str(time.monotonic_ns()))\n time.sleep(0.02)\n"
            )
            parent = f"import subprocess,time; subprocess.Popen([{sys.executable!r},'-c',{child!r}]); time.sleep(60)"
            try:
                with unittest.mock.patch.object(assurance, "ROOT", root):
                    result = assurance.execute(
                        [sys.executable, "-c", parent], root / "timeout.log", cwd=root, timeout=1
                    )
                self.assertEqual(result["status"], "timeout")
                self.assertTrue(pidfile.exists(), "descendant must start before the timeout")
                time.sleep(0.1)
                stopped = heartbeat.read_text()
                time.sleep(0.15)
                self.assertEqual(
                    heartbeat.read_text(), stopped, "descendant continued after timeout cleanup"
                )
            finally:
                if pidfile.exists():
                    try:
                        os.kill(int(pidfile.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass


if __name__ == "__main__":
    unittest.main()
