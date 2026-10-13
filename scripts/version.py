#!/usr/bin/env -S uv run
"""Enforce commit versions and publish validated, annotated release tags."""

import argparse
import os
import pathlib
import re
import subprocess
import sys
import tomllib

import semver
import tomlkit
import tomlkit.exceptions

ZERO_OID = "0" * 40
MANIFESTS = ("Cargo.toml", "Cargo.lock")
HEADER = re.compile(
    r"(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)"
    r"(?:\([^()]+\))?(!)?: "
)
MAX_COMPONENT = 2**64 - 1


def policy(message: str) -> tuple[str, str]:
    """Interpret the first non-comment subject and any breaking-change footer."""
    subject = next(
        (
            line.strip()
            for line in message.splitlines()
            if line.strip() and not line.strip().startswith("#")
        ),
        "",
    )
    if not subject:
        raise ValueError("commit message is empty")
    header = HEADER.match(subject)
    if not header:
        raise ValueError("a Conventional Commit title is required")
    breaking = header[2] or any(
        line.lstrip().startswith(("BREAKING CHANGE:", "BREAKING-CHANGE:"))
        for line in message.splitlines()
    )
    return subject, "major" if breaking else "minor" if header[1] == "feat" else "patch"


def parse_version(value: str) -> semver.Version:
    """Validate the same unsigned semantic-version components as the Rust helper."""
    version = semver.Version.parse(value)
    if max(version.major, version.minor, version.patch) > MAX_COMPONENT:
        raise ValueError("semantic-version component overflow")
    return version


def bumped(version: semver.Version, kind: str) -> semver.Version:
    """Increment the required component and discard pre-release and build metadata."""
    if getattr(version, kind) == MAX_COMPONENT:
        raise ValueError(f"{kind} version overflow")
    if kind == "major":
        return semver.Version(version.major + 1, 0, 0)
    if kind == "minor":
        return semver.Version(version.major, version.minor + 1, 0)
    return semver.Version(version.major, version.minor, version.patch + 1)


def git(*args: str, optional: bool = False) -> str | None:
    """Execute Git and retain its failure diagnostics."""
    result = subprocess.run(["git", *args], capture_output=True, text=True, check=False)
    if result.returncode:
        if optional:
            return None
        raise ValueError(result.stderr.strip() or result.stdout.strip() or "git command failed")
    return result.stdout.rstrip("\n")


def output(*args: str) -> str:
    """Read the output of a required Git command."""
    return git(*args) or ""


def package_at(reference: str) -> tuple[str, semver.Version]:
    """Read the package identity and version from a tree or the index."""
    target = ":Cargo.toml" if reference == ":" else f"{reference}:Cargo.toml"
    package = tomllib.loads(output("show", target))["package"]
    return package["name"], parse_version(package["version"])


def lock_version(reference: str, name: str) -> semver.Version:
    """Locate the manifest's package in the corresponding lockfile snapshot."""
    target = ":Cargo.lock" if reference == ":" else f"{reference}:Cargo.lock"
    packages = tomllib.loads(output("show", target))["package"]
    if not isinstance(packages, list) or any(not isinstance(package, dict) for package in packages):
        raise ValueError("Cargo.lock is missing package tables")
    for package in packages:
        if package.get("name") == name:
            return parse_version(package["version"])
    raise ValueError(f"Cargo.lock does not contain {name} package")


def parents(commit: str) -> list[str]:
    """Read immediate parents without traversing the remaining history."""
    return output("rev-list", "--parents", "-n", "1", commit).split()[1:]


def commits_in_range(base: str, head: str) -> list[str]:
    """Verify new branches from the latest reachable local version tag."""
    if base == ZERO_OID:
        base = (
            git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", head, optional=True) or ""
        )
    return output("rev-list", "--reverse", f"{base}..{head}" if base else head).splitlines()


def verify_commit(commit: str) -> None:
    """Require linear history, the prescribed increment and a matching lockfile."""
    subject, kind = policy(output("log", "-1", "--format=%B", commit))
    name, actual = package_at(commit)
    previous = parents(commit)
    if len(previous) > 1:
        raise ValueError(f"commit {commit} has multiple parents; rebase before release")
    if previous:
        _, parent_version = package_at(previous[0])
        expected = bumped(parent_version, kind)
        if str(actual) != str(expected):
            raise ValueError(
                f"commit {commit} `{subject}` has Cargo version {actual}; "
                f"expected {expected} from parent {parent_version}"
            )
    locked = lock_version(commit, name)
    if str(locked) != str(actual):
        raise ValueError(
            f"commit {commit} has Cargo.toml version {actual}, but Cargo.lock records {locked}"
        )


def verify_range(base: str, head: str) -> None:
    """Check every commit in the requested range."""
    for commit in commits_in_range(base, head):
        verify_commit(commit)


def update_version(version: semver.Version) -> None:
    """Edit only package versions, preserving TOML comments and representation."""
    manifest = tomlkit.parse(pathlib.Path("Cargo.toml").read_text())
    lock = tomlkit.parse(pathlib.Path("Cargo.lock").read_text())
    package = manifest["package"]
    name = package["name"]
    matched = next((entry for entry in lock["package"] if entry.get("name") == name), None)
    if matched is None:
        raise ValueError(f"Cargo.lock does not contain {name} package")
    package["version"] = str(version)
    matched["version"] = str(version)
    pathlib.Path("Cargo.toml").write_text(tomlkit.dumps(manifest))
    pathlib.Path("Cargo.lock").write_text(tomlkit.dumps(lock))


def commit_msg(message_file: str) -> None:
    """Validate the index or generate an unstaged increment for explicit staging."""
    subject, kind = policy(pathlib.Path(message_file).read_text())
    base = os.environ.get("GIT_VERSION_BASE", "HEAD")
    name, staged = package_at(":")
    if str(lock_version(":", name)) != str(staged):
        raise ValueError("staged Cargo.toml and Cargo.lock versions differ")
    if base == "ROOT" or (
        base == "HEAD" and git("rev-parse", "--verify", "HEAD", optional=True) is None
    ):
        return
    _, previous = package_at(base)
    expected = bumped(previous, kind)
    if str(staged) == str(expected):
        return
    if str(staged) != str(previous):
        raise ValueError(
            f"staged Cargo package version is {staged}, but `{subject}` requires {expected} from {previous}"
        )
    if output("diff", "--", *MANIFESTS):
        raise ValueError(
            "stage or restore existing manifest and lockfile edits before generating a version bump"
        )
    update_version(expected)
    raise ValueError(
        f"Version bump required: {previous} -> {expected} for `{subject}`.\n"
        "Updated Cargo.toml and Cargo.lock; review, stage them, and re-run git commit."
    )


def amend(args: list[str]) -> None:
    """Retain the existing commit's parent as the amendment baseline."""
    previous = parents("HEAD")
    if len(previous) > 1:
        raise ValueError("versioned commits must have at most one parent")
    environment = dict(os.environ, GIT_VERSION_BASE=previous[0] if previous else "ROOT")
    result = subprocess.run(["git", "commit", "--amend", *args], env=environment, check=False)
    if result.returncode:
        raise ValueError("git commit --amend failed")


def pre_push() -> None:
    """Consume prek's revision environment or native Git pre-push input."""
    if "PRE_COMMIT_TO_REF" in os.environ:
        head = os.environ["PRE_COMMIT_TO_REF"]
        if head != ZERO_OID:
            verify_range(os.environ.get("PRE_COMMIT_FROM_REF", ZERO_OID), head)
        return
    for line in sys.stdin:
        if not line.strip():
            continue
        fields = line.split()
        if len(fields) != 4:
            raise ValueError(f"invalid pre-push input line: {line.strip()}")
        if fields[1] != ZERO_OID:
            verify_range(fields[3], fields[1])


def published_targets(remote_tags: str) -> set[str]:
    """Prefer peeled annotated targets and ignore non-semantic-version tags."""
    targets: dict[str, str] = {}
    for line in remote_tags.splitlines():
        target, separator, reference = line.partition("\t")
        if not separator or not reference.startswith("refs/tags/v"):
            continue
        version = reference.removeprefix("refs/tags/v").removesuffix("^{}")
        try:
            parse_version(version)
        except ValueError:
            continue
        if reference.endswith("^{}") or version not in targets:
            targets[version] = target
    return set(targets.values())


def release_commits(base: str, head: str) -> list[str]:
    """Establish automatic release progress from published tags, including retries."""
    if base != ZERO_OID:
        return commits_in_range(base, head)
    published = published_targets(output("ls-remote", "--tags", "origin"))
    base = next(
        (
            commit
            for commit in output("rev-list", "--first-parent", head).splitlines()
            if commit in published
        ),
        "",
    )
    return output("rev-list", "--reverse", f"{base}..{head}" if base else head).splitlines()


def tag_range(base: str, head: str) -> None:
    """Validate before creating tags, then publish only that range atomically."""
    existing = {}
    for line in output(
        "for-each-ref", "refs/tags", "--format=%(refname:short) %(objectname) %(*objectname)"
    ).splitlines():
        name, *objects = line.split()
        existing[name] = objects[-1]
    tags: dict[str, str] = {}
    for commit in release_commits(base, head):
        verify_commit(commit)
        _, version = package_at(commit)
        tag = f"v{version}"
        if tag in existing and existing[tag] != commit:
            raise ValueError(f"tag {tag} already points at {existing[tag]}, not {commit}")
        if tag in tags:
            raise ValueError(f"multiple commits claim tag {tag}")
        tags[tag] = commit
    for tag, commit in sorted(tags.items()):
        if tag not in existing:
            output("tag", "-a", tag, commit, "-m", tag)
    if tags:
        output(
            "push",
            "--atomic",
            "--no-follow-tags",
            "origin",
            *(f"refs/tags/{tag}:refs/tags/{tag}" for tag in sorted(tags)),
        )


def main(argv: list[str] | None = None) -> int:
    """Dispatch repository maintenance commands without compiling the application."""
    arguments = list(sys.argv[1:] if argv is None else argv)
    try:
        # Git's amendment arguments are forwarded verbatim, including option-like values.
        if arguments and arguments[0] == "amend":
            amend(arguments[1:])
            return 0
        parser = argparse.ArgumentParser(description=__doc__)
        commands = parser.add_subparsers(dest="command", required=True)
        commands.add_parser("amend", help="amend with the original version baseline")
        commands.add_parser("commit-msg").add_argument("message_file")
        commands.add_parser("pre-push")
        for name in ("verify-range", "tag-range"):
            command = commands.add_parser(name)
            command.add_argument("base")
            command.add_argument("head")
        args = parser.parse_args(arguments)
        if args.command == "commit-msg":
            commit_msg(args.message_file)
        elif args.command == "pre-push":
            pre_push()
        elif args.command == "verify-range":
            verify_range(args.base, args.head)
        elif args.command == "tag-range":
            tag_range(args.base, args.head)
    except (OSError, ValueError, KeyError, TypeError, tomlkit.exceptions.TOMLKitError) as error:
        print(f"version: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
