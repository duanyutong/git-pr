#!/usr/bin/env -S uv run
"""Record command duration and exit status without concealing failures."""

import json
import pathlib
import subprocess
import sys
import time

phase, *command = sys.argv[1:]
started = time.monotonic()
result = subprocess.run(command, check=False)
record = {
    "phase": phase,
    "seconds": time.monotonic() - started,
    "status": result.returncode,
}
path = pathlib.Path("target/validation/timings.jsonl")
path.parent.mkdir(parents=True, exist_ok=True)
with path.open("a") as stream:
    stream.write(json.dumps(record) + "\n")
print(json.dumps(record), flush=True)
sys.exit(result.returncode)
