#!/usr/bin/env -S uv run
"""Exercise the common versioning contract in isolated, disposable repositories."""

import contextlib
import io
import os
import pathlib
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

import version

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/version.py"


class PolicyTests(unittest.TestCase):
    def test_conventional_messages(self) -> None:
        for message, kind in (
            ("# comment\n\nfix(parser): accept input\n", "patch"),
            ("feat: introduce command", "minor"),
            ("refactor(api)!: replace command", "major"),
            ("fix!: replace command", "major"),
            ("fix: change\n\n BREAKING CHANGE: incompatible", "major"),
            ("fix: change\nBREAKING-CHANGE: incompatible", "major"),
        ):
            with self.subTest(message=message):
                self.assertEqual(version.policy(message)[1], kind)
        for message in (
            "",
            "# comment",
            "plain message",
            "fix(): empty",
            "fix(a(b)): nested",
            "other: invalid",
        ):
            with self.subTest(message=message), self.assertRaises(ValueError):
                version.policy(message)

    def test_semantic_versions_and_overflow(self) -> None:
        original = version.parse_version("1.2.3-rc.1+build.7")
        for kind, expected in (("patch", "1.2.4"), ("minor", "1.3.0"), ("major", "2.0.0")):
            self.assertEqual(str(version.bumped(original, kind)), expected)
        for text in ("1.2", "01.2.3", "1.2.3-01", f"{2**64}.0.0"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                version.parse_version(text)
        for kind, text in (
            ("patch", f"0.0.{2**64 - 1}"),
            ("minor", f"0.{2**64 - 1}.0"),
            ("major", f"{2**64 - 1}.0.0"),
        ):
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, "overflow"):
                version.bumped(version.parse_version(text), kind)

    def test_published_annotated_and_lightweight_targets(self) -> None:
        self.assertEqual(
            version.published_targets(
                "broken line\nother\trefs/heads/main\ninvalid\trefs/tags/vnope\n"
                "object\trefs/tags/v1.2.3\ncommit\trefs/tags/v1.2.3^{}\n"
                "lightweight\trefs/tags/v2.0.0\n"
            ),
            {"commit", "lightweight"},
        )

    def test_main_reports_file_errors(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()) as error:
            self.assertEqual(version.main(["commit-msg", "/nonexistent/message"]), 1)
        self.assertIn("version:", error.getvalue())


class RepositoryTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("GIT_", "PRE_COMMIT_"))
        }
        self.environment.update(
            HOME=str(self.root),
            XDG_CONFIG_HOME=str(self.root / "config"),
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_TERMINAL_PROMPT="0",
            COVERAGE_FILE=str(ROOT / "coverage/.versioning"),
        )
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Version test")
        self.git("config", "user.email", "version@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgsign", "false")
        self.git("config", "credential.helper", "")
        self.write_versions("1.2.3")
        self.commit("chore: initialise repository")
        self.git("tag", "v1.2.3")

    def run_command(
        self, args: list[str], *, input: str = "", env: dict | None = None
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
        result = self.run_command(["git", *args])
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def cli(
        self, *args: str, success: bool = True, input: str = "", env: dict | None = None
    ) -> subprocess.CompletedProcess[str]:
        result = self.run_command([sys.executable, str(SCRIPT), *args], input=input, env=env)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def write_versions(self, manifest: str, lock: str | None = None) -> None:
        (self.repo / "Cargo.toml").write_text(
            '# retained manifest comment\n[package]\nname = "fixture-package"\n'
            f'version = "{manifest}" # retained version comment\nedition = "2024"\n'
        )
        (self.repo / "Cargo.lock").write_text(
            '# retained lock comment\nversion = 4\n\n[[package]]\nname = "other"\nversion = "9.8.7"\n'
            f'\n[[package]]\nname = "fixture-package"\nversion = "{lock or manifest}"\n'
        )

    def commit(self, message: str) -> str:
        self.git("add", ".")
        self.git("commit", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def message(self, text: str = "fix: adjust behaviour") -> str:
        path = self.root / "message"
        path.write_text(text)
        return str(path)

    def remote(self, *, baseline: bool = True) -> pathlib.Path:
        remote = self.root / "remote.git"
        self.git("init", "--bare", str(remote))
        self.git("remote", "add", "origin", str(remote))
        if baseline:
            self.git("push", "origin", "refs/tags/v1.2.3")
        return remote

    def test_generated_bump_preserves_formatting_and_requires_staging(self) -> None:
        before = {name: (self.repo / name).read_text() for name in version.MANIFESTS}
        message = self.message()
        result = self.cli("commit-msg", message, success=False)
        self.assertIn("1.2.3 -> 1.2.4", result.stderr)
        for name, text in before.items():
            self.assertEqual((self.repo / name).read_text(), text.replace('"1.2.3"', '"1.2.4"'))
            self.assertIn("1.2.3", self.git("show", f":{name}"))
        self.git("add", *version.MANIFESTS)
        self.cli("commit-msg", message)

    def test_rejects_incorrect_staged_bump_and_preserves_unstaged_edits(self) -> None:
        message = self.message()
        manifest = self.repo / "Cargo.toml"
        original = manifest.read_text() + 'description = "uncommitted"\n'
        manifest.write_text(original)
        self.assertIn("stage or restore", self.cli("commit-msg", message, success=False).stderr)
        self.assertEqual(manifest.read_text(), original)
        self.write_versions("1.3.0")
        self.git("add", ".")
        self.assertIn("requires 1.2.4", self.cli("commit-msg", message, success=False).stderr)
        self.assertEqual(self.git("diff"), "")
        self.write_versions("1.2.4", "1.2.3")
        self.git("add", ".")
        self.assertIn("versions differ", self.cli("commit-msg", message, success=False).stderr)
        self.write_versions("1.2.4+build")
        self.git("add", ".")
        self.assertIn("requires 1.2.4", self.cli("commit-msg", message, success=False).stderr)
        self.write_versions("1.2.4", "1.2.4+build")
        self.git("add", ".")
        self.assertIn("versions differ", self.cli("commit-msg", message, success=False).stderr)

    def test_verifies_ranges_and_version_mismatches(self) -> None:
        for number, subject in (
            ("1.2.4", "fix: patch"),
            ("1.3.0", "feat: minor"),
            ("2.0.0", "refactor!: major"),
        ):
            self.write_versions(number)
            self.commit(subject)
        for base in ("v1.2.3", version.ZERO_OID, "HEAD"):
            self.cli("verify-range", base, "HEAD")
        self.write_versions("2.0.1", "2.0.0")
        self.commit("fix: mismatched lock")
        self.assertIn(
            "Cargo.lock records", self.cli("verify-range", "HEAD^", "HEAD", success=False).stderr
        )
        self.write_versions("2.1.0")
        self.commit("fix: wrong increment")
        self.assertIn(
            "expected 2.0.2", self.cli("verify-range", "HEAD^", "HEAD", success=False).stderr
        )

    def test_push_input_and_prek_environment(self) -> None:
        base = self.git("rev-parse", "HEAD")
        self.write_versions("1.2.4")
        head = self.commit("fix: adjust behaviour")
        self.cli("pre-push", input=f"\nrefs/heads/main {head} refs/heads/main {version.ZERO_OID}\n")
        self.cli("pre-push", input=f"(delete) {version.ZERO_OID} refs/heads/main {head}\n")
        self.cli("pre-push", env={"PRE_COMMIT_TO_REF": version.ZERO_OID})
        self.assertIn(
            "invalid pre-push", self.cli("pre-push", input="invalid\n", success=False).stderr
        )
        head = self.commit("fix: missing bump")
        self.cli(
            "pre-push", input=f"refs/heads/main {head} refs/heads/main {base}\n", success=False
        )
        self.cli(
            "pre-push", env={"PRE_COMMIT_FROM_REF": base, "PRE_COMMIT_TO_REF": head}, success=False
        )
        self.cli("pre-push", env={"PRE_COMMIT_TO_REF": head}, success=False)

    def test_initial_commit_and_amendment_baselines(self) -> None:
        self.git("checkout", "--orphan", "initial")
        self.git("tag", "-d", "v1.2.3")
        self.cli("commit-msg", self.message("feat: initial"))
        self.commit("feat: initial")
        self.cli("verify-range", version.ZERO_OID, "HEAD")
        hook = self.repo / ".git/hooks/commit-msg"
        hook.write_text(
            f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(SCRIPT))} commit-msg "$1"\n'
        )
        hook.chmod(0o755)
        self.cli("amend", "--no-edit")
        self.write_versions("1.2.4")
        self.commit("fix: patch")
        self.cli("amend", "--no-edit")
        self.cli("verify-range", "HEAD^", "HEAD")
        self.cli("amend", "-m", "feat: requires minor", success=False)

    def test_failed_explicit_publication_retries_without_recreating_tags(self) -> None:
        self.write_versions("1.2.4")
        head = self.commit("fix: patch")
        self.cli("tag-range", "v1.2.3", "HEAD", success=False)
        tag = self.git("rev-parse", "v1.2.4")
        self.remote()
        self.git("tag", "-a", "unrelated", "-m", "unrelated")
        self.git("config", "push.followTags", "true")
        for _ in range(2):
            self.cli("tag-range", "v1.2.3", "HEAD")
        self.assertEqual(self.git("rev-parse", "v1.2.4"), tag)
        self.assertEqual(self.git("cat-file", "-t", "v1.2.4"), "tag")
        self.assertIn(head, self.git("ls-remote", "origin", "refs/tags/v1.2.4^{}"))
        self.assertEqual(self.git("ls-remote", "origin", "refs/tags/unrelated"), "")

    def test_automatic_publication_retries_failed_push(self) -> None:
        remote = self.remote()
        releases = []
        for number in ("1.2.4", "1.2.5"):
            self.write_versions(number)
            releases.append((f"v{number}", self.commit("fix: patch")))
        reject = remote / "hooks/pre-receive"
        reject.write_text("#!/bin/sh\nexit 1\n")
        reject.chmod(0o755)
        self.cli("tag-range", version.ZERO_OID, "HEAD", success=False)
        objects = [self.git("rev-parse", tag) for tag, _ in releases]
        reject.unlink()
        for _ in range(2):
            self.cli("tag-range", version.ZERO_OID, "HEAD")
        for (tag, commit), original in zip(releases, objects, strict=True):
            self.assertEqual(self.git("rev-parse", tag), original)
            self.assertIn(commit, self.git("ls-remote", "origin", f"refs/tags/{tag}^{{}}"))

    def test_remote_baseline_ignores_unpublished_and_unrelated_tags(self) -> None:
        # Run the same contract with lightweight and annotated published baselines.
        for annotated in (False, True):
            with self.subTest(annotated=annotated):
                if annotated:
                    self.git("tag", "-fa", "v1.2.3", "HEAD", "-m", "baseline")
                remote = self.remote()
                self.git("tag", "-d", "v1.2.3")
                self.write_versions("1.2.4")
                head = self.commit("fix: patch")
                self.git("tag", "-a", "v8.8.8", "-m", "unpublished")
                self.git("checkout", "-b", "unrelated", "HEAD^")
                self.write_versions("9.9.9")
                self.commit("feat: unrelated history")
                self.git("tag", "v9.9.9")
                self.git("tag", "non-version")
                self.git("push", "origin", "refs/tags/v9.9.9", "refs/tags/non-version")
                self.git("checkout", "main")
                self.git("config", "push.followTags", "true")
                self.cli("tag-range", version.ZERO_OID, "HEAD")
                self.assertIn(head, self.git("ls-remote", "origin", "refs/tags/v1.2.4^{}"))
                self.assertEqual(self.git("ls-remote", "origin", "refs/tags/v8.8.8"), "")
                # Restore only this disposable fixture for the annotated case.
                self.git("remote", "remove", "origin")
                self.git("reset", "--hard", "HEAD^")
                self.git("branch", "-D", "unrelated")
                self.git("tag", "-d", "v1.2.4", "v8.8.8", "v9.9.9", "non-version")
                self.git("tag", "v1.2.3")
                shutil.rmtree(remote)

    def test_automatic_release_validates_before_creating_or_publishing_tags(self) -> None:
        self.remote()
        self.write_versions("1.2.4")
        self.commit("fix: valid")
        self.git("tag", "v1.2.4")
        self.commit("fix: missing increment")
        self.cli("tag-range", version.ZERO_OID, "HEAD", success=False)
        self.assertEqual(self.git("tag", "--list"), "v1.2.3\nv1.2.4")
        self.assertEqual(self.git("ls-remote", "origin", "refs/tags/v1.2.4"), "")

    def test_initial_release_and_missing_origin(self) -> None:
        initial = self.git("rev-parse", "HEAD")
        self.write_versions("1.2.4")
        head = self.commit("fix: patch")
        self.cli("tag-range", version.ZERO_OID, "HEAD", success=False)
        self.assertEqual(self.git("tag", "--list"), "v1.2.3")
        self.remote(baseline=False)
        self.cli("tag-range", version.ZERO_OID, "HEAD")
        self.assertIn(initial, self.git("ls-remote", "origin", "refs/tags/v1.2.3"))
        self.assertIn(head, self.git("ls-remote", "origin", "refs/tags/v1.2.4^{}"))

    def test_conflicting_tags_and_complete_explicit_validation(self) -> None:
        initial = self.git("rev-parse", "HEAD")
        self.git("tag", "v1.2.4")
        self.write_versions("1.2.4")
        self.commit("fix: patch")
        self.assertIn(
            "already points at", self.cli("tag-range", "v1.2.3", "HEAD", success=False).stderr
        )
        self.assertEqual(self.git("rev-parse", "v1.2.4"), initial)
        self.git("tag", "-d", "v1.2.4")
        self.commit("fix: missing increment")
        self.cli("tag-range", "v1.2.3", "HEAD", success=False)
        self.assertEqual(self.git("tag", "--list"), "v1.2.3")

    def test_rejects_merges_and_malformed_manifests(self) -> None:
        self.git("checkout", "-b", "side")
        (self.repo / "side").write_text("side")
        self.commit("fix: side")
        self.git("checkout", "main")
        (self.repo / "main").write_text("main")
        self.commit("fix: main")
        self.git("merge", "--no-ff", "side", "-m", "fix: merge")
        self.cli("verify-range", "HEAD^", "HEAD", success=False)
        self.cli("amend", "--no-edit", success=False)
        for manifest, lock in (
            ("[package", ""),
            ('[package]\nname="fixture-package"\n', ""),
            (
                '[package]\nname="fixture-package"\nversion="1.2.3"\n',
                '[[package]]\nname="other"\nversion="0.0.0"\n',
            ),
            (
                '[package]\nname="fixture-package"\nversion="1.2.3"\n',
                'package = ["invalid"]\n',
            ),
            (
                '[package]\nname="fixture-package"\nversion="1.2.3"\n',
                '[package]\nname="fixture-package"\nversion="1.2.3"\n',
            ),
        ):
            (self.repo / "Cargo.toml").write_text(manifest)
            (self.repo / "Cargo.lock").write_text(lock)
            self.git("add", ".")
            result = self.cli("commit-msg", self.message(), success=False)
            self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
