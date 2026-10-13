//! Native process protocols are exercised only in disposable directories.
use super::{ProcessRunner, wait};
use crate::commands::CommandRunner;
use std::os::unix::process::CommandExt as _;
use std::{
    collections::BTreeMap,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn binary_input_and_diagnostics_use_separate_streams() {
    let directory = tempfile::tempdir().unwrap();
    let output = ProcessRunner::default()
        .execute_data(
            "sh",
            &["-c".into(), "cat; printf diagnostic >&2".into()],
            directory.path(),
            &BTreeMap::new(),
            b"a\0b\xff",
        )
        .unwrap();
    assert_eq!(output.stdout, b"a\0b\xff");
    assert_eq!(output.stderr, b"diagnostic");
    assert_eq!(output.status, 0);
}

#[test]
fn expired_commands_are_reaped_and_oversized_observations_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new("sh");
    command
        .args(["-c", "sleep 30 & wait"])
        .current_dir(directory.path())
        .process_group(0);
    let started = Instant::now();
    assert!(
        wait(
            &mut command,
            Duration::from_millis(50),
            &std::sync::atomic::AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("deadline")
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    let error = ProcessRunner::default()
        .execute(
            "sh",
            &["-c".into(), "head -c 70000000 /dev/zero; sleep 30".into()],
            directory.path(),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("64 MiB"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn cancellation_child_fixture() {
    let Some(directory) = std::env::var_os("GIT_PR_CANCEL_FIXTURE") else {
        return;
    };
    ProcessRunner::default()
        .execute(
            "sh",
            &[
                "-c".into(),
                "printf '%s' \"$$\" > command.pid; sleep 30 & printf '%s' \"$!\" > descendant.pid; wait".into(),
            ],
            std::path::Path::new(&directory),
            &BTreeMap::new(),
        )
        .unwrap();
}

#[test]
fn interruption_stops_owned_commands_and_their_descendants() {
    use wait_timeout::ChildExt as _;
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::system::tests::cancellation_child_fixture",
            "--nocapture",
        ])
        .env("GIT_PR_CANCEL_FIXTURE", directory.path())
        .process_group(0)
        .spawn()
        .unwrap();
    let started = Instant::now();
    let descendant_path = directory.path().join("descendant.pid");
    while !std::fs::read_to_string(&descendant_path).is_ok_and(|s| !s.is_empty())
        && started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ready = std::fs::read_to_string(&descendant_path).is_ok_and(|s| !s.is_empty());
    if !ready {
        let _ = fixture.kill();
        let _ = fixture.wait();
    }
    assert!(ready, "cancellation fixture did not start");
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &fixture.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = fixture.wait_timeout(Duration::from_secs(5)).unwrap();
    if status.is_none() {
        let _ = fixture.kill();
        let _ = fixture.wait();
    }
    assert_eq!(status.unwrap().code(), Some(130));
    for name in ["command.pid", "descendant.pid"] {
        let pid = std::fs::read_to_string(directory.path().join(name)).unwrap();
        let state = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&state.stdout);
        assert!(
            state.trim().is_empty() || state.trim().starts_with('Z'),
            "surviving {name}: {state}"
        );
    }
}

#[test]
fn configured_deadlines_and_combined_diagnostic_order_are_preserved() {
    let runner = ProcessRunner::default();
    runner.configure_timeouts(Duration::from_millis(50), Duration::from_millis(75));
    let directory = tempfile::tempdir().unwrap();
    let args = ["-c".into(), "sleep 30".into()];
    assert!(
        runner
            .execute_data("sh", &args, directory.path(), &BTreeMap::new(), b"")
            .unwrap_err()
            .to_string()
            .contains("deadline")
    );
    assert!(
        runner
            .execute("sh", &args, directory.path(), &BTreeMap::new())
            .unwrap_err()
            .to_string()
            .contains("deadline")
    );
    runner.configure_timeouts(Duration::from_secs(5), Duration::from_secs(5));
    let output = runner
        .execute(
            "sh",
            &[
                "-c".into(),
                "printf first; printf second >&2; printf third".into(),
            ],
            directory.path(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(output.text, "firstsecondthird");
    assert!(
        runner
            .execute(
                "git-pr-nonexistent-test-command",
                &[],
                directory.path(),
                &BTreeMap::new()
            )
            .unwrap_err()
            .to_string()
            .contains("could not start")
    );
}

#[test]
fn completed_commands_cannot_be_held_open_by_continuous_descendant_output() {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::AtomicBool;

    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let started = Instant::now();
    let producer = std::thread::spawn(move || {
        while started.elapsed() < Duration::from_secs(4) {
            if writer.write_all(b"still writing").is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let result = super::read_bounded(&mut reader, &AtomicBool::new(true));
    drop(reader);
    producer.join().unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("still open in a descendant")
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn capture_failure_terminates_descendants_after_their_parent_exits() {
    let directory = tempfile::tempdir().unwrap();
    let error = ProcessRunner::default()
        .execute(
            "sh",
            &[
                "-c".into(),
                "(sleep 2; printf escaped > escaped) & exit 0".into(),
            ],
            directory.path(),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("still open in a descendant"));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        !directory.path().join("escaped").exists(),
        "descendant continued after capture failed"
    );
}

#[test]
fn denied_execution_and_signal_termination_preserve_native_causes() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("denied");
    std::fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = ProcessRunner::default()
        .execute(
            executable.to_str().unwrap(),
            &[],
            directory.path(),
            &BTreeMap::new(),
        )
        .unwrap_err();
    let cause = error.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(cause.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(format!("{error:#}").contains("could not start"));

    let error = ProcessRunner::default()
        .execute(
            "sh",
            &["-c".into(), "kill -TERM $$".into()],
            directory.path(),
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("terminated by signal 15"));
}

#[test]
fn capture_failure_does_not_replace_the_original_command_failure() {
    let (reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30"]).process_group(0);
    let error = super::capture(command, Duration::from_millis(50), vec![reader]).unwrap_err();
    drop(writer);
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("deadline exceeded")
            && diagnostic.contains("output capture also failed"),
        "{diagnostic}"
    );
}

#[test]
fn interruption_exits_even_when_the_diagnostic_pipe_is_full() {
    use std::{io::Write as _, os::fd::OwnedFd, process::Stdio};
    use wait_timeout::ChildExt as _;
    let directory = tempfile::tempdir().unwrap();
    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    loop {
        match writer.write(&[0_u8; 8192]) {
            Ok(count) => assert_ne!(count, 0),
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                break;
            }
        }
    }
    writer.set_nonblocking(false).unwrap();
    let mut fixture = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::system::tests::cancellation_child_fixture",
            "--nocapture",
        ])
        .env("GIT_PR_CANCEL_FIXTURE", directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::from(OwnedFd::from(writer)))
        .process_group(0)
        .spawn()
        .unwrap();
    let started = Instant::now();
    let path = directory.path().join("descendant.pid");
    while !std::fs::read_to_string(&path).is_ok_and(|text| !text.is_empty())
        && started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ready = std::fs::read_to_string(&path).is_ok_and(|text| !text.is_empty());
    if ready {
        assert!(
            Command::new("/bin/kill")
                .args(["-INT", &fixture.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
    let status = fixture.wait_timeout(Duration::from_secs(5)).unwrap();
    if status.is_none() {
        let _ = fixture.kill();
        let _ = fixture.wait();
    }
    drop(reader);
    assert!(ready, "cancellation fixture did not start");
    assert_eq!(status.unwrap().code(), Some(130));
}

#[test]
fn confirmation_failure_fixture() {
    use crate::commands::Terminal as _;
    let Some(_) = std::env::var_os("GIT_PR_CONFIRM_FAILURE_FIXTURE") else {
        return;
    };
    assert!(!super::Console::default().confirm("Publish changes?"));
}

#[test]
fn confirmation_declines_piped_input_without_accepting() {
    use std::process::Stdio;
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::system::tests::confirmation_failure_fixture",
            "--nocapture",
        ])
        .env("GIT_PR_CONFIRM_FAILURE_FIXTURE", "1")
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Close the pipe without accepting input: non-interactive confirmation
    // must decline immediately rather than depending on an answer or EOF.
    drop(fixture.stdin.take());
    let output = fixture.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("non-interactive; pass --yes to accept. Declining."),
        "{error}"
    );
    assert!(error.contains("Publish changes?"), "{error}");
}

#[test]
fn confirmation_declines_invalid_utf8_from_a_real_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let script = r"
import os, pty, select, subprocess, sys, time
master, slave = pty.openpty()
environment = dict(os.environ, GIT_PR_CONFIRM_FAILURE_FIXTURE='1')
command = [sys.argv[1], '--exact', 'commands::system::tests::confirmation_failure_fixture', '--nocapture']
process = subprocess.Popen(command, stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
os.close(slave)
diagnostic = b''
try:
    deadline = time.monotonic() + 5
    while b'Publish changes? [y/N]:' not in diagnostic and time.monotonic() < deadline:
        if select.select([process.stderr], [], [], .1)[0]:
            chunk = os.read(process.stderr.fileno(), 4096)
            if not chunk:
                break
            diagnostic += chunk
    assert b'Publish changes? [y/N]:' in diagnostic, diagnostic
    os.write(master, b'\xff\n')
    output, tail = process.communicate(timeout=5)
    diagnostic += tail
    assert process.returncode == 0, (output, diagnostic)
    assert b'test result: ok. 1 passed;' in output, output
    assert b'could not read confirmation:' in diagnostic, diagnostic
    assert b'declining' in diagnostic, diagnostic
    assert b'non-interactive' not in diagnostic, diagnostic
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    os.close(master)
";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn system_clock_obeys_the_requested_wait() {
    use crate::commands::Clock as _;
    let started = Instant::now();
    let requested = Duration::from_millis(2);
    super::SystemClock.sleep(requested);
    assert!(started.elapsed() >= requested);
}

#[test]
fn output_worker_allocation_failure_does_not_start_a_command_or_leak_readers() {
    use std::{os::fd::OwnedFd, os::unix::net::UnixStream, process::Stdio};
    let directory = tempfile::tempdir().unwrap();
    for successful_workers in [0, 1] {
        let (stdout, stdout_writer) = UnixStream::pair().unwrap();
        let (stderr, stderr_writer) = UnixStream::pair().unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf started > unexpected"])
            .current_dir(directory.path())
            .stdout(Stdio::from(OwnedFd::from(stdout_writer)))
            .stderr(Stdio::from(OwnedFd::from(stderr_writer)))
            .process_group(0);
        let mut allocated = 0;
        let started = Instant::now();
        let error = super::capture_with_builder(
            command,
            Duration::from_secs(2),
            vec![stdout, stderr],
            &mut || {
                allocated += 1;
                if allocated <= successful_workers {
                    Ok(std::thread::Builder::new())
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "output worker allocation unavailable",
                    ))
                }
            },
        )
        .unwrap_err();
        assert!(
            error.downcast_ref::<std::io::Error>().is_some(),
            "{error:#}"
        );
        assert_eq!(allocated, successful_workers + 1);
        assert!(!directory.path().join("unexpected").exists());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

#[test]
fn output_reads_retry_transient_errors_and_preserve_permanent_failures() {
    use std::{collections::VecDeque, io, sync::atomic::AtomicBool};

    struct Reader(VecDeque<io::Result<Vec<u8>>>);
    impl io::Read for Reader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let bytes = self.0.pop_front().unwrap()?;
            buffer[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
    }

    let mut reader = Reader(VecDeque::from([
        Err(io::ErrorKind::Interrupted.into()),
        Err(io::ErrorKind::WouldBlock.into()),
        Err(io::ErrorKind::TimedOut.into()),
        Ok(b"complete observation".to_vec()),
        Ok(vec![]),
    ]));
    assert_eq!(
        super::read_bounded(&mut reader, &AtomicBool::new(false)).unwrap(),
        b"complete observation"
    );
    let mut reader = Reader(VecDeque::from([Err(io::Error::new(
        io::ErrorKind::ConnectionReset,
        "capture socket reset",
    ))]));
    let error = super::read_bounded(&mut reader, &AtomicBool::new(false)).unwrap_err();
    let cause = error.downcast_ref::<io::Error>().unwrap();
    assert_eq!(cause.kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(cause.to_string(), "capture socket reset");
}

#[test]
fn panicking_output_reader_is_reported_as_failure_without_unwinding_the_caller() {
    let error = std::thread::scope(|scope| {
        let reader = scope.spawn(|| -> anyhow::Result<Vec<u8>> {
            panic!("reader failed unexpectedly");
        });
        super::join_reader(reader).unwrap_err()
    });
    assert_eq!(error.to_string(), "command output reader failed");
}

#[test]
fn native_editor_protocol_preserves_literal_messages_and_removes_private_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let output = ProcessRunner::default()
        .edit_messages(
            "sh",
            &[
                "-c".into(),
                "test \"$GIT_EDITOR\" = \"$JJ_EDITOR\" || exit 2; printf '++ reword aaaa\\nold one\\n++ reword bbbb\\nold two\\n' > edit; sh \"$GIT_PR_EDITOR_SCRIPT\" edit; printf '%s\\n' \"$GIT_PR_EDIT_DIR\"; cat edit".into(),
            ],
            directory.path(),
            &BTreeMap::new(),
            &[
                ("aaaaaaaa".into(), "First message\n\nBody".into()),
                ("bbbbbbbb".into(), "Literal $(touch unexpected) and 'quotes'".into()),
            ],
        )
        .unwrap();
    assert_eq!(output.status, 0, "{}", output.text);
    let (private_directory, edited) = output.text.split_once('\n').unwrap();
    assert!(!std::path::Path::new(private_directory).exists());
    assert_eq!(
        edited,
        "++ reword aaaa\nFirst message\n\nBody\n\n++ reword bbbb\nLiteral $(touch unexpected) and 'quotes'"
    );
    assert!(!directory.path().join("unexpected").exists());
}

#[test]
fn terminal_remembers_the_first_write_failure_after_later_writes_succeed() {
    use crate::commands::Terminal as _;
    use std::os::unix::net::UnixStream;

    let console = super::Console::default();
    let (reader, mut writer) = UnixStream::pair().unwrap();
    drop(reader);
    console.write(&mut writer, "stdout", b"lost output");
    let failure = console.failure().unwrap();
    assert!(failure.starts_with("could not write stdout:"));
    let mut received = Vec::new();
    console.write(&mut received, "stderr", b"durable diagnostic");
    assert_eq!(received, b"durable diagnostic");
    console.write(&mut writer, "stderr", b"also lost");
    assert_eq!(console.failure().as_deref(), Some(failure.as_str()));
}
