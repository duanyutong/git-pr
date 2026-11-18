#!/usr/bin/env -S uv run
"""Follow fresh mutation evidence outside the buffering hook runner."""

import dataclasses
import json
import os
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
CAMPAIGN = ROOT / "target/validation/assurance/mutants"
type Snapshot = tuple[int, int, int, int]


def snapshot(path: pathlib.Path) -> Snapshot | None:
    try:
        info = path.stat()
    except FileNotFoundError:
        return None
    return info.st_dev, info.st_ino, info.st_mtime_ns, info.st_size


@dataclasses.dataclass
class LogFollower:
    """Track one invocation without replaying restored campaign output."""

    path: pathlib.Path
    initial: Snapshot | None = dataclasses.field(init=False)
    identity: tuple[int, int] | None = None
    position: int = 0
    started: bool = False

    def __post_init__(self) -> None:
        self.initial = snapshot(self.path)

    def read(self) -> bytes:
        try:
            with self.path.open("rb") as stream:
                info = os.fstat(stream.fileno())
                current = info.st_dev, info.st_ino, info.st_mtime_ns, info.st_size
                if not self.started and current == self.initial:
                    return b""
                self.started = True
                if current[:2] != self.identity or info.st_size < self.position:
                    self.position = 0
                self.identity = current[:2]
                stream.seek(self.position)
                data = stream.read()
                self.position = stream.tell()
                return data
        except FileNotFoundError:
            return b""


def completed(directory: pathlib.Path, initial: tuple[Snapshot | None, ...]) -> str:
    """Use only new, complete progress records; the gate validates final outcomes."""
    paths = [directory / "mutants.json", directory / "outcomes.json"]
    if any(snapshot(path) == previous for path, previous in zip(paths, initial, strict=True)):
        return "running"
    try:
        inventory, outcomes = [json.loads(path.read_text()) for path in paths]
        if not isinstance(inventory, list) or not isinstance(outcomes, dict):
            return "running"
        records = outcomes.get("outcomes")
        if not isinstance(records, list):
            return "running"
        count = sum(
            isinstance(item, dict) and isinstance(item.get("scenario"), dict) for item in records
        )
        return f"{count}/{len(inventory)} mutations completed"
    except FileNotFoundError, UnicodeDecodeError, json.JSONDecodeError:
        return "running"


def follow(
    log: pathlib.Path, ready: pathlib.Path, *, interval: float = 0.1, heartbeat: float = 30
) -> None:
    follower = LogFollower(log)
    directory = log.parent / "mutants.out"
    initial = tuple(snapshot(directory / name) for name in ("mutants.json", "outcomes.json"))
    started = time.monotonic()
    next_heartbeat = started + heartbeat
    ready.touch()
    print("Mutation analysis: waiting for current campaign output", flush=True)
    while True:
        data = follower.read()
        if data:
            sys.stdout.buffer.write(data)
            sys.stdout.buffer.flush()
        now = time.monotonic()
        if now >= next_heartbeat:
            status = completed(directory, initial) if follower.started else "running"
            print(f"Mutation analysis: {status} ({now - started:.0f}s elapsed)", flush=True)
            next_heartbeat = now + heartbeat
        time.sleep(interval)


if __name__ == "__main__":
    follow(CAMPAIGN / "campaign.log", pathlib.Path(sys.argv[1]))
