#!/usr/bin/env -S uv run
"""Compare isolated executable workflows and retain their complete evidence."""

import argparse
import collections
import concurrent.futures
import contextlib
import fcntl
import hashlib
import http.server
import io
import json
import os
import pathlib
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from collections.abc import Iterator, Sequence
from typing import Any

# HTTP fixtures deliberately retain heterogeneous JSON payloads for exact comparison.
type Record = dict[str, Any]

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORK = ROOT / "target/validation"
REVISION = "132dc66ea004d03606585c9bdfe95289bd0b9dcb"
REAL_GIT = shutil.which("git") or ""
if not REAL_GIT:
    raise RuntimeError("Git is required for executable acceptance")
ANSI = re.compile(r"\x1b\[[0-9;]*m")
REMOTE_GIT_SETTINGS = frozenset({"remote.pushDefault", "branch.*.pushRemote", "branch.*.remote"})
NEW_GIT_SETTINGS = (
    frozenset(
        {
            "git-pr.stackUp",
            "git-pr.output",
            "git-pr.addLabel",
            "git-pr.addTipLabel",
            "git-pr.compactStack",
            "git-pr.readTimeout",
            "git-pr.commandTimeout",
            "git-pr.transitionJournal",
        }
    )
    | REMOTE_GIT_SETTINGS
)
FOLLOWUP_GIT_SETTINGS = (
    frozenset(
        {
            "git-pr.stackUp",
            "git-pr.compactStack",
            "git-pr.readTimeout",
            "git-pr.commandTimeout",
            "git-pr.transitionJournal",
        }
    )
    | REMOTE_GIT_SETTINGS
)
ADDED_HELP = """  -compact-stack
    \tUse adjacent-PR navigation instead of the full stack footer (default false)
  -read-timeout int
    \tStructured command deadline in seconds (default 120; config git-pr.readTimeout)
  -command-timeout int
    \tBackend and publication command deadline in seconds (default 600; config git-pr.commandTimeout)
  -vv\tTiming plus detailed command and HTTP diagnostics (also -v -v)
"""
STACK_UP_HELP = """  -stack-up
    \tDisplay the stack with its base at the bottom and tip at the top (default true; false inverts it)
  -reverse
    \tDeprecated alias for -stack-up; retains the same Boolean value
"""
LEGACY_REVERSE_HELP = """  -reverse
    \tShow stack newest-first; use -reverse=false for oldest-first order (default true)
"""
RENAMED_GIT_SETTINGS = {
    "git-pr.draft": "git-pr.defaultDraft",
    "git-pr.github-stack": "git-pr.githubStack",
    "git-pr.stack-emoji": "git-pr.stackEmoji",
    "git-pr.branch-from-title": "git-pr.branchFromTitle",
}

# The legacy help advertises an option that has no effect in either implementation.
BRANCH_HELP = "Accepted compatibility switch; branch selection is unchanged"
LEGACY_BRANCH_HELP = "Generate branch names from commit title instead of hash"
VERBOSE_HELP = "Timing and progress output"
LEGACY_VERBOSE_HELP = "Verbose output"


def command(
    args: Sequence[str | pathlib.Path],
    cwd: pathlib.Path = ROOT,
    env: dict[str, str] | None = None,
    *,
    timeout: int = 600,
) -> str:
    """Execute a bounded fixture command and expose complete failure diagnostics."""
    result = subprocess.run(
        args, cwd=cwd, env=env, check=False, capture_output=True, text=True, timeout=timeout
    )
    if result.returncode:
        raise RuntimeError(
            f"{args!r} failed ({result.returncode})\n{result.stdout}\n{result.stderr}"
        )
    return result.stdout.strip()


def reference() -> pathlib.Path:
    """Extract the pinned executable specification without modifying its checkout."""
    destination = WORK / "reference"
    if not (destination / "go.mod").exists():
        source = os.environ.get("REFERENCE_REPOSITORY")
        if source:
            data = subprocess.check_output(["git", "-C", str(source), "archive", REVISION])
            prefix = ""
        else:
            with urllib.request.urlopen(
                f"https://codeload.github.com/duanyutong/git-pr-go/tar.gz/{REVISION}",
                timeout=60,
            ) as response:
                data = response.read()
            prefix = f"git-pr-go-{REVISION}/"
        destination.mkdir(parents=True, exist_ok=True)
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            for member in archive:
                if prefix:
                    if not member.name.startswith(prefix):
                        continue
                    member.name = member.name[len(prefix) :]
                if member.name:
                    archive.extract(member, destination, filter="data")
    return destination


def build(release: bool = False) -> pathlib.Path:
    """Serialise shared preparation whilst retaining independent scenario state."""
    WORK.mkdir(parents=True, exist_ok=True)
    # Reference extraction and Go compilation share immutable inputs across gates.
    # Serialise preparation; executable scenarios retain independent repositories.
    with (WORK / "reference-build.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return build_locked(release)


def build_locked(release: bool = False) -> pathlib.Path:
    """Validate the case ledger and build both executable boundary fixtures."""
    source = reference()
    env = dict(os.environ, GOCACHE=str(pathlib.Path.home() / ".cache/git-pr/go-build"))
    test_results = command(["go", "test", "-json", "-count=1", "./..."], cwd=source, env=env)
    results_path = WORK / "reference-tests.jsonl"
    results_path.write_text(test_results + "\n")
    command([sys.executable, "scripts/reference_inventory.py", "--results", str(results_path)])
    command(["go", "build", "-o", str(WORK / "reference-bin"), "."], cwd=source, env=env)
    adapted = WORK / "reference-fixture"
    adapted.mkdir(exist_ok=True)
    for path in source.iterdir():
        if path.is_file() and (path.suffix == ".go" or path.name in ("go.mod", "go.sum")):
            shutil.copy2(path, adapted / path.name)
    for path in adapted.glob("*.go"):
        path.write_text(path.read_text().replace("time.Sleep(", "fixtureSleep("))
    (adapted / "fixture_clock.go").write_text("""package main
import("encoding/json";"os";"sync";"time")
var fixtureClockLock sync.Mutex
func fixtureSleep(d time.Duration){start:=time.Now();time.Sleep(d);end:=time.Now();fixtureClockLock.Lock();defer fixtureClockLock.Unlock();f,e:=os.OpenFile(os.Getenv("GIT_PR_WAIT_FILE"),os.O_APPEND|os.O_CREATE|os.O_WRONLY,0600);if e!=nil{panic(e)};defer f.Close();e=json.NewEncoder(f).Encode(map[string]float64{"start":float64(start.UnixNano())/1e9,"end":float64(end.UnixNano())/1e9,"scheduled":d.Seconds()});if e!=nil{panic(e)}}
""")
    (adapted / "fixture_transport.go").write_text("""package main
import("net/http";"net/url";"os")
type fixtureTransport struct{ inner http.RoundTripper; endpoint *url.URL }
func(t fixtureTransport)RoundTrip(r *http.Request)(*http.Response,error){ c:=r.Clone(r.Context());u:=*r.URL;u.Scheme=t.endpoint.Scheme;u.Host=t.endpoint.Host;c.URL=&u;c.Host=t.endpoint.Host;return t.inner.RoundTrip(c) }
func init(){ if s:=os.Getenv("GIT_PR_FIXTURE_ENDPOINT");s!=""{u,e:=url.Parse(s);if e!=nil{panic(e)};http.DefaultTransport=fixtureTransport{http.DefaultTransport,u}} }
""")
    command(
        ["go", "build", "-o", str(WORK / "reference-fixture-bin"), "."],
        cwd=adapted,
        env=env,
    )
    profile = ["--release"] if release else []
    command(["cargo", "build", "--locked", *profile])
    output = command(
        ["cargo", "test", "--locked", "--lib", "--no-run", "--message-format=json", *profile]
    )
    executable = None
    for line in output.splitlines():
        entry = json.loads(line)
        if (
            entry.get("reason") == "compiler-artifact"
            and entry.get("profile", {}).get("test")
            and entry.get("executable")
        ):
            executable = entry["executable"]
    if not executable:
        raise RuntimeError("Candidate test executable was not emitted")
    return pathlib.Path(executable)


class Service:
    """Provide deterministic GitHub responses and record external operations."""

    def __init__(self, scenario: Record) -> None:
        self.prs: dict[int, Record] = {}
        self.trace: list[Record] = []
        self.lock = threading.Lock()
        self.scenario = scenario
        self.stack_calls = 0
        self.stack_errors = list(scenario.get("stack_errors", []))

    def handle(self, method: str, path: str, data: Any) -> tuple[int, Any]:
        parsed = urllib.parse.urlparse(path)
        with self.lock:
            if parsed.path == "/fixture/command":
                self.trace.append(dict(data, time=time.monotonic()))
                if data["event"] == "complete":
                    return 200, {}
                program, args = data["program"], data["args"]
                if program == "git" and args == ["remote", "-v"]:
                    return 200, {
                        "status": 0,
                        "output": "origin\thttps://github.com/org/repo.git (fetch)\norigin\thttps://github.com/org/repo.git (push)\n",
                    }
                if (
                    program == "git"
                    and args == ["branchless", "--version"]
                    and self.scenario.get("backend", "git") == "git"
                ):
                    return 200, {"status": 199, "output": ""}
                if program == "gh":
                    return 200, self.gh(args)
                failure = self.scenario.get("command_errors", {}).get(" ".join([program, *args]))
                if failure:
                    return 200, failure
                return 200, {}
            self.trace.append(
                {
                    "event": "http",
                    "method": method,
                    "path": path,
                    "body": data,
                    "time": time.monotonic(),
                }
            )
            return self.api(method, parsed, data)

    def api(self, method: str, parsed: urllib.parse.ParseResult, data: Any) -> tuple[int, Any]:
        """Model REST and GraphQL effects against the same PR and topology state."""
        suffix = parsed.path.removeprefix("/repos/org/repo/")
        if suffix == "/graphql":
            if data["query"].startswith("mutation"):
                if self.scenario.get("api_error"):
                    return 503, {"message": "API unavailable"}
                result = {}
                for key, value in data["variables"].items():
                    number = int(value["pullRequestId"].removeprefix("PR_"))
                    for field in ("title", "body"):
                        if field in value:
                            self.prs[number][field] = value[field]
                    result[key] = {"pullRequest": {"number": number}}
                return 200, {"data": result}
            result = {}
            for key, head in data["variables"].items():
                if key.startswith("n") and key[1:].isdigit():
                    pr = self.prs.get(int(head))
                    result["p" + key[1:]] = (
                        None
                        if pr is None
                        else {
                            "number": pr["number"],
                            "merged": pr["merged"] or pr.get("merged_at") is not None,
                        }
                    )
                    continue
                if key.startswith("b"):
                    result["p" + key[1:]] = {
                        "nodes": [
                            {"number": pr["number"], "headRefName": pr["head"]["ref"]}
                            for pr in self.prs.values()
                            if pr["base"]["ref"] == head and pr["state"] == "open"
                        ],
                        "pageInfo": {"hasNextPage": False},
                    }
                    continue
                if not key.startswith("h"):
                    continue
                nodes = [
                    {
                        "id": f"PR_{pr['number']}",
                        "number": pr["number"],
                        "title": pr["title"],
                        "body": pr["body"],
                        "state": pr["state"].upper(),
                        "merged": pr["merged"],
                        "headRefName": pr["head"]["ref"],
                        "baseRefName": pr["base"]["ref"],
                        "headRepository": {"nameWithOwner": "org/repo"},
                        "stack": pr.get("stack"),
                        "labels": {
                            "nodes": [{"name": label} for label in pr["labels"]],
                            "pageInfo": {"hasNextPage": False},
                        },
                    }
                    for pr in self.prs.values()
                    if pr["head"]["ref"] == head
                ]
                result["p" + key[1:]] = {"nodes": nodes, "pageInfo": {"hasNextPage": False}}
            return 200, {"data": {"repository": result}}
        if suffix == "pulls" and method == "POST":
            number = max(self.prs, default=100) + 1
            pr = dict(
                data,
                number=number,
                node_id=f"PR_{number}",
                head={"ref": data["head"], "repo": {"full_name": "org/repo"}},
                base={"ref": data["base"], "repo": {"full_name": "org/repo"}},
                state="open",
                merged=False,
                labels=[],
            )
            self.prs[number] = pr
            return 201, pr
        if suffix == "pulls":
            query = urllib.parse.parse_qs(parsed.query)
            head = query["head"][0].removeprefix("org:")
            state = query["state"][0]
            return 200, [
                pr
                for pr in self.prs.values()
                if pr["head"]["ref"] == head and (state == "all" or pr["state"] == state)
            ]
        if suffix.startswith("commits/"):
            return 200, [
                {**pr, "head": {**pr["head"], "repo": {"full_name": "org/repo"}}}
                for pr in self.prs.values()
            ]
        if suffix.startswith("pulls/"):
            number = int(suffix[6:])
            if number not in self.prs:
                return 404, {"message": "Not Found"}
            if method == "PATCH":
                if self.scenario.get("api_error"):
                    return 503, {"message": "API unavailable"}
                assert "draft" not in data
                if "base" in data:
                    if self.prs[number].get("stack"):
                        return 422, {"message": "pull request is part of a stack"}
                    self.prs[number]["base"]["ref"] = data["base"]
                for field in ("body", "title"):
                    if field in data:
                        self.prs[number][field] = data[field]
            return 200, self.prs[number]
        if suffix.startswith("issues/"):
            _, number, _, *label_path = suffix.split("/")
            labels = self.prs[int(number)]["labels"]
            if method == "POST":
                labels.extend(label for label in data["labels"] if label not in labels)
            elif method == "DELETE":
                label = urllib.parse.unquote("/".join(label_path))
                labels[:] = [old for old in labels if old != label]
            return 200, [{"name": label} for label in labels]
        if suffix == "stacks" or suffix.startswith("stacks/"):
            if self.scenario.get("stack_error", "").startswith("unknown command"):
                return 404, {"message": "Native stacks unavailable"}
            number = int(suffix.split("/")[1]) if suffix != "stacks" else 9
            if suffix.endswith("/unstack"):
                if self.stack_errors and "Cannot update stack" in self.stack_errors[0]:
                    self.stack_errors.pop(0)
                if self.stack_errors:
                    message = self.stack_errors.pop(0)
                    if "remain stacked" in message:
                        return 200, {"number": number, "pull_requests": list(self.prs.values())}
                    return 422, {"message": message}
                for pr in self.prs.values():
                    pr.pop("stack", None)
                return 204, None
            if method == "POST":
                for pr_number in data["pull_requests"]:
                    self.prs[pr_number]["stack"] = {"number": number}
            members = [
                pr for pr in self.prs.values() if pr.get("stack", {}).get("number") == number
            ]
            return 200, {
                "number": number,
                "base": {"ref": members[0]["base"]["ref"] if members else "main"},
                "pull_requests": members,
            }
        return 404, {"message": "unknown endpoint"}

    def gh(self, args: list[str]) -> Record:
        def flag(name: str) -> str:
            return args[args.index(name) + 1] if name in args else ""

        if args[:2] == ["pr", "create"]:
            number = max(self.prs, default=100) + 1
            self.prs[number] = {
                "number": number,
                "node_id": f"PR_{number}",
                "title": flag("--title"),
                "body": flag("--body"),
                "head": {"ref": flag("--head"), "repo": {"full_name": "org/repo"}},
                "base": {"ref": flag("--base"), "repo": {"full_name": "org/repo"}},
                "draft": "--draft" in args,
                "state": "open",
                "merged": False,
                "labels": [s for s in flag("--label").split(",") if s],
            }
            return {
                "status": 0,
                "output": f"https://github.com/org/repo/pull/{number}\n",
            }
        if args[:2] == ["pr", "edit"]:
            pr = self.prs[int(args[2])]
            if flag("--base"):
                if "stack" in pr:
                    return {
                        "status": 1,
                        "output": "GraphQL: Cannot change the base branch because the pull request is part of a stack. (updatePullRequest)",
                    }
                pr["base"]["ref"] = flag("--base")
            if flag("--add-label"):
                pr["labels"] = list(dict.fromkeys([*pr["labels"], *flag("--add-label").split(",")]))
            if flag("--remove-label"):
                pr["labels"] = [
                    s for s in pr["labels"] if s not in flag("--remove-label").split(",")
                ]
            return {"status": 0, "output": ""}
        if args[:2] == ["pr", "list"]:
            return {"status": 0, "output": ""}
        if args[0] == "stack" and args[1] in ("link", "unstack"):
            self.stack_calls += 1
            if self.stack_errors:
                error = self.stack_errors.pop(0)
                if error:
                    return {"status": 0 if "remain stacked" in error else 1, "output": error}
            if self.scenario.get("stack_error"):
                return {"status": 1, "output": self.scenario["stack_error"]}
            if args[1] == "unstack":
                for pr in self.prs.values():
                    pr.pop("stack", None)
            if args[1] == "link":
                members = [
                    next(pr for pr in self.prs.values() if pr["head"]["ref"] == name)
                    for name in args[2:]
                ]
                for index, pr in enumerate(members):
                    pr["stack"] = {"number": 9}
                    pr["base"]["ref"] = members[index - 1]["head"]["ref"] if index else "main"
                self.trace.append(
                    {"event": "native-child", "time": time.monotonic(), "pushes": len(members)}
                )
            return {"status": 0, "output": "Stack linked\n"}
        raise AssertionError(f"Unexpected gh invocation: {args}")


@contextlib.contextmanager
def server(service: Service, evidence_path: pathlib.Path) -> Iterator[str]:
    """Own a loopback service and close its listener when the scenario concludes."""

    class FixtureServer(http.server.ThreadingHTTPServer):
        # Concurrent submissions also report command starts and completions.
        # Avoid artificial connection stalls from the default backlog of five.
        request_queue_size = 128

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            self.respond()

        def do_POST(self) -> None:
            self.respond()

        def do_PATCH(self) -> None:
            self.respond()

        def do_DELETE(self) -> None:
            self.respond()

        def respond(self) -> None:
            length = int(self.headers.get("Content-Length", "0"))
            data = json.loads(self.rfile.read(length)) if length else None
            # Delay service traffic only; fixture command telemetry is local overhead.
            if not self.path.startswith("/fixture/"):
                time.sleep(service.scenario.get("latency_ms", 0) / 1000)
            status, result = service.handle(self.command, self.path, data)
            body = b"" if status == 204 else json.dumps(result).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            pass

    httpd = FixtureServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{httpd.server_port}"
    except Exception as error:
        try:
            evidence_path.mkdir(parents=True, exist_ok=True)
            (evidence_path / "failure.json").write_text(
                json.dumps(
                    {"error": str(error), "trace": service.trace, "prs": service.prs}, indent=2
                )
            )
        except OSError:
            # Preserve the original failure when storage itself is unavailable.
            pass
        raise
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)


def fixture_environment() -> dict[str, str]:
    """Inherit executable and instrumentation paths without copying user credentials."""
    allowed = {
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LLVM_PROFILE_FILE",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "LD_LIBRARY_PATH",
    }
    return {key: value for key, value in os.environ.items() if key in allowed}


def seed(
    path: pathlib.Path, scenario: Record, endpoint: str
) -> tuple[pathlib.Path, dict[str, str]]:
    """Create private repositories, credentials and deterministic backend configuration."""
    real_jj = shutil.which("jj")
    if scenario.get("backend", "").startswith("jj") and real_jj is None:
        raise RuntimeError("Jujutsu is required; run setup.sh with the declared backend version")
    home = path / "home"
    repo = path / "repo"
    remote = path / "remote.git"
    bin_dir = path / "bin"
    for directory in (home / ".config/gh", repo, bin_dir):
        directory.mkdir(parents=True)
    (home / ".config/gh/hosts.yml").write_text(
        "github.com:\n  user: fixture\n  oauth_token: fixture-token\n"
    )
    # The fixture's isolated PATH must retain the parent uv interpreter.
    (bin_dir / "python3").symlink_to(sys.executable)
    for name in ("git", "gh", "jj"):
        (bin_dir / name).symlink_to(ROOT / "scripts/fixture_command.py")
    env = fixture_environment()
    env.update(
        {
            "HOME": str(home),
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_CEILING_DIRECTORIES": str(path),
            "LC_ALL": "C",
            "GIT_AUTHOR_DATE": "2025-01-01T00:00:00Z",
            "GIT_COMMITTER_DATE": "2025-01-01T00:00:00Z",
            "GIT_PR_FIXTURE_ENDPOINT": endpoint,
            "GIT_PR_REAL_GIT": REAL_GIT or "git",
            "GIT_PR_REAL_JJ": real_jj or "jj",
            "GIT_PR_REAL_GH": shutil.which("gh") or "gh",
            "PATH": str(bin_dir) + os.pathsep + os.environ["PATH"],
        }
    )

    def git(*args: str) -> str:
        return command([REAL_GIT, *args], repo, env)

    git("init", "--initial-branch=main")
    git("config", "user.name", "Fixture Author")
    git("config", "user.email", "fixture@example.com")
    git("config", "commit.gpgsign", "false")
    git("config", "color.ui", "false")
    git("config", "log.date", "default")
    for relative, contents in scenario.get("files", {}).items():
        target = repo / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(contents)
    (repo / "base").write_text("base\n")
    git("add", ".")
    git("commit", "-m", "Base")
    git("init", "--bare", str(remote))
    git("remote", "add", "origin", str(remote))
    git("push", "origin", "main")
    git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main")
    if scenario.get("backend") == "branchless":
        git("branchless", "init", "--main-branch", "main")
        git("config", "branchless.restack.preserveTimestamps", "true")
    for i in range(1, scenario.get("size", 2) + 1):
        git("checkout", "-b", f"topic/{i}")
        (repo / f"file-{i}").write_text(f"content {i}\n")
        git("add", ".")
        trailer = "" if scenario.get("rewrite") else f"\n\nRemote-Ref: topic/{i}"
        git("commit", "-m", f"Feature {i}\n\nBody {i}{trailer}")
    if scenario.get("backend", "").startswith("jj"):
        config = home / "jj.toml"
        config.write_text(
            "[user]\nname='Fixture Author'\nemail='fixture@example.com'\n[ui]\ncolor='never'\n"
            "[debug]\nrandomness-seed=42\ncommit-timestamp='2025-01-01T00:00:00Z'\n"
            "operation-timestamp='2025-01-01T00:00:00Z'\n"
        )
        env["JJ_CONFIG"] = str(config)
        jj = env["GIT_PR_REAL_JJ"]
        command([jj, "git", "init", "--colocate"], repo, env)
        if scenario["backend"] == "jj-workspace":
            workspace = path / "workspace"
            command(
                [
                    jj,
                    "--config",
                    "debug.randomness-seed=43",
                    "workspace",
                    "add",
                    "--no-colocate",
                    str(workspace),
                ],
                repo,
                env,
            )
            repo = workspace
    return repo, env


def normalise(text: str, root: pathlib.Path) -> str:
    """Remove disposable paths, terminal colour and language runtime backtraces."""
    text = ANSI.sub("", text).replace(str(root), "<fixture>")
    # Runtime backtraces differ by language; preserve the complete error message.
    if text.startswith("panic: "):
        text = text.removeprefix("panic: ").split("\n\ngoroutine ", 1)[0]
    return text.rstrip() + "\n"


def run_one(kind: str, test_binary: pathlib.Path, scenario: Record, path: pathlib.Path) -> Record:
    """Retain each scenario step and its output, mutations and operation trace."""
    lifecycle_started = time.monotonic()
    mutation_seconds = 0.0
    snapshot_seconds = 0.0
    service = Service(scenario)
    with server(service, path) as endpoint:
        repo, env = seed(path, scenario, endpoint)
        initial_snapshot = snapshot(repo, env, service)
        preparation_seconds = time.monotonic() - lifecycle_started
        service.trace.clear()
        samples = []
        outputs = []
        runs = []
        snapshots = []
        for step in scenario.get("runs", [scenario.get("args", [])]):
            mutation_started = time.monotonic()
            args = step.get("args", []) if isinstance(step, dict) else step
            if isinstance(step, dict):
                if step.get("amend"):
                    message = "Feature 1 amended\n\nUpdated body\n\nRemote-Ref: topic/1"
                    if scenario.get("backend", "").startswith("jj"):
                        command(
                            [
                                env["GIT_PR_REAL_JJ"],
                                "--ignore-immutable",
                                "describe",
                                "-r",
                                "topic/1",
                                "-m",
                                message,
                            ],
                            repo,
                            env,
                        )
                    else:
                        command([REAL_GIT, "reword", "topic/1", "-m", message], repo, env)
                if step.get("stacked"):
                    for pr in service.prs.values():
                        pr["stack"] = {"number": 9}
                        pr["base"]["ref"] = "stale"
                    service.stack_errors = list(step.get("stack_errors", []))
                if step.get("checkout"):
                    command([REAL_GIT, "checkout", step["checkout"]], repo, env)
            mutation_seconds += time.monotonic() - mutation_started
            wait_file = path / "waits.jsonl"
            wait_file.write_text("")
            env["GIT_PR_WAIT_FILE"] = str(wait_file)
            trace_start = len(service.trace)
            started = time.monotonic()
            invocation_args = [
                arg.replace("--stack-up", "--reverse", 1)
                if kind == "reference" and arg.partition("=")[0] == "--stack-up"
                else arg
                for arg in args
            ]
            if kind == "reference" and scenario.get("reference_accept_native"):
                invocation_args.append("--yes")
            if kind == "reference" and not scenario.get("rust_reference"):
                result = subprocess.run(
                    [str(WORK / "reference-fixture-bin"), *invocation_args],
                    cwd=repo,
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=scenario.get("submission_timeout", 90),
                    input="" if scenario.get("pipe") else None,
                )
                status = result.returncode
                stdout = result.stdout
                stderr = result.stderr
            else:
                input_path = path / "input.json"
                status_path = path / "status"
                input_path.write_text(
                    json.dumps(
                        {
                            "cwd": str(repo),
                            "env": env,
                            "args": invocation_args,
                            "endpoint": endpoint,
                            "status": str(status_path),
                        }
                    )
                )
                result = subprocess.run(
                    [
                        str(scenario["rust_reference"] if kind == "reference" else test_binary),
                        "--exact",
                        "tests::differential::differential_entrypoint",
                        "--nocapture",
                    ],
                    cwd=repo,
                    env=dict(env, GIT_PR_TEST_INPUT=str(input_path)),
                    capture_output=True,
                    text=True,
                    timeout=scenario.get("submission_timeout", 90),
                    input="" if scenario.get("pipe") else None,
                )
                if result.returncode:
                    raise RuntimeError(result.stdout + result.stderr)
                status = int(status_path.read_text())
                stdout = result.stdout.split("running 1 test\n", 1)[1].rsplit(
                    "test tests::differential::differential_entrypoint ... ok", 1
                )[0]
                stderr = result.stderr
            samples.append(time.monotonic() - started)
            outputs.append(
                {
                    "status": status,
                    "stdout": normalise(stdout, path),
                    "stderr": normalise(stderr, path),
                }
            )
            trace = service.trace[trace_start:]
            runs.append(
                {
                    "args": args,
                    "trace_start": trace_start,
                    "trace_end": len(service.trace),
                    **phase_evidence(
                        trace,
                        started,
                        samples[-1],
                        [json.loads(line) for line in wait_file.read_text().splitlines()],
                    ),
                }
            )
            snapshot_started = time.monotonic()
            snapshots.append(snapshot(repo, env, service))
            snapshot_seconds += time.monotonic() - snapshot_started
            expected = (
                step.get("status", scenario.get("status", 0))
                if isinstance(step, dict)
                else scenario.get("status", 0)
            )
            if kind == "candidate" and scenario.get("stack_error", "").startswith("! [rejected]"):
                expected = 0  # The redundant extension push no longer exists.
            if kind == "candidate" and "candidate_status" in scenario:
                expected = scenario["candidate_status"]
            if kind == "candidate" and isinstance(step, dict) and "candidate_status" in step:
                expected = step["candidate_status"]
            if status != expected:
                (path / "unexpected-status.json").write_text(
                    json.dumps(
                        {"outputs": outputs, "snapshots": snapshots, "trace": service.trace},
                        indent=2,
                    )
                )
                raise AssertionError(
                    f"Expected status {expected}, got {status}: {path}\n{stdout}\n{stderr}"
                )
        evidence = {
            "outputs": outputs,
            "initial_snapshot": initial_snapshot,
            "snapshot": snapshots,
            "trace": service.trace,
            "seconds": samples,
            "phases": runs,
        }
        cleanup_started = time.monotonic()
    evidence["fixture_seconds"] = {
        "preparation": preparation_seconds,
        "mutation": mutation_seconds,
        "snapshots": snapshot_seconds,
        "cleanup": time.monotonic() - cleanup_started,
        "total": time.monotonic() - lifecycle_started,
    }
    (path / "evidence.json").write_text(json.dumps(evidence, indent=2, ensure_ascii=False))
    return evidence


def snapshot(repo: pathlib.Path, env: dict[str, str], service: Service) -> Record:
    """Capture repository identity, checkout state and simulated pull requests."""
    git_env = dict(env)
    if service.scenario.get("backend") == "jj-workspace":
        git_env["GIT_DIR"] = command([env["GIT_PR_REAL_JJ"], "git", "root"], repo, env)
    refs = command([REAL_GIT, "for-each-ref", "--format=%(refname)|%(objectname)"], repo, git_env)
    commits = command(
        [REAL_GIT, "log", "--all", "--format=%H%x00%T%x00%P%x00%B%x00", "--no-decorate"],
        repo,
        git_env,
    )
    checkout = command([REAL_GIT, "rev-parse", "HEAD"], repo, git_env)
    branch = command([REAL_GIT, "branch", "--show-current"], repo, git_env)
    working_copy = ""
    if service.scenario.get("backend", "").startswith("jj"):
        working_copy = command(
            [
                env["GIT_PR_REAL_JJ"],
                "--ignore-working-copy",
                "log",
                "-r",
                "@",
                "--no-graph",
                "-T",
                "change_id ++ commit_id",
            ],
            repo,
            env,
        )
    return {
        "refs": refs,
        "commits": commits,
        "checkout": checkout,
        "branch": branch,
        "working_copy": working_copy,
        "prs": json.loads(json.dumps(service.prs)),
    }


def phase_evidence(
    trace: list[Record], started: float, elapsed: float, waits: list[Record]
) -> Record:
    """Verify phase barriers and distinguish operation counts from elapsed spans."""
    phases: dict[str, list[Record]] = collections.defaultdict(list)
    for event in trace:
        program, args = event.get("program"), event.get("args", [])
        phase = "discovery"
        if (program == "git" and args[:1] == ["reword"]) or (
            program == "jj" and args[:1] == ["describe"]
        ):
            phase = "rewrite"
        elif program == "git" and args[:1] == ["push"]:
            phase = "push"
        elif program == "gh" and args[:2] == ["pr", "create"]:
            phase = "create"
        elif program == "gh" and args[:2] == ["pr", "edit"]:
            phase = "base" if "--base" in args else "labels"
        elif program == "gh" and args[:1] == ["stack"]:
            phase = "native"
        elif event["event"] == "http":
            path = urllib.parse.urlparse(event["path"]).path
            body = event.get("body") or {}
            if "/stacks" in path:
                phase = "native"
            elif path.endswith("/pulls") and event["method"] == "POST":
                phase = "create"
            elif "/labels" in path and event["method"] != "GET":
                phase = "labels"
            elif event["method"] == "PATCH":
                phase = "base" if "base" in body else "description"
            elif body.get("query", "").startswith("mutation"):
                phase = "description"
            else:
                phase = "lookup"
        phases[phase].append(event)

    def ends(phase: str) -> list[float]:
        return [e["time"] for e in phases[phase] if e["event"] == "complete"]

    def starts(phase: str) -> list[float]:
        return [e["time"] for e in phases[phase] if e["event"] in ("start", "http")]

    for before, after in [
        ("push", "create"),
        ("push", "base"),
        ("create", "base"),
        ("create", "description"),
        ("base", "description"),
        ("labels", "native"),
    ]:
        if ends(before) and starts(after):
            assert max(ends(before)) < min(starts(after)), (
                f"Phase barrier violated: {before} -> {after}"
            )
    if starts("description") and starts("native"):
        assert max(starts("description")) < min(starts("native")), (
            "Descriptions must precede native linking"
        )
    return {
        "elapsed": elapsed,
        "commands": sum(e["event"] == "start" for e in trace),
        "requests": sum(e["event"] == "http" for e in trace),
        "new_git_settings": dict(
            collections.Counter(
                key for e in trace if e["event"] == "start" for key in new_git_settings(e)
            )
        ),
        "github_stack_setting_reads": sum(
            logical[-1] in {"git-pr.github-stack", "git-pr.githubStack"}
            for event in trace
            if event["event"] == "start"
            for logical in configuration_reads(event)
        ),
        "deliberate_wait_seconds": wait_union(waits),
        "scheduled_wait_seconds": sum(w["scheduled"] for w in waits),
        "waits": waits,
        "native_child_pushes": sum(
            e.get("pushes", 0) for e in trace if e["event"] == "native-child"
        ),
        "phases": {
            name: {
                "first": min(e["time"] for e in events) - started,
                "last": max(e["time"] for e in events) - started,
                "operations": sum(e["event"] in ("start", "http") for e in events),
            }
            for name, events in phases.items()
            if events
        },
    }


def wait_union(waits: list[Record]) -> float:
    """Measure the union of actual wait intervals, including overlapping workers."""
    total, right = 0.0, 0.0
    for wait in sorted(waits, key=lambda w: w["start"]):
        total += max(0.0, wait["end"] - max(right, wait["start"]))
        right = max(right, wait["end"])
    return total


def configuration_reads(event: Record) -> list[list[str]]:
    """Expand framed batch queries into their declared logical setting reads."""
    args = event.get("args", [])
    if event.get("program") != "git" or args[:1] != ["config"]:
        return []
    if "--get-regexp" not in args:
        return [list(args)] if "--get" in args else []
    known = NEW_GIT_SETTINGS | frozenset(
        {
            "git-pr.tags",
            "git-pr.draftPattern",
            "git-pr.githubStack",
            "git-pr.stackEmoji",
            "git-pr.reverse",
            "git-pr.defaultDraft",
            "git-pr.skipDraft",
            "git-pr.branchFromTitle",
            "user.name",
            "user.email",
        }
    )
    canonical = {key.lower(): key for key in known}
    pattern = args[-1]
    assert pattern.startswith("^(") and pattern.endswith(")$")
    expression = pattern[2:-2]
    branch_scope = r"branch\..*\.(pushremote|remote)"
    scoped_keys: list[str] = []
    if expression == branch_scope or expression.endswith("|" + branch_scope):
        expression = expression.removesuffix(branch_scope).removesuffix("|")
        scoped_keys = ["branch.*.pushremote", "branch.*.remote"]
    keys = [re.sub(r"\\(.)", r"\1", key) for key in expression.split("|")] if expression else []
    keys.extend(scoped_keys)
    assert keys and len(set(keys)) == len(keys) and all(key in canonical for key in keys)
    types = [arg for arg in args if arg.startswith("--type=")]
    return [["config", *types, "--get", canonical[key]] for key in keys]


def new_git_settings(event: Record) -> list[str]:
    """Identify declared settings in scalar and batched configuration reads."""
    return [args[-1] for args in configuration_reads(event) if args[-1] in NEW_GIT_SETTINGS]


def comparable_trace(trace: list[Record], *, reference: bool = False) -> collections.Counter[str]:
    """Retain publication identities and configuration effects across batching."""
    result = []
    for event in trace:
        if event["event"] != "start":
            continue
        program, args = event["program"], event["args"]
        if program == "git" and args[:1] == ["push"]:
            assert "-f" in args
            offset = args.index("origin")
            for spec in args[offset + 1 :]:
                oid, destination = spec.split(":", 1)
                result.append(["publish", "origin", oid[:8], destination])
        elif program == "git" and args[:1] == ["config"]:
            reads = configuration_reads(event)
            for logical in reads or [list(args)]:
                if logical[-1] in NEW_GIT_SETTINGS:
                    continue
                if reference:
                    logical[-1] = RENAMED_GIT_SETTINGS.get(logical[-1], logical[-1])
                if logical[-1] in {"git-pr.skipDraft", "git-pr.branchFromTitle"}:
                    logical = [arg for arg in logical if arg != "--type=bool"]
                result.append([program, *logical])
    return collections.Counter(json.dumps(e, ensure_ascii=False) for e in result)


def publication_contract(evidence: Record, *, reference: bool = False) -> collections.Counter[str]:
    """Permit only the redundant reference setting read under an explicit native flag."""
    contract = comparable_trace(evidence["trace"], reference=reference)
    setting = json.dumps(["git", "config", "--type=bool", "--get", "git-pr.githubStack"])
    for phase in evidence["phases"]:
        if "--github-stack" not in phase["args"]:
            continue
        reads = phase["github_stack_setting_reads"]
        if reference:
            assert reads in (0, 1)
            contract.subtract({setting: reads})
        else:
            assert reads == 0, "explicit native mode must not query its overridden Git setting"
    return contract


def comparable_output(output: Record) -> Record:
    """Remove exact fixture framing and declared progress-message differences."""
    result = dict(output)
    result["stdout"] = re.sub(
        r"(?m)^test tests::differential::differential_entrypoint has been running for over 60 seconds\n",
        "",
        result["stdout"],
    )
    result["stdout"] = result["stdout"].replace("Waiting for GitHub to sync...\n", "")
    result["stdout"] = re.sub(
        r"(?m)^((?:https://github\.com/|\[#\d+\]\(|[0-9a-f]{8} #\d+: )[^\n]+) \(updated\)$",
        r"\1",
        result["stdout"],
    )
    result["stdout"] = re.sub(
        r"(?m)^GitHub Stack unchanged:", "GitHub Stack updated:", result["stdout"]
    )
    return result


def declared_output(name: str, output: Record, args: Sequence[str], *, reference: bool) -> Record:
    """Translate enumerated native-protocol diagnostics, retaining unexpected text."""
    output = comparable_output(output)
    # These fixtures use default Git orientation; an explicit flag is the sole override.
    stack_up = True
    for arg in args:
        key, separator, value = arg.partition("=")
        if key in {"--stack-up", "--reverse"}:
            stack_up = not separator or value.lower() not in {"false", "f", "0"}
    if not reference and stack_up:
        # Only selected-commit rows move; row contents and all other output remain exact.
        output["stdout"] = re.sub(
            r"(?m)(?:^[0-9a-f]{8} (?:\([^\n]*\))? [^\n]+\n)+",
            lambda match: "".join(reversed(match[0].splitlines(keepends=True))),
            output["stdout"],
        )
    if name == "no-rewrite-backend":
        diagnostic = (
            "ERROR: neither jj nor git-branchless is available\n\n"
            "This tool requires either:\n"
            "  1. Jujutsu (jj) - install from https://martinvonz.github.io/jj/\n"
            "     OR\n"
            "  2. git-branchless - install from https://github.com/arxanas/git-branchless\n"
            "     Then run: git branchless init\n\n"
            "After installation, try again.\n"
        )
        phase = (
            ""
            if reference
            else (
                "commit preparation failed; inspect Git or Jujutsu status before retrying "
                "because local history may already have changed: "
            )
        )
        assert output["status"] == 1 and output["stderr"] == phase + diagnostic
        output["stderr"] = diagnostic
    if name == "api-failure":
        assert output["status"] == 2
        if reference:
            assert (
                output["stderr"]
                == 'ERROR: failed to call http request: (503 Service Unavailable) {"message": "API unavailable"}\n'
            )
            diagnostic = 'failed to call http request: https://api.github.com/repos/org/repo/pulls/101 503 Service Unavailable\n{"message": "API unavailable"}\n'
            assert output["stdout"].endswith(diagnostic)
            output["stdout"] = output["stdout"].removesuffix(diagnostic)
        else:
            assert output["stderr"] == (
                "PR content reconciliation failed after publication; pushed refs and completed PR changes are retained: ERROR: "
                "PR content recovery exhausted after 4 attempt(s); outstanding PRs: #101. "
                "Completed changes are retained; inspect these descriptions before retrying: "
                'GitHub POST https://api.github.com/graphql failed: HTTP 503 Service Unavailable from https://api.github.com/graphql: {"message": "API unavailable"}\n'
                "Hint: check GitHub service availability; verify whether any requested mutation completed before retrying\n"
            )
            assert "API unavailable" not in output["stdout"]
        output["stderr"] = "ERROR: API unavailable\n"
    if name == "native-missing":
        if reference:
            assert output["status"] == 0
            assert (
                output["stderr"]
                == "gh-stack extension not installed; skipping GitHub Stack.\n  install: gh extension install github/gh-stack (or pass --no-stack)\n"
            )
        else:
            assert output["status"] == 1
            assert (
                output["stderr"]
                == "native stack reconciliation failed after publication; pushed refs and completed PR changes are retained: "
                'GitHub POST https://api.github.com/repos/org/repo/stacks failed: HTTP 404 Not Found from https://api.github.com/repos/org/repo/stacks: {"message": "Native stacks unavailable"}\n'
                "Hint: check the repository or resource name, access permissions and server feature availability\n"
            )
            assert "Native stacks unavailable" not in output["stdout"]
        output["status"] = 1
        output["stderr"] = "native unavailable\n"
    if name in {"native-rebuild", "native-partial-unstack"}:
        old = "the existing GitHub stack #9 cannot be updated in place.\n  gh said: Cannot update stack: would remove #9\ngit-pr will dissolve and rebuild it as: topic/1 topic/2\n"
        new = "the existing GitHub stack #9 must be dissolved and rebuilt as: topic/1 topic/2\n"
        output["stderr"] = output["stderr"].replace(old, new)
        output["stdout"] = output["stdout"].replace(
            "native stack rebuilt:", "GitHub Stack updated:"
        )
        output["stderr"] = output["stderr"].replace(
            "ERROR: failed to rebuild GitHub stack: stack #9 could not be dissolved: some PRs are queued for merge or have\nauto-merge enabled, so GitHub kept them stacked. Let them land (or dequeue /\ndisable auto-merge), then re-run; or fix the stack with `gh stack modify`.\n",
            "stack #9 could not be dissolved: some PRs remain stacked\n",
        )
        if not reference and name == "native-partial-unstack" and output["status"] == 1:
            phase = "native stack reconciliation failed after publication; pushed refs and completed PR changes are retained: "
            expected = new + phase + "stack #9 could not be dissolved: some PRs remain stacked\n"
            assert output["stderr"] == expected
            output["stderr"] = new + "stack #9 could not be dissolved: some PRs remain stacked\n"
    if name == "native-rejected":
        if reference:
            assert (
                output["status"] == 1
                and "Your commits are already on the remote" in output["stderr"]
            )
            output["status"] = 0
            output["stderr"] = "\n"
            output["stdout"] += "GitHub Stack updated: topic/1 topic/2\n"
        else:
            assert output["status"] == 0 and output["stderr"] == "\n"
    return completed_listing(name, output, reference=reference)


def completed_listing(name: str, output: Record, *, reference: bool) -> Record:
    """Compare every result row independently of the corrected completion position."""
    output = dict(output)
    match = re.search(r"\nStack of (\d+) \((?:newest|oldest) at the top\):\n\n", output["stdout"])
    output["completed_listing"] = ""
    if match is None:
        return output
    end = match.end()
    for index in range(int(match[1])):
        if index:
            assert output["stdout"][end] == "\n"
            end += 1
        row_end = output["stdout"].index("\n", end)
        assert row_end > end
        end = row_end + 1
    listing = output["stdout"][match.start() : end]
    if output["status"]:
        assert reference and name in {"api-failure", "native-missing", "native-partial-unstack"}
    else:
        output["completed_listing"] = listing
    output["stdout"] = output["stdout"][: match.start()] + output["stdout"][end:]
    return output


def commit_objects(text: str) -> dict[str, list[str]]:
    """Retain every tree, parent and message without depending on equal-date ordering."""
    fields = text.split("\0")
    assert len(fields) % 4 == 1
    return {fields[i].lstrip("\n"): fields[i + 1 : i + 4] for i in range(0, len(fields) - 1, 4)}


def comparable_snapshot(value: Record) -> Record:
    """Canonicalise only the corrected separator before the managed footer."""
    value = json.loads(json.dumps(value))
    value["commits"] = commit_objects(value["commits"])
    for pr in value["prs"].values():
        pr["body"] = pr["body"].replace(
            "---\n\n<!-- git-pr-stack-start -->", "---\n<!-- git-pr-stack-start -->"
        )
    return value


def operation_budgets(candidate: Record) -> None:
    """Enforce ownership, bounded batches, and zero-effect repeat submissions."""
    for phase in candidate["phases"]:
        assert phase["native_child_pushes"] == 0
    for event in candidate["trace"]:
        if event["event"] == "start":
            assert event["program"] != "gh", "the application must own API effects"
            if event["program"] == "git" and event["args"][:1] == ["push"]:
                args = event["args"]
                assert args[:6] == [
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                    "-f",
                    "--",
                    "origin",
                ]
                assert 1 <= len(args[6:]) <= 128
                assert sum(len(spec.encode()) + 1 for spec in args[6:]) <= 64 * 1024
                assert all(
                    re.fullmatch(r"[0-9a-f]{40,64}:refs/heads/.+", spec) for spec in args[6:]
                )
        elif event["event"] == "http" and event["path"] == "/graphql":
            variables = event["body"]["variables"]
            count = sum(key.startswith(("h", "p", "b")) for key in variables)
            assert 1 <= count <= 25
            if event["body"]["query"].startswith("mutation"):
                assert count <= 5
                assert (
                    len(
                        json.dumps(
                            event["body"], ensure_ascii=False, separators=(",", ":")
                        ).encode()
                    )
                    <= 1024 * 1024
                )
    for index, (previous, current) in enumerate(
        zip(candidate["snapshot"], candidate["snapshot"][1:], strict=False)
    ):
        if previous == current:
            phase = candidate["phases"][index + 1]
            assert not any(
                phase["phases"].get(key, {}).get("operations", 0)
                for key in ("create", "base", "description", "labels")
            )
            assert phase["scheduled_wait_seconds"] == 0


def compare(
    name: str,
    reference: Record,
    candidate: Record,
    directory: pathlib.Path,
    *,
    rust_reference: bool = False,
) -> None:
    """Compare effects exactly, with an explicit ledger of efficiency changes."""
    if name in {"partial", "middle-descendants", "native-rebuild", "native-partial-unstack"}:
        compare_refused_publication(
            name, reference, candidate, directory, rust_reference=rust_reference
        )
        return
    if name == "branchless-rewrite":
        assert candidate["snapshot"] == [candidate["initial_snapshot"]]
        assert candidate["outputs"][0]["status"] == 0
        assert "Would insert Remote-Ref" in candidate["outputs"][0]["stdout"]
        assert not any(
            e["event"] == "http"
            or (e["event"] == "start" and e["args"][0] in {"reword", "describe", "push"})
            for e in candidate["trace"]
        )
        assert reference["snapshot"] != [reference["initial_snapshot"]]
        return
    differences = []
    if [
        declared_output(name, output, phase["args"], reference=True)
        for output, phase in zip(reference["outputs"], reference["phases"], strict=True)
    ] != [
        declared_output(name, output, phase["args"], reference=False)
        for output, phase in zip(candidate["outputs"], candidate["phases"], strict=True)
    ]:
        differences.append("output/status")
    reference_snapshots = [comparable_snapshot(o) for o in reference["snapshot"]]
    if name == "jj-workspace":
        # The excluded base remains description context despite having no PR.
        initial_refs = dict(
            line.split("|", 1) for line in reference["initial_snapshot"]["refs"].splitlines()
        )
        base = initial_refs["refs/heads/topic/1"]
        assert len(reference_snapshots) == 1
        assert reference_snapshots[0]["prs"].keys() == {"101"}
        pr = reference_snapshots[0]["prs"]["101"]
        start, end = "<!-- git-pr-stack-start -->", "<!-- git-pr-stack-end -->"
        row = re.search(r"\* #101 👈 This PR \([0-9a-f]{8}\)", pr["body"])
        assert row is not None
        assert pr["body"].count(start) == pr["body"].count(end) == 1
        context = (
            f"* [Feature 1 ({base[:8]})](https://github.com/org/repo/commit/{base})"
            " — fixture&#x200B;@example.com"
        )
        old_footer = f"{start}\n{row.group()}\n\n{end}"
        new_footer = (
            f"{start}\nThis PR is entry **2 of 2** in a stack (newest at the top)\n\n"
            f"{row.group()}\n{context}\n\n{end}"
        )
        if old_footer in pr["body"]:
            pr["body"] = pr["body"].replace(old_footer, new_footer, 1)
        else:
            assert rust_reference and new_footer in pr["body"]
    if name == "native-rejected":
        for snapshot in reference_snapshots:
            for pr in snapshot["prs"].values():
                assert "stack" not in pr
                pr["stack"] = {"number": 9}
    candidate_snapshots = [comparable_snapshot(o) for o in candidate["snapshot"]]
    retained = set(commit_objects(candidate["initial_snapshot"]["commits"]))
    for expected, actual in zip(reference_snapshots, candidate_snapshots, strict=True):
        # Backend transactions have different unpublished recovery objects. Preserve
        # every original object and every object ever reachable through user refs.
        for snapshot in (expected, actual):
            public_refs = [
                line
                for line in snapshot["refs"].splitlines()
                if not line.startswith(("refs/branchless/", "refs/jj/keep/"))
            ]
            roots = [line.split("|", 1)[1] for line in public_refs]
            roots.append(snapshot["checkout"])
            if snapshot["working_copy"]:
                roots.append(snapshot["working_copy"][-40:])
            pending = list(roots)
            visited = set()
            while pending:
                oid = pending.pop()
                if oid in visited:
                    continue
                visited.add(oid)
                assert oid in snapshot["commits"], "incomplete reachable history"
                pending.extend(snapshot["commits"][oid][1].split())
            retained.update(visited)
            snapshot["refs"] = "\n".join(public_refs)
        for snapshot in (expected, actual):
            assert retained <= snapshot["commits"].keys(), (
                "original or published recovery objects were lost"
            )
            snapshot["commits"] = {oid: snapshot["commits"][oid] for oid in retained}
    if reference_snapshots != candidate_snapshots:
        differences.append("repository/PR snapshot")
    if name == "invalid-range":
        assert not differences, f"{name}: {', '.join(differences)}"
        assert candidate["trace"] == [], "invalid range syntax must precede external discovery"
        assert candidate["snapshot"] == [candidate["initial_snapshot"]]
        return
    reference_contract = publication_contract(reference, reference=True)
    candidate_contract = publication_contract(candidate)
    if reference_contract != candidate_contract:
        differences.append("publication/configuration contract")
    operation_budgets(candidate)
    if name == "api-failure":
        requests = [
            event
            for event in candidate["trace"]
            if event["event"] == "http" and event["path"] == "/graphql"
        ]
        mutations = [
            index
            for index, event in enumerate(requests)
            if event["body"]["query"].startswith("mutation")
        ]
        assert len(mutations) == 4, "uncertain content recovery must have bounded retries"
        assert all(
            after > before + 1 for before, after in zip(mutations, mutations[1:], strict=False)
        ), "every uncertain write must be observed before another write is attempted"
    for expected, actual in zip(reference["phases"], candidate["phases"], strict=True):
        new_settings = dict.fromkeys(NEW_GIT_SETTINGS, 1)
        if any(arg.partition("=")[0] in {"--stack-up", "--reverse"} for arg in actual["args"]):
            del new_settings["git-pr.stackUp"]
        reference_settings = (
            dict.fromkeys(NEW_GIT_SETTINGS - FOLLOWUP_GIT_SETTINGS, 1) if rust_reference else {}
        )
        if (
            expected["new_git_settings"] != reference_settings
            or actual["new_git_settings"] != new_settings
        ):
            differences.append("declared Git configuration reads")
    if differences:
        failure = {
            "scenario": name,
            "differences": differences,
            "reference_only": list((reference_contract - candidate_contract).elements()),
            "candidate_only": list((candidate_contract - reference_contract).elements()),
        }
        (directory / "difference.json").write_text(json.dumps(failure, indent=2))
        raise AssertionError(f"{name}: {', '.join(differences)}; see {directory}")


def compare_refused_publication(
    name: str,
    reference: Record,
    candidate: Record,
    directory: pathlib.Path,
    *,
    rust_reference: bool,
) -> None:
    """Require exact safe-prefix parity and zero effects for newly rejected transitions."""
    prefix = 2 if name == "partial" else 1
    assert len(candidate["outputs"]) == len(reference["outputs"]) == (4 if name == "partial" else 2)
    expected = json.loads(json.dumps(candidate["snapshot"][prefix - 1]))
    if name == "middle-descendants":
        expected["branch"] = "topic/2"
        expected["checkout"] = dict(line.split("|", 1) for line in expected["refs"].splitlines())[
            "refs/heads/topic/2"
        ]
    elif name != "partial":
        for pr in expected["prs"].values():
            pr["stack"] = {"number": 9}
            pr["base"]["ref"] = "stale"
    for index in range(prefix, len(candidate["outputs"])):
        output = candidate["outputs"][index]
        assert output["status"] == 2
        assert (
            "publication safety preflight failed; no local rewrites, pushes or PR changes were performed"
            in output["stderr"]
        )
        assert "unsafe publication:" in output["stderr"]
        assert "Stack of" not in output["stdout"] and "push -f" not in output["stdout"]
        assert candidate["snapshot"][index] == expected
        phase = candidate["phases"][index]
        assert phase["scheduled_wait_seconds"] == 0
        for event in candidate["trace"][phase["trace_start"] : phase["trace_end"]]:
            if event["event"] == "http":
                assert event["path"] == "/graphql" and event["body"]["query"].startswith("query")
            if event["event"] == "start":
                assert event["program"] != "gh"
                assert event["args"][0] not in {"push", "reword", "describe", "update-ref"}
                if event["args"][0] == "config":
                    assert any(arg.startswith("--get") for arg in event["args"])
        assert reference["phases"][index]["phases"]["push"]["operations"] > 0

    def safe_prefix(evidence: Record) -> Record:
        result = dict(evidence)
        for key in ("outputs", "snapshot", "phases"):
            result[key] = result[key][:prefix]
        result["trace"] = result["trace"][: result["phases"][-1]["trace_end"]]
        return result

    compare(
        name + "-safe-prefix",
        safe_prefix(reference),
        safe_prefix(candidate),
        directory,
        rust_reference=rust_reference,
    )
    operation_budgets(candidate)


def cli_parity(release: bool = False) -> int:
    """Compare parser statuses and terminal streams with declared help corrections."""
    cases = [
        ["--help"],
        ["-h"],
        ["--unknown"],
        ["--draft=YES"],
        ["--timeout"],
        ["--timeout=bad"],
        ["---draft"],
        ["-=x"],
        ["--stop-after=invalid"],
        ["--timeout=1__2"],
        ["--timeout=_1"],
        ["--timeout=1_"],
        ["--timeout=9223372036854775808"],
        ["--timeout=170141183460469231731687303715884105728"],
    ]
    for case in cases:
        results = []
        target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
        for executable in [
            WORK / "reference-bin",
            target / ("release" if release else "debug") / "git-pr",
        ]:
            with tempfile.TemporaryDirectory() as temp:
                env = fixture_environment()
                env.update(HOME=temp, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
                result = subprocess.run(
                    [str(executable), *case],
                    cwd=temp,
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                stderr = result.stderr
                if executable == WORK / "reference-bin":
                    stderr = ANSI.sub("", stderr)
                else:
                    assert "\x1b" not in stderr, (
                        "redirected diagnostics must not contain terminal controls"
                    )
                results.append(
                    (
                        result.returncode,
                        result.stdout.replace(ADDED_HELP, "")
                        .replace(STACK_UP_HELP, LEGACY_REVERSE_HELP)
                        .replace(BRANCH_HELP, LEGACY_BRANCH_HELP)
                        .replace(VERBOSE_HELP, LEGACY_VERBOSE_HELP),
                        stderr.replace(ADDED_HELP, "")
                        .replace(STACK_UP_HELP, LEGACY_REVERSE_HELP)
                        .replace(BRANCH_HELP, LEGACY_BRANCH_HELP)
                        .replace(VERBOSE_HELP, LEGACY_VERBOSE_HELP),
                    )
                )
        if results[0] != results[1]:
            raise AssertionError(f"CLI {case}: {results}")
    return len(cases)


def identity(binary: pathlib.Path) -> Record:
    """Identify the application sources, executable and measured host environment."""
    sources = [
        ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs")
    ]
    sources.extend(path for path in (ROOT / "src").rglob("*") if path.is_file())
    digest = hashlib.sha256()
    for path in sorted(sources):
        digest.update(str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes())
    with binary.open("rb") as executable:
        executable_digest = hashlib.file_digest(executable, "sha256").hexdigest()
    return {
        "application_sha256": digest.hexdigest(),
        "harness_sha256": {
            name: hashlib.sha256((ROOT / "scripts" / name).read_bytes()).hexdigest()
            for name in ("parity.py", "fixture_command.py")
        },
        "fixture_executable_sha256": executable_digest,
        "platform": platform.platform(),
        "architecture": platform.machine(),
        "compiler": command(["rustc", "--version"]),
        "git": command([REAL_GIT, "--version"]),
        "python": platform.python_version(),
    }


def build_rust_reference(revision: str) -> tuple[pathlib.Path, Record]:
    """Build an immutable previous Rust tree without changing the developer checkout."""
    revision = command([REAL_GIT, "rev-parse", "--verify", revision + "^{commit}"])
    source = WORK / "rust-reference" / revision
    source.mkdir(parents=True, exist_ok=True)
    if not (source / "Cargo.toml").exists():
        data = subprocess.check_output([REAL_GIT, "archive", revision], cwd=ROOT)
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            archive.extractall(source, filter="data")
    output = command(
        ["cargo", "test", "--release", "--locked", "--lib", "--no-run", "--message-format=json"],
        source,
        timeout=1800,
    )
    executables = [
        entry["executable"]
        for line in output.splitlines()
        if (entry := json.loads(line)).get("executable") and entry.get("profile", {}).get("test")
    ]
    if len(executables) != 1:
        raise AssertionError("Expected one baseline library test executable")
    binary = pathlib.Path(executables[0])
    with binary.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return binary, {
        "revision": revision,
        "tree": command([REAL_GIT, "rev-parse", revision + "^{tree}"]),
        "fixture_executable_sha256": digest,
        "profile": "release",
        "wait_instrumentation": "baseline clock unchanged; elapsed duration includes waits",
    }


def main() -> None:
    """Run bounded comparisons and retain raw samples alongside median and tail timings."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--bench", action="store_true")
    parser.add_argument(
        "--rust-reference", help="Pinned Rust revision to build in an isolated archive"
    )
    parser.add_argument("--sizes", type=int, nargs="+", default=[1, 5, 10])
    parser.add_argument("--scenario")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--latency-ms", type=int, choices=[0, 20, 100, 300], default=0)
    args = parser.parse_args()
    if args.rust_reference and not args.bench:
        parser.error("--rust-reference requires --bench")
    if not args.sizes or min(args.sizes) < 1:
        parser.error("--sizes must be positive")
    if args.jobs < 1 or args.repetitions < 1:
        parser.error("--jobs and --repetitions must be positive")
    base_prefix = (
        "bench"
        if args.bench
        else ("coverage-parity" if os.environ.get("CARGO_LLVM_COV") else "parity")
    )
    prefix = f"{base_prefix}-latency-{args.latency_ms}" if args.latency_ms else base_prefix
    binary_cache = WORK / f"{base_prefix}-test-binary"
    binary = (
        build(args.bench) if not args.no_build else pathlib.Path(binary_cache.read_text().strip())
    )
    binary_cache.write_text(str(binary))
    count = cli_parity(args.bench)
    baseline = None
    if args.rust_reference:
        baseline, baseline_identity = build_rust_reference(args.rust_reference)
        prefix += "-rust-" + baseline_identity["revision"][:8]
    scenarios = {
        "first-repeat": {
            "runs": [
                [],
                [],
                ["--draft=false", "--add-label", "all", "--add-tip-label", "tip"],
            ]
        },
        "partial": {
            "size": 3,
            "runs": [
                [],
                ["topic/1..topic/3"],
                {"args": ["topic/3", "topic/1"], "candidate_status": 2},
                {"args": ["topic/2"], "candidate_status": 2},
            ],
        },
        "dry-run": {"args": ["--dry-run"]},
        "native-enabled": {"args": ["--github-stack", "--yes"]},
        "native-unattended": {"args": ["--github-stack"], "reference_accept_native": True},
        "native-piped": {"args": ["--github-stack"], "pipe": True, "reference_accept_native": True},
        "native-missing": {
            "args": ["--github-stack", "--yes"],
            "stack_error": 'unknown command "stack" for "gh"',
            "candidate_status": 1,
        },
        "native-rebuild": {
            "runs": [
                [],
                {
                    "args": ["--github-stack", "--yes"],
                    "stacked": True,
                    "candidate_status": 2,
                    "stack_errors": ["Cannot update stack: would remove #9"],
                },
            ]
        },
        "native-partial-unstack": {
            "runs": [
                [],
                {
                    "args": ["--github-stack", "--yes"],
                    "stacked": True,
                    "candidate_status": 2,
                    "status": 1,
                    "stack_errors": [
                        "Cannot update stack: would remove #9",
                        "Some pull requests remain stacked on GitHub",
                    ],
                },
            ]
        },
        "native-rejected": {
            "status": 1,
            "args": ["--github-stack", "--yes"],
            "stack_error": "! [rejected] topic/2 (non-fast-forward)",
        },
        "api-failure": {"size": 1, "api_error": True, "status": 2},
        "no-rewrite-backend": {"size": 1, "rewrite": True, "status": 1},
        "invalid-range": {"args": ["main...HEAD"], "status": 1},
        "display-formats": {
            "size": 1,
            "runs": [
                ["--output=url"],
                ["--output=markdown"],
                ["--output={shorthash} #{number}: {title}", "--stack-up=false"],
            ],
        },
        "middle-descendants": {
            "size": 3,
            "runs": [[], {"args": [], "checkout": "topic/2", "candidate_status": 2}],
        },
        "changed-branchless": {
            "backend": "branchless",
            "rewrite": True,
            "runs": [[], {"args": [], "amend": True}, []],
        },
        "jj-rewrite": {"backend": "jj", "rewrite": True},
        "jj-workspace": {"backend": "jj-workspace", "rewrite": True, "args": ["topic/1..topic/2"]},
        "branchless-rewrite": {
            "backend": "branchless",
            "rewrite": True,
            "args": ["--dry-run"],
        },
    }
    for phase in ["validate", "get-commits", "rewrite", "push", "pr-create"]:
        scenarios["stop-" + phase] = {"args": ["--stop-after", phase]}
    if args.bench:
        scenarios = {
            f"{backend}-{size}-{stack}-sample{sample}": {
                "backend": backend,
                "size": size,
                "rewrite": True,
                "workload": f"{backend}-{size}-{stack}",
                "runs": [
                    {"args": ["--github-stack", "--yes"] if stack else []},
                    {"args": ["--github-stack", "--yes"] if stack else [], "amend": True},
                    {"args": ["--github-stack", "--yes"] if stack else []},
                ],
            }
            for backend in ["branchless", "jj", "jj-workspace"]
            for size in args.sizes
            for stack in [False, True]
            for sample in range(args.repetitions)
        }
    if args.scenario:
        scenarios = {
            name: scenario
            for name, scenario in scenarios.items()
            if re.fullmatch(args.scenario, name)
        }
        if not scenarios:
            raise ValueError(f"Unknown scenario: {args.scenario}")

    for scenario in scenarios.values():
        scenario["latency_ms"] = args.latency_ms
        scenario["submission_timeout"] = max(90, scenario.get("size", 2) * 8)
        if baseline is not None:
            scenario["rust_reference"] = str(baseline)

    def evaluate(item: tuple[str, Record]) -> Record:
        name, scenario = item
        directory = WORK / prefix / name
        if directory.exists():
            previous = WORK / "history" / f"{prefix}-{time.time_ns()}" / name
            previous.parent.mkdir(parents=True)
            shutil.move(directory, previous)
        directory.mkdir(parents=True)
        order = ["reference", "candidate"]
        if args.bench and int(name.rsplit("sample", 1)[1]) % 2:
            order.reverse()
        results = {kind: run_one(kind, binary, scenario, directory / kind) for kind in order}
        compare(
            name,
            results["reference"],
            results["candidate"],
            directory,
            rust_reference=baseline is not None,
        )
        print(f"PASS {name}", flush=True)
        return {
            "scenario": name,
            "workload": scenario.get("workload", name),
            "order": order,
            **{
                kind: {
                    "seconds": evidence["seconds"],
                    "phases": evidence["phases"],
                    "fixture_seconds": evidence["fixture_seconds"],
                }
                for kind, evidence in results.items()
            },
        }

    summaries = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = [executor.submit(evaluate, item) for item in scenarios.items()]
        try:
            for future in concurrent.futures.as_completed(futures):
                summaries.append(future.result())
        except BaseException:
            for future in futures:
                future.cancel()
            raise
    statistics_by_workload = {}
    for workload in sorted({sample["workload"] for sample in summaries}):
        samples = [sample for sample in summaries if sample["workload"] == workload]
        statistics_by_workload[workload] = {
            kind: [
                {
                    "median": statistics.median(
                        sample[kind]["seconds"][index] for sample in samples
                    ),
                    "tail": max(sample[kind]["seconds"][index] for sample in samples),
                    "samples": [sample[kind]["seconds"][index] for sample in samples],
                }
                for index in range(len(samples[0][kind]["seconds"]))
            ]
            for kind in ["reference", "candidate"]
        }
    report = {
        "reference": baseline_identity if baseline is not None else REVISION,
        "candidate": identity(binary),
        "cli_cases": count,
        "configuration_contract": {
            "additional_git_reads": sorted(NEW_GIT_SETTINGS),
            "renamed_git_reads": RENAMED_GIT_SETTINGS,
            "git_boolean_conversion": ["git-pr.skipDraft", "git-pr.branchFromTitle"],
            "omitted_overridden_read": "git-pr.githubStack under explicit --github-stack",
        },
        "jobs": args.jobs,
        "latency_ms": args.latency_ms,
        "profile": "release" if args.bench else "debug",
        "scenarios": sorted(summaries, key=lambda item: item["scenario"]),
        "statistics": statistics_by_workload,
    }
    (WORK / f"{prefix}.json").write_text(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
