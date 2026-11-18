#!/usr/bin/env -S uv run
"""Verify fresh live output and the mutation recipe's failure and cleanup effects."""

import json
import os
import pathlib
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

import mutation_progress

ROOT = pathlib.Path(__file__).resolve().parents[1]


class CampaignOutput(unittest.TestCase):
    def test_cached_output_is_not_replayed_and_replaced_or_truncated_logs_are_followed(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            log = root / "campaign.log"
            log.write_bytes(b"cached output\n")
            follower = mutation_progress.LogFollower(log)
            self.assertEqual(follower.read(), b"")
            log.rename(root / "previous.log")
            self.assertEqual(follower.read(), b"")
            log.write_bytes(b"fresh output\n")
            self.assertEqual(follower.read(), b"fresh output\n")
            with log.open("ab") as stream:
                stream.write(b"\xe2\x98\x83\n")
            self.assertEqual(follower.read(), b"\xe2\x98\x83\n")
            log.write_bytes(b"")
            self.assertEqual(follower.read(), b"")
            log.write_bytes(b"new output\n")
            self.assertEqual(follower.read(), b"new output\n")
            self.assertEqual((root / "previous.log").read_bytes(), b"cached output\n")

    def test_heartbeat_uses_only_fresh_complete_outcomes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            inventory, outcomes = [root / name for name in ("mutants.json", "outcomes.json")]
            inventory.write_text("[1,2]")
            outcomes.write_text('{"outcomes":[{"scenario":{}},{"scenario":{}}]}')
            initial = tuple(mutation_progress.snapshot(path) for path in (inventory, outcomes))
            self.assertEqual(mutation_progress.completed(root, initial), "running")
            inventory.write_text("[1,2,3]")
            outcomes.write_text("{")
            self.assertEqual(mutation_progress.completed(root, initial), "running")
            outcomes.write_text(
                json.dumps(
                    {
                        "outcomes": [
                            {"scenario": "Baseline"},
                            {"scenario": {"Mutant": {}}},
                        ]
                    }
                )
            )
            self.assertEqual(mutation_progress.completed(root, initial), "1/3 mutations completed")


class MutationRecipe(unittest.TestCase):
    def test_recipe_streams_before_completion_preserves_failures_and_reaps_the_follower(
        self,
    ) -> None:
        for status in (0, 7):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                (root / "scripts").mkdir()
                (root / "bin").mkdir()
                (root / "tmp").mkdir()
                shutil.copyfile(
                    ROOT / "scripts/mutation_progress.py", root / "scripts/mutation_progress.py"
                )
                log = root / "target/validation/assurance/mutants/campaign.log"
                log.parent.mkdir(parents=True)
                log.write_text("CACHED_CONTENT_MUST_NOT_APPEAR\n")
                producer = root / "producer.py"
                producer.write_text(
                    "import pathlib,time\n"
                    f"root=pathlib.Path({str(root)!r})\n"
                    f"log=pathlib.Path({str(log)!r})\n"
                    "log.rename(log.with_name('previous.log'))\n"
                    "log.write_text('LIVE_CAMPAIGN_OUTPUT\\n')\n"
                    "while not (root/'release').exists(): time.sleep(0.01)\n"
                    f"raise SystemExit({status})\n"
                )
                uv = root / "bin/uv"
                uv.write_text(
                    "#!/bin/sh\n"
                    f"printf '%s\\n' \"$$\" > {shlex.quote(str(root / 'follower.pid'))}\n"
                    f'shift 2\nexec {shlex.quote(sys.executable)} "$@"\n'
                )
                uv.chmod(0o755)
                prek = root / "bin/prek"
                prek.write_text(
                    f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(producer))}\n"
                )
                prek.chmod(0o755)
                environment = {
                    key: value
                    for key, value in os.environ.items()
                    if not key.startswith(("GIT_", "GH_", "GITHUB_"))
                }
                environment |= {
                    "HOME": str(root),
                    "TMPDIR": str(root / "tmp"),
                    "PATH": str(root / "bin") + os.pathsep + os.environ["PATH"],
                }
                command = [
                    "just",
                    "--justfile",
                    str(ROOT / "justfile"),
                    "--working-directory",
                    str(root),
                    "mutants",
                    "",
                ]
                with subprocess.Popen(
                    command,
                    env=environment,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                ) as process:
                    assert process.stdout is not None
                    output = bytearray()
                    try:
                        deadline = time.monotonic() + 10
                        while b"LIVE_CAMPAIGN_OUTPUT" not in output and time.monotonic() < deadline:
                            if select.select([process.stdout], [], [], 0.05)[0]:
                                data = os.read(process.stdout.fileno(), 4096)
                                if not data:
                                    break
                                output.extend(data)
                        self.assertIn(b"LIVE_CAMPAIGN_OUTPUT", output)
                        self.assertIsNone(
                            process.poll(), "progress must arrive before producer completion"
                        )
                        (root / "release").touch()
                        remaining, _ = process.communicate(timeout=5)
                        output.extend(remaining)
                        self.assertEqual(process.returncode == 0, status == 0, output.decode())
                        self.assertNotIn(b"CACHED_CONTENT_MUST_NOT_APPEAR", output)
                        self.assertEqual(list((root / "tmp").iterdir()), [])
                        pid = int((root / "follower.pid").read_text())
                        with self.assertRaises(ProcessLookupError):
                            os.kill(pid, 0)
                    finally:
                        if process.poll() is None:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()


if __name__ == "__main__":
    unittest.main()
