#!/usr/bin/env -S uv run
"""Require stable replay to reject Clippy warnings without changing its caller."""

import argparse
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest
import unittest.mock

import fuzzing


class ReplayWarnings(unittest.TestCase):
    def test_cached_clippy_warnings_fail_replay_and_preserve_the_environment(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            workspace = root / "fuzz"
            source = workspace / "src"
            (source / "bin").mkdir(parents=True)
            (workspace / "Cargo.toml").write_text(
                '[package]\nname = "replay-warning-fixture"\nversion = "0.0.0"\n'
                'edition = "2024"\n[workspace]\n'
            )
            (source / "lib.rs").write_text(
                "pub fn redundant(value: bool) -> bool {\n"
                "    if value {\n        true\n    } else {\n        false\n    }\n}\n"
            )
            (source / "bin/replay.rs").write_text("fn main() {}\n")
            shutil.copyfile(fuzzing.ROOT / "rust-toolchain.toml", root / "rust-toolchain.toml")
            environment = {
                key: value
                for key, value in os.environ.items()
                if not key.startswith(("GIT_", "GH_", "GITHUB_", "CARGO_REGISTRIES_"))
                and key not in ("CARGO_REGISTRY_TOKEN", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS")
            } | {
                "CARGO_HOME": str(root / "cargo-home"),
                "CARGO_BUILD_WARNINGS": "allow",
                "CARGO_NET_OFFLINE": "true",
            }
            subprocess.run(
                ["cargo", "fmt", "--manifest-path", "fuzz/Cargo.toml"],
                cwd=root,
                env=environment,
                check=True,
                capture_output=True,
                timeout=60,
            )
            subprocess.run(
                ["cargo", "generate-lockfile", "--manifest-path", "fuzz/Cargo.toml"],
                cwd=root,
                env=environment,
                check=True,
                capture_output=True,
                timeout=60,
            )
            subprocess.run(
                [
                    "cargo",
                    "clippy",
                    "--manifest-path",
                    "fuzz/Cargo.toml",
                    "--lib",
                    "--bin",
                    "replay",
                    "--locked",
                    "--target-dir",
                    "fuzz/target",
                ],
                cwd=root,
                env=environment,
                check=True,
                capture_output=True,
                timeout=60,
            )
            output = root / "evidence"
            report: dict[str, object] = {}
            args = argparse.Namespace(output=output, seed=20261015, cases=1)
            with (
                unittest.mock.patch.object(fuzzing, "ROOT", root),
                unittest.mock.patch.dict(os.environ, environment, clear=True),
            ):
                self.assertFalse(fuzzing.replay(args, report))
                self.assertEqual(os.environ["CARGO_BUILD_WARNINGS"], "allow")
            self.assertEqual(report["build_status"], 0)
            self.assertEqual(report["format_status"], 0, (output / "format.log").read_text())
            self.assertEqual(report["clippy_status"], 101, (output / "clippy.log").read_text())
            self.assertIn("needless_bool", (output / "clippy.log").read_text())
            self.assertNotIn("targets", report)


if __name__ == "__main__":
    unittest.main()
