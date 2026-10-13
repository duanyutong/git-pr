//! Cancellation failures remain observable without spawning unowned commands.

use super::{await_exit, observe_exit_with, wait};
use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
use std::{
    process::Command,
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

#[test]
fn exit_observation_retries_interruption_without_reaping_the_owned_child() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new("sh")
        .args(["-c", "exit 7"])
        .current_dir(directory.path())
        .spawn()
        .unwrap();
    let pid = Pid::from_child(&child);
    let mut calls = 0;
    let observed = observe_exit_with(|| {
        calls += 1;
        if calls == 1 {
            Err(rustix::io::Errno::INTR)
        } else {
            waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
            )
        }
    })
    .unwrap();
    assert_eq!(observed, 7);
    assert_eq!(calls, 2);
    assert_eq!(child.wait().unwrap().code(), Some(7));
}

#[test]
fn absent_and_failed_exit_observations_are_not_reported_as_success() {
    let error = observe_exit_with(|| Ok(None)).unwrap_err();
    assert!(error.to_string().contains("returned no status"));
    let error = observe_exit_with(|| Err(rustix::io::Errno::CHILD)).unwrap_err();
    assert_eq!(
        error.downcast_ref::<rustix::io::Errno>(),
        Some(&rustix::io::Errno::CHILD)
    );
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    let started = Instant::now();
    let error =
        await_exit(&receiver, Duration::from_secs(30), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.to_string(), "command exit observer failed");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn unavailable_signal_handler_fixture() {
    let Some(directory) = std::env::var_os("GIT_PR_HANDLER_FAILURE_FIXTURE") else {
        return;
    };
    ctrlc::set_handler(|| {}).unwrap();
    let mut command = Command::new("sh");
    command
        .args(["-c", "printf started > unexpected"])
        .current_dir(&directory);
    let error = wait(
        &mut command,
        Duration::from_secs(1),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("failed to initialise command cancellation:")
    );
    assert!(!std::path::Path::new(&directory).join("unexpected").exists());
}

#[test]
fn signal_handler_installation_failure_prevents_any_command_effect() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::cancellation::tests::unavailable_signal_handler_fixture",
            "--nocapture",
        ])
        .env("GIT_PR_HANDLER_FAILURE_FIXTURE", directory.path())
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!directory.path().join("unexpected").exists());
}

#[test]
fn exit_observer_allocation_failure_terminates_the_already_owned_command() {
    use std::os::unix::process::CommandExt as _;
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new("sh");
    command
        .args(["-c", "sleep 1; printf escaped > escaped"])
        .current_dir(directory.path())
        .process_group(0);
    let error = super::wait_with_builder(
        &mut command,
        Duration::from_secs(5),
        &AtomicBool::new(false),
        &mut || {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "exit observer allocation unavailable",
            ))
        },
    )
    .unwrap_err();
    let cause = error.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(cause.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(cause.to_string(), "exit observer allocation unavailable");
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        !directory.path().join("escaped").exists(),
        "command continued after its exit observer could not start"
    );
}
