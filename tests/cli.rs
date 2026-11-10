// Subprocess fixture failures shall fail the test immediately.
#![allow(clippy::unwrap_used)]

use std::{os::unix::fs::PermissionsExt as _, process::Command};

fn run(args: &[&str]) -> std::process::Output {
    let temp = tempfile::tempdir().unwrap();
    command(&temp).args(args).output().unwrap()
}

fn command(temp: &tempfile::TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_git-pr"));
    command
        .current_dir(temp.path())
        .env_clear()
        .envs(std::env::vars_os().filter(|(key, _)| {
            matches!(
                key.to_str(),
                Some(
                    "PATH"
                        | "TMPDIR"
                        | "LLVM_PROFILE_FILE"
                        | "DYLD_LIBRARY_PATH"
                        | "DYLD_FALLBACK_LIBRARY_PATH"
                        | "LD_LIBRARY_PATH"
                )
            )
        }))
        .env("HOME", temp.path())
        .env("XDG_STATE_HOME", temp.path().join("state"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_PR_STOP_AFTER", "")
        .env("GIT_PR_OUTPUT", "");
    command
}

fn invocation_metadata(temp: &tempfile::TempDir) -> Vec<(std::path::PathBuf, serde_json::Value)> {
    let directory = temp.path().join("state/git-pr/logs");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let metadata = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            (path.with_extension("log"), metadata)
        })
        .collect()
}

#[test]
fn ordinary_failures_save_verbose_diagnostics_without_printing_them() {
    let temp = tempfile::tempdir().unwrap();
    let result = command(&temp).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(!stderr.contains("Timing summary") && !stderr.contains("start command:"));
    let logs = invocation_metadata(&temp);
    assert_eq!(logs.len(), 1);
    let (path, metadata) = &logs[0];
    assert_eq!(metadata["outcome"], "exit 1");
    assert!(metadata["commits"].is_null());
    assert!(metadata["seconds"].as_f64().unwrap() > 0.0);
    let log = std::fs::read_to_string(path).unwrap();
    assert!(log.contains("start phase: configuration and repository discovery"));
    assert!(log.contains("start command: git rev-parse"));
    assert!(log.contains("Timing summary: failed"));
    assert!(log.contains("You need to run it in a git repository"));
    let result = command(&temp).arg("--logs").output().unwrap();
    assert!(result.status.success());
    let listing = String::from_utf8(result.stdout).unwrap();
    assert!(listing.contains("TIMESTAMP (UTC)") && listing.contains("REPOSITORY"));
    assert!(listing.contains(path.to_str().unwrap()));
    assert_eq!(invocation_metadata(&temp).len(), 1);
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn logs_list_recent_invocations_newest_first_and_all_when_requested() {
    let temp = tempfile::tempdir().unwrap();
    let empty = command(&temp).arg("--logs").output().unwrap();
    assert!(empty.status.success());
    assert_eq!(empty.stdout, b"No invocation logs recorded.\n");
    assert!(!temp.path().join("state").exists());
    for index in 0..23 {
        let result = command(&temp)
            .arg(if index == 0 {
                "--help"
            } else if index == 1 {
                "--unknown"
            } else {
                "--version"
            })
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(if index == 1 { 2 } else { 0 }));
    }
    let logs = invocation_metadata(&temp);
    assert_eq!(logs.len(), 23);
    assert_eq!(logs[1].1["outcome"], "exit 2");
    let recent = command(&temp).arg("--logs").output().unwrap();
    let recent = String::from_utf8(recent.stdout).unwrap();
    assert!(!recent.contains(logs[2].0.to_str().unwrap()));
    assert!(recent.contains(logs[22].0.to_str().unwrap()));
    assert!(recent.contains("--logs=all"));
    let all = command(&temp).arg("--logs=all").output().unwrap();
    assert!(all.status.success());
    let all = String::from_utf8(all.stdout).unwrap();
    let positions: Vec<_> = logs
        .iter()
        .map(|(path, _)| all.find(path.to_str().unwrap()).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] > pair[1]));
    assert_eq!(invocation_metadata(&temp).len(), 23);
}

#[test]
fn concurrent_invocations_keep_distinct_complete_logs() {
    let temp = tempfile::tempdir().unwrap();
    let children: Vec<_> = (0..8)
        .map(|_| {
            command(&temp)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let logs = invocation_metadata(&temp);
    assert_eq!(logs.len(), 8);
    for (path, metadata) in logs {
        assert_eq!(metadata["outcome"], "ok");
        let log = std::fs::read_to_string(path).unwrap();
        assert_eq!(log.matches("Invocation finished: ok").count(), 1);
    }
}

#[test]
fn default_log_location_is_available_without_an_xdg_override() {
    let temp = tempfile::tempdir().unwrap();
    let result = command(&temp)
        .env_remove("XDG_STATE_HOME")
        .arg("--version")
        .output()
        .unwrap();
    assert!(result.status.success());
    let expected = temp.path().join(if cfg!(target_os = "macos") {
        "Library/Logs/git-pr"
    } else {
        ".local/state/git-pr/logs"
    });
    assert_eq!(std::fs::read_dir(&expected).unwrap().count(), 2);
    let result = command(&temp)
        .env("XDG_STATE_HOME", "relative-state")
        .arg("--logs")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains(expected.to_str().unwrap())
    );
    assert!(!temp.path().join("relative-state").exists());
}

#[test]
fn unavailable_log_storage_warns_without_preventing_the_invocation() {
    let temp = tempfile::tempdir().unwrap();
    let blocked = temp.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    let result = command(&temp)
        .env("XDG_STATE_HOME", &blocked)
        .arg("--version")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .starts_with("git-pr ")
    );
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("invocation diagnostics could not be saved")
    );
}

#[test]
fn non_unicode_process_inputs_report_errors_without_panicking() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let temp = tempfile::tempdir().unwrap();
    let invalid = OsString::from_vec(vec![0xff]);
    let result = command(&temp).arg(&invalid).output().unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        result.stderr,
        b"ERROR: command-line arguments must be valid UTF-8\n"
    );
    let result = command(&temp)
        .env("UNRELATED_BINARY", &invalid)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("You need to run it in a git repository")
    );
    for key in ["GIT_PR_OUTPUT", "HOME"] {
        let result = command(&temp).env(key, &invalid).output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(result.stderr).unwrap(),
            format!("ERROR: environment variable {key} must be valid UTF-8\n")
        );
    }
}

#[test]
fn help_version_and_parser_failure_streams() {
    for flag in ["-h", "--help", "-help=false"] {
        let result = run(&[flag]);
        assert!(result.status.success());
        assert_eq!(result.stdout, include_bytes!("../src/cli/usage.txt"));
        assert_eq!(result.stderr, include_bytes!("../src/cli/flags.txt"));
    }
    let result = run(&["--version"]);
    assert!(result.status.success());
    let version = String::from_utf8(result.stdout).unwrap();
    assert!(version.starts_with(concat!(
        "git-pr version ",
        env!("CARGO_PKG_VERSION"),
        " (git "
    )));
    for field in [", dirty=", ", target=", ", rustc=", ", built="] {
        assert!(version.contains(field), "{version}");
    }
    assert_eq!(result.stderr, b"");
    let result = run(&["--unknown"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .starts_with("flag provided but not defined: -unknown\n")
    );
    let result = run(&[]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("You need to run it in a git repository")
    );
    let result = run(&["--stop-after=wrong"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("invalid --stop-after")
    );
}

#[test]
fn unavailable_commands_and_directories_retain_actionable_os_errors() {
    let temp = tempfile::tempdir().unwrap();
    let output = command(&temp).env("PATH", temp.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("could not start") && error.contains("git") && error.contains("installed"),
        "{error}"
    );
    assert!(!error.contains("exit code 199") && !error.contains("panicked"));

    // Delete only this disposable directory after the shell has entered it.
    let deleted = temp.path().join("deleted");
    std::fs::create_dir(&deleted).unwrap();
    let configuration = command(&temp);
    let mut shell = Command::new("/bin/sh");
    shell.current_dir(&deleted).env_clear().envs(
        configuration
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value))),
    );
    let output = shell
        .args([
            "-c",
            "rmdir \"$PWD\"; exec \"$1\"",
            "fixture",
            env!("CARGO_BIN_EXE_git-pr"),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("current working directory") && error.contains("existing, accessible"),
        "{error}"
    );
}

#[test]
fn broken_output_streams_produce_failure_status_without_panicking() {
    use std::{io::pipe, process::Stdio};
    let temp = tempfile::tempdir().unwrap();
    let (reader, writer) = pipe().unwrap();
    drop(reader);
    let output = command(&temp)
        .arg("--help")
        .stdout(Stdio::from(writer))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("could not write stdout") && !error.contains("panicked"));

    let (reader, writer) = pipe().unwrap();
    drop(reader);
    let output = command(&temp)
        .arg("--help")
        .stderr(Stdio::from(writer))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_ne!(output.stdout, [] as [u8; 0]);
}

#[test]
fn native_literal_draft_matching_exercises_fallback_and_stops_before_submission() {
    let temp = tempfile::tempdir().unwrap();
    let git = |arguments: &[&str]| {
        let environment = command(&temp);
        let output = Command::new("git")
            .current_dir(temp.path())
            .env_clear()
            .envs(
                environment
                    .get_envs()
                    .filter_map(|(key, value)| value.map(|value| (key, value))),
            )
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Fixture Author"]);
    git(&["config", "user.email", "fixture@example.com"]);
    std::fs::write(temp.path().join("file"), "base").unwrap();
    git(&["add", "file"]);
    git(&["commit", "-q", "-m", "Base"]);
    git(&["remote", "add", "origin", "https://github.com/org/repo.git"]);
    git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    git(&["switch", "-q", "-c", "dyt/fixture"]);
    std::fs::write(temp.path().join("file"), "changed").unwrap();
    git(&["add", "file"]);
    git(&["commit", "-q", "-m", "aaaaaaaaac"]);
    let output = command(&temp)
        .args([
            "--no-stack",
            "--skip-draft",
            "--draft=false",
            "--draft-pattern=*aaaab*,*aac*",
            "--stop-after=get-commits",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("skip draft \"aaaaaaaaac\"")
    );
    let logs = invocation_metadata(&temp);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].1["repository"], "org/repo");
    assert_eq!(logs[0].1["commits"], 0);
    assert_eq!(logs[0].1["outcome"], "ok");
}

#[test]
fn verbose_failure_reports_discovery_timings_on_stderr() {
    for flag in ["-v", "-vv"] {
        let result = run(&[flag]);
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(result.stdout, b"");
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(stderr.contains("start phase: configuration and repository discovery"));
        assert!(stderr.contains("start command: git rev-parse"));
        assert!(stderr.contains("Timing summary: failed"));
        assert!(stderr.contains("You need to run it in a git repository"));
    }
}

#[test]
fn invalid_hyperlinks_fail_before_repository_discovery() {
    for args in [
        vec!["--hyperlinks"],
        vec!["--hyperlinks="],
        vec!["--hyperlinks=wrong"],
    ] {
        let result = run(&args);
        assert_eq!(result.status.code(), Some(2));
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(error.contains("hyperlinks"));
        assert!(!error.contains("could not discover"));
    }
}
