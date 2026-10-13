#!/usr/bin/env -S uv run
"""Require the declared VCS executables and run their real repository tests."""

import os
import pathlib
import subprocess
import tomllib

root = pathlib.Path(__file__).resolve().parents[1]
env = dict(os.environ)
config = tomllib.loads((root / "prek.toml").read_text())
hook = next(hook for repo in config["repos"] for hook in repo["hooks"] if hook["id"] == "backends")
pins = dict(
    dependency.removeprefix("cli:").rsplit(":", 1) for dependency in hook["additional_dependencies"]
)
for command, expected in [
    (["git", "branchless", "--version"], pins["git-branchless"]),
    (["jj", "version"], pins["jj-cli"]),
]:
    result = subprocess.run(command, env=env, check=True, capture_output=True, text=True)
    if expected not in result.stdout:
        raise RuntimeError(f"Expected {expected}, got {result.stdout}")
    print(result.stdout.strip())
for test in ["tests::backends", "branchless_rewriting"]:
    subprocess.run(["cargo", "test", "--locked", test], cwd=root, env=env, check=True)
