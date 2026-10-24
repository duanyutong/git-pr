#!/usr/bin/env python3
"""Controlled process boundary for disposable compatibility fixtures."""

import json
import os
import pathlib
import subprocess
import sys
import urllib.request
from typing import Any

program = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
endpoint = os.environ["GIT_PR_FIXTURE_ENDPOINT"]


def send(value: dict[str, Any]) -> dict[str, Any]:
    data = json.dumps(value).encode()
    request = urllib.request.Request(
        endpoint + "/fixture/command",
        data=data,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


response = send({"program": program, "args": args, "event": "start"})
if "status" in response:
    output = response.get("output", "")
    status = response["status"]
else:
    execution_args = list(args)
    if program == "git" and args[:4] == [
        "ls-remote",
        "--refs",
        "--",
        "https://github.com/org/repo.git",
    ]:
        execution_args[3] = "origin"
    result = subprocess.run(
        [os.environ["GIT_PR_REAL_" + program.upper()], *execution_args],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    output = result.stdout.decode(errors="replace")
    status = result.returncode
send(
    {
        "program": program,
        "args": args,
        "event": "complete",
        "status": status,
        "output": output,
    }
)
sys.stdout.write(output)
sys.exit(status)
