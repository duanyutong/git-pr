//! Own process groups through normal completion, deadlines and terminal interruption.

use anyhow::{Context, Result, anyhow, bail};
use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, WaitIdStatus, kill_process_group, waitid,
};
use std::{
    io::{IsTerminal as _, Write as _},
    process::{Child, Command},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

static INTERRUPTING: AtomicBool = AtomicBool::new(false);
static CHILDREN: Mutex<Vec<Arc<Mutex<Child>>>> = Mutex::new(Vec::new());
static HANDLER: OnceLock<std::result::Result<(), String>> = OnceLock::new();

pub(crate) fn interrupting() -> bool {
    INTERRUPTING.load(Ordering::Acquire)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn signal_group(child: &Child, signal: Signal) {
    let _ = kill_process_group(Pid::from_child(child), signal);
}

fn terminate(child: &mut Child) {
    signal_group(child, Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "Shutdown retains registry and child ownership until process exit to prevent new effects"
)]
fn interrupt() {
    INTERRUPTING.store(true, Ordering::Release);
    // Registration and spawning share this lock: interruption cannot miss a child
    // between spawn and registration, nor permit a new command during shutdown.
    let children = lock(&CHILDREN);
    let mut active: Vec<_> = children.iter().map(|child| lock(child)).collect();
    for child in &active {
        signal_group(child, Signal::INT);
    }
    if !active.is_empty() {
        // Retain unreaped leaders throughout the grace period: descendants may
        // ignore SIGINT, and a reaped leader's process-group identity can be reused.
        std::thread::sleep(Duration::from_secs(2));
    }
    for child in &mut active {
        terminate(child);
    }
    // A full pipe or a terminal lock held by interrupted work must not delay exit.
    let (notice, completed) = std::sync::mpsc::channel();
    if std::thread::Builder::new().spawn(move || {
        let mut stderr = std::io::stderr().lock();
        if std::io::stderr().is_terminal() {
            let _ = write!(stderr, "\r\x1b[2K");
        }
        let _ = writeln!(stderr, "Interrupted: owned commands have been stopped. Completed local or remote changes may remain; inspect their state before retrying.");
        drop(stderr);
        let _ = notice.send(());
    }).is_ok() {
        let _ = completed.recv_timeout(Duration::from_millis(100));
    }
    // The handler runs on an ordinary thread, so locking, waiting and exit are safe.
    std::process::exit(130);
}

#[derive(Debug)]
pub(super) struct Registration {
    child: Arc<Mutex<Child>>,
    completed: bool,
}

impl Registration {
    pub(super) fn complete(mut self) {
        self.completed = true;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        await_shutdown();
        // Reaping and unregistering share ownership with terminal cancellation.
        let mut children = lock(&CHILDREN);
        let mut child = lock(&self.child);
        if self.completed {
            let _ = child.wait();
        } else {
            terminate(&mut child);
        }
        drop(child);
        children.retain(|active| !Arc::ptr_eq(active, &self.child));
    }
}

pub(super) fn wait(
    command: &mut Command,
    deadline: Duration,
    failed: &AtomicBool,
) -> Result<(i32, Registration)> {
    wait_with_builder(command, deadline, failed, &mut || {
        Ok(std::thread::Builder::new())
    })
}

fn wait_with_builder(
    command: &mut Command,
    deadline: Duration,
    failed: &AtomicBool,
    builder: &mut dyn FnMut() -> std::io::Result<std::thread::Builder>,
) -> Result<(i32, Registration)> {
    HANDLER
        .get_or_init(|| ctrlc::set_handler(interrupt).map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| anyhow!("failed to initialise command cancellation: {error}"))?;
    await_shutdown();
    let child = {
        let mut children = lock(&CHILDREN);
        let child = command.spawn().with_context(|| format!(
            "could not start {}; check that the executable is installed, its permissions permit execution, and the working directory exists",
            command.get_program().to_string_lossy()
        ))?;
        let child = Arc::new(Mutex::new(child));
        children.push(Arc::clone(&child));
        drop(children);
        child
    };
    let registration = Registration {
        child,
        completed: false,
    };
    let status = wait_registered(&registration.child, deadline, failed, builder)?;
    Ok((status, registration))
}

fn wait_registered(
    child: &Mutex<Child>,
    deadline: Duration,
    failed: &AtomicBool,
    builder: &mut dyn FnMut() -> std::io::Result<std::thread::Builder>,
) -> Result<i32> {
    let pid = Pid::from_child(&lock(child));
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        // Blocking waitid wakes immediately on exit, without a polling delay on
        // every short command. NOWAIT reserves the group identity through capture.
        builder().and_then(|builder| {
            builder.spawn_scoped(scope, move || {
                let _ = sender.send(observe_exit(pid));
            })
        })?;
        let result = await_exit(&receiver, deadline, failed);
        if result.is_err() {
            let mut child = lock(child);
            signal_group(&child, Signal::KILL);
            let _ = child.kill();
        }
        result
    })
}

fn observe_exit(pid: Pid) -> Result<i32> {
    observe_exit_with(|| {
        waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        )
    })
}

fn observe_exit_with(
    mut observe: impl FnMut() -> rustix::io::Result<Option<WaitIdStatus>>,
) -> Result<i32> {
    loop {
        match observe() {
            Err(rustix::io::Errno::INTR) => {}
            result => {
                let status = result?.context("command exit observation returned no status")?;
                return status.exit_status().ok_or_else(|| {
                    anyhow!(
                        "command terminated by signal {}",
                        status.terminating_signal().unwrap_or(0)
                    )
                });
            }
        }
    }
}

fn await_exit(
    receiver: &std::sync::mpsc::Receiver<Result<i32>>,
    deadline: Duration,
    failed: &AtomicBool,
) -> Result<i32> {
    let started = Instant::now();
    loop {
        await_shutdown();
        let remaining = deadline.saturating_sub(started.elapsed());
        if failed.load(Ordering::Acquire) {
            bail!("command output capture failed; command terminated");
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(status) => return status,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if started.elapsed() < deadline => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                bail!("command exit observer failed")
            }
            Err(_) => {
                bail!(
                    "command deadline exceeded after {} seconds",
                    deadline.as_secs()
                );
            }
        }
    }
}

fn await_shutdown() {
    // Release normal execution to the signal thread instead of competing for a
    // child mutex while shutdown owns termination and reaping.
    while INTERRUPTING.load(Ordering::Acquire) {
        std::thread::park();
    }
}

#[cfg(test)]
#[path = "../tests/cancellation.rs"]
mod tests;
