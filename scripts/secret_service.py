#!/usr/bin/env -S uv run
"""Exercise Linux credential storage in an isolated, disposable D-Bus session."""

import os
import pathlib
import subprocess
import sys
import tempfile


def main() -> None:
    """Create a private login collection and remove it when the session exits."""
    if sys.platform != "linux":
        raise SystemExit("The Secret Service acceptance job requires Linux")
    arguments = sys.argv[1:]
    deny_search = arguments[:1] == ["--deny-search"]
    command = (arguments[1:] if deny_search else arguments) or [
        "cargo",
        "test",
        "--locked",
        "linux_secret_service",
        "--",
        "--ignored",
    ]
    if command[0] == "--session":
        subprocess.run(
            ["gnome-keyring-daemon", "--unlock", "--components=secrets"],
            input=b"disposable-fixture-password",
            check=True,
            capture_output=True,
            timeout=30,
        )
        result = subprocess.run(command[1:], check=False, timeout=600)
        raise SystemExit(result.returncode)
    with tempfile.TemporaryDirectory(prefix="git-pr-secret-service-") as temporary:
        environment = dict(os.environ)
        environment["CARGO_HOME"] = os.environ.get(
            "CARGO_HOME", str(pathlib.Path.home() / ".cargo")
        )
        environment["RUSTUP_HOME"] = os.environ.get(
            "RUSTUP_HOME", str(pathlib.Path.home() / ".rustup")
        )
        environment.update(
            HOME=temporary,
            XDG_DATA_HOME=temporary + "/data",
            XDG_CONFIG_HOME=temporary + "/config",
            XDG_RUNTIME_DIR=temporary + "/run",
            GIT_PR_TEST_SECRET_SERVICE="1",
        )
        pathlib.Path(environment["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        session = ["dbus-run-session"]
        if deny_search:
            configuration = pathlib.Path(temporary) / "denied-search.conf"
            configuration.write_text(
                "<busconfig>\n"
                "  <include>/usr/share/dbus-1/session.conf</include>\n"
                '  <policy context="mandatory">\n'
                '    <deny send_destination="org.freedesktop.secrets"\n'
                '          send_interface="org.freedesktop.Secret.Collection"\n'
                '          send_member="SearchItems"/>\n'
                "  </policy>\n"
                "</busconfig>\n"
            )
            session.extend(["--config-file", str(configuration)])
        result = subprocess.run(
            [*session, "--", sys.executable, __file__, "--session", *command],
            env=environment,
            check=False,
            timeout=650,
        )
        raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
