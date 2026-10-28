#!/usr/bin/env -S uv run
"""Verify CI publication wiring and execute its guards with isolated Git repositories."""

import json
import os
import pathlib
import subprocess
import tempfile
import unittest
from typing import Any

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]


def workflow(path: pathlib.Path) -> dict[str, Any]:
    """Keep GitHub's `on` key and expressions intact without YAML 1.1 Boolean conversion."""
    return yaml.load(path.read_text(), Loader=yaml.BaseLoader)


def release_step(name: str) -> dict[str, Any]:
    """Select the actual step executed by the sole publisher."""
    steps = workflow(ROOT / ".github/workflows/ci.yml")["jobs"]["release"]["steps"]
    return next(step for step in steps if step.get("name") == name)


class WorkflowContracts(unittest.TestCase):
    def setUp(self) -> None:
        self.ci = workflow(ROOT / ".github/workflows/ci.yml")
        self.release = self.ci["jobs"]["release"]

    def test_publication_waits_for_both_complete_matrices_on_main_pushes(self) -> None:
        self.assertEqual(self.release["needs"], ["validation", "boundaries"])
        predicates = {part.strip() for part in self.release["if"].split("&&")}
        self.assertEqual(
            predicates,
            {
                "github.event_name == 'push'",
                "github.ref == 'refs/heads/main'",
                "needs.validation.result == 'success'",
                "needs.boundaries.result == 'success'",
            },
        )
        self.assertEqual(
            self.ci["jobs"]["validation"]["strategy"]["matrix"]["os"], ["ubuntu-24.04", "macos-15"]
        )
        self.assertEqual(
            self.ci["jobs"]["boundaries"]["strategy"]["matrix"]["gate"],
            ["backends", "coverage", "assurance", "mutants"],
        )
        self.assertEqual(self.ci["on"]["push"]["branches"], ["main"])
        self.assertEqual(self.release["name"], "Release")
        for job in ["validation", "boundaries"]:
            self.assertEqual(self.ci["jobs"][job]["strategy"]["fail-fast"], "false")

    def test_validation_supersession_cannot_cancel_publication_or_matrix_peers(self) -> None:
        self.assertNotIn("concurrency", self.ci)
        for job, dimension in [("validation", "os"), ("boundaries", "gate")]:
            concurrency = self.ci["jobs"][job]["concurrency"]
            self.assertEqual(concurrency["cancel-in-progress"], "true")
            for field in [
                "github.workflow",
                "github.event_name",
                "github.ref",
                f"matrix.{dimension}",
            ]:
                self.assertIn("${{ " + field + " }}", concurrency["group"])
            self.assertIn(f"-{job}-", concurrency["group"])
        # GitHub documents queue:max; actionlint 1.7.12 does not yet model this field.
        self.assertEqual(
            self.release["concurrency"],
            {"group": "release-tags", "cancel-in-progress": "false", "queue": "max"},
        )

    def test_only_the_release_job_has_write_permission_without_custom_credentials(self) -> None:
        self.assertEqual(self.ci["permissions"], {"contents": "read"})
        self.assertEqual(self.release["permissions"], {"contents": "write"})
        self.assertNotIn("secrets.", json.dumps(self.ci))
        self.assertNotIn("create-github-app-token", json.dumps(self.ci))
        for name in ["validation", "boundaries"]:
            self.assertNotIn("permissions", self.ci["jobs"][name])
            serialised = json.dumps(self.ci["jobs"][name])
            self.assertNotIn("github.token", serialised)
            checkout = next(
                step
                for step in self.ci["jobs"][name]["steps"]
                if step.get("uses", "").startswith("actions/checkout@")
            )
            self.assertEqual(checkout["with"]["persist-credentials"], "false")

    def test_validated_revision_and_step_scoped_credentials_are_explicit(self) -> None:
        checkout = next(
            step
            for step in self.release["steps"]
            if step.get("uses", "").startswith("actions/checkout@")
        )
        self.assertEqual(
            checkout["with"],
            {"persist-credentials": "false", "fetch-depth": "0", "ref": "${{ github.sha }}"},
        )
        publish = release_step("Verify and publish annotated tags")
        self.assertEqual(publish["env"]["VALIDATED_HEAD"], "${{ github.sha }}")
        self.assertEqual(publish["env"]["GH_TOKEN"], "${{ github.token }}")
        self.assertNotIn("env", self.release)
        self.assertNotIn("workflow_run", json.dumps(self.release))
        for name, job in self.ci["jobs"].items():
            for step in job["steps"]:
                if step.get("name") != publish["name"]:
                    self.assertNotIn("github.token", json.dumps(step))
            self.assertNotIn("secrets.", json.dumps(job.get("env", {})), name)
        for step in self.release["steps"]:
            if "uses" in step:
                self.assertRegex(step["uses"], r"^[\w/-]+@[0-9a-f]{40}$")

    def test_there_is_exactly_one_automatic_publisher(self) -> None:
        publishers = []
        directory = ROOT / ".github/workflows"
        for path in sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")]):
            document = workflow(path)
            for job_id, job in document["jobs"].items():
                for step in job.get("steps", []):
                    if "scripts/version.py tag-range" in step.get("run", ""):
                        self.assertNotIn("workflow_run", document["on"], str(path))
                        publishers.append((path.name, job_id))
        self.assertEqual(publishers, [("ci.yml", "release")])
        self.assertFalse((ROOT / ".github/workflows/release.yml").exists())
        self.assertIn("scripts/test_workflows.py", (ROOT / "justfile").read_text())


class ReleaseExecution(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "calls.json"
        self.environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("GIT_", "GH_", "GITHUB_", "PRE_COMMIT_"))
        }
        self.environment.update(
            HOME=str(self.root),
            XDG_CONFIG_HOME=str(self.root / "config"),
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_TERMINAL_PROMPT="0",
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
        )
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Release fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "credential.helper", "")
        self.git("commit", "--allow-empty", "-m", "chore: initialise fixture")
        self.initial = self.git("rev-parse", "HEAD")
        self.git("init", "--bare", str(self.root / "remote.git"))
        self.git("remote", "add", "origin", str(self.root / "remote.git"))
        self.git("push", "origin", "main")
        # Fake only the final tag-helper invocation; the production Git guards execute.
        self.executable(
            "uv",
            f"#!/usr/bin/env python3\nimport json,pathlib,sys\npathlib.Path({str(self.calls)!r}).write_text(json.dumps(sys.argv[1:]))\n",
        )

    def executable(self, name: str, source: str) -> None:
        path = self.bin / name
        path.write_text(source)
        path.chmod(0o755)

    def command(
        self, args: list[str], *, env: dict[str, str] | None = None, input: str = ""
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            args,
            cwd=self.repo,
            env=self.environment | (env or {}),
            input=input,
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )

    def git(self, *args: str) -> str:
        result = self.command(["git", *args])
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def publish(self, validated: str) -> subprocess.CompletedProcess[str]:
        step = release_step("Verify and publish annotated tags")
        environment = step["env"] | {"VALIDATED_HEAD": validated, "GH_TOKEN": "fixture-token-only"}
        before = (self.repo / ".git/config").read_bytes()
        result = self.command(["bash", "-euo", "pipefail", "-c", step["run"]], env=environment)
        self.assertEqual((self.repo / ".git/config").read_bytes(), before)
        self.assertEqual(self.git("tag", "--list"), "")
        return result

    def test_descendant_advance_releases_the_original_validated_revision(self) -> None:
        self.git("commit", "--allow-empty", "-m", "fix: advance main")
        newer = self.git("rev-parse", "HEAD")
        self.git("push", "origin", "main")
        result = self.publish(self.initial)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads(self.calls.read_text()),
            [
                "run",
                "--locked",
                "--no-dev",
                "scripts/version.py",
                "tag-range",
                "0" * 40,
                self.initial,
            ],
        )
        self.assertEqual(self.git("rev-parse", "HEAD"), newer)

    def test_rewritten_main_refuses_the_superseded_revision_before_publication(self) -> None:
        self.git("commit", "--allow-empty", "-m", "fix: superseded revision")
        superseded = self.git("rev-parse", "HEAD")
        self.git("push", "origin", "main")
        self.git("push", "--force", "origin", f"{self.initial}:refs/heads/main")
        result = self.publish(superseded)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no longer in origin/main", result.stderr)
        self.assertFalse(self.calls.exists())
        self.assertEqual(self.git("rev-parse", "HEAD"), superseded)

    def test_git_credential_helper_uses_only_the_step_token_without_persistence(self) -> None:
        self.executable(
            "gh",
            "#!/bin/sh\n[ \"$*\" = 'auth git-credential get' ] || exit 1\ncat >/dev/null\nprintf 'username=x-access-token\\npassword=%s\\n' \"$GH_TOKEN\"\n",
        )
        step = release_step("Verify and publish annotated tags")
        environment = step["env"] | {"GH_TOKEN": "fixture-token-only"}
        result = self.command(
            ["git", "credential", "fill"],
            env=environment,
            input="protocol=https\nhost=github.com\n\n",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("password=fixture-token-only", result.stdout)
        self.assertNotIn("fixture-token-only", (self.repo / ".git/config").read_text())


if __name__ == "__main__":
    unittest.main()
