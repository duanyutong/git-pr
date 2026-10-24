//! Native process, terminal and timing boundaries.

use super::cancellation::wait;
use super::{Clock, CommandOutput, CommandRunner, StructuredOutput, Terminal};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{Read, Seek, Write},
    os::{
        fd::{AsFd, OwnedFd},
        unix::{fs::FileTypeExt, net::UnixStream, process::CommandExt as _},
    },
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(crate) struct ProcessRunner {
    timeouts: Mutex<(Duration, Duration)>,
}
impl Default for ProcessRunner {
    fn default() -> Self {
        Self {
            timeouts: Mutex::new((Duration::from_secs(120), Duration::from_secs(600))),
        }
    }
}
impl ProcessRunner {
    fn timeout(&self, structured: bool) -> Duration {
        let timeouts = self
            .timeouts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if structured { timeouts.0 } else { timeouts.1 }
    }
}

impl CommandRunner for ProcessRunner {
    fn configure_timeouts(&self, read: Duration, command: Duration) {
        *self
            .timeouts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (read, command);
    }

    #[cfg(test)]
    fn edit_messages(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<OsString, OsString>,
        messages: &[(String, String)],
    ) -> Result<CommandOutput> {
        let directory = tempfile::tempdir()?;
        let mut mapping = String::new();
        for (i, (oid, message)) in messages.iter().enumerate() {
            use std::fmt::Write as _;
            writeln!(mapping, "{oid} {i}")?;
            std::fs::write(directory.path().join(i.to_string()), message)?;
        }
        std::fs::write(directory.path().join("map"), mapping)?;
        let script = directory.path().join("editor.sh");
        std::fs::write(&script, include_str!("editor.sh"))?;
        let mut environment = env.clone();
        environment.insert("GIT_PR_EDIT_DIR".into(), directory.path().into());
        environment.insert("GIT_PR_EDITOR_SCRIPT".into(), script.into());
        environment.insert(
            "GIT_EDITOR".into(),
            "sh -c 'exec sh \"$GIT_PR_EDITOR_SCRIPT\" \"$@\"' git-pr-editor".into(),
        );
        environment.insert(
            "JJ_EDITOR".into(),
            "sh -c 'exec sh \"$GIT_PR_EDITOR_SCRIPT\" \"$@\"' git-pr-editor".into(),
        );
        self.execute(program, args, cwd, &environment)
    }

    fn execute_data(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<OsString, OsString>,
        input: &[u8],
    ) -> Result<StructuredOutput> {
        // Input is finite; output is drained concurrently into bounded buffers.
        let mut stdin = tempfile::tempfile()?;
        stdin.write_all(input)?;
        stdin.rewind()?;
        let (stdout, out_writer) = UnixStream::pair()?;
        let (stderr, err_writer) = UnixStream::pair()?;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .envs(env)
            .stdin(stdin)
            .stdout(Stdio::from(OwnedFd::from(out_writer)))
            .stderr(Stdio::from(OwnedFd::from(err_writer)))
            .process_group(0);
        let (status, mut streams) = capture(command, self.timeout(true), vec![stdout, stderr])?;
        let stderr = streams
            .pop()
            .context("internal error: stderr capture is missing")?;
        let stdout = streams
            .pop()
            .context("internal error: stdout capture is missing")?;
        Ok(StructuredOutput {
            status,
            stdout,
            stderr,
        })
    }

    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        // Both descriptors share a socket, preserving combined diagnostic order.
        let (reader, writer) = UnixStream::pair()?;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer.try_clone()?)))
            .stderr(Stdio::from(OwnedFd::from(writer)))
            .process_group(0);
        let (status, mut streams) = capture(command, self.timeout(false), vec![reader])?;
        let bytes = streams
            .pop()
            .context("internal error: command output capture is missing")?;
        Ok(CommandOutput {
            status,
            text: String::from_utf8_lossy(&bytes).trim().into(),
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Console {
    failure: Mutex<Option<String>>,
}

impl Console {
    fn write(&self, writer: &mut dyn Write, stream: &str, bytes: &[u8]) {
        if let Err(error) = writer.write_all(bytes) {
            self.failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(|| format!("could not write {stream}: {error}"));
        }
    }
}

impl Terminal for Console {
    fn out(&self, text: &str) {
        let mut out = std::io::stdout().lock();
        self.write(&mut out, "stdout", text.as_bytes());
        if !text.ends_with('\n') {
            self.write(&mut out, "stdout", b"\n");
        }
    }
    fn err(&self, text: &str) {
        self.write(&mut std::io::stderr().lock(), "stderr", text.as_bytes());
    }
    fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn confirm(&self, prompt: &str) -> bool {
        // The command contract recognises character devices, including /dev/null.
        let interactive = std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .and_then(|fd| std::fs::File::from(fd).metadata().ok())
            .is_some_and(|metadata| metadata.file_type().is_char_device());
        if !interactive {
            self.err(&format!(
                "{prompt} [y/N]: non-interactive; pass --yes to accept. Declining.\n"
            ));
            return false;
        }
        if crate::cli::progress::colour_enabled() {
            self.err(&format!("\x1b[1;36m{prompt} [y/N]: \x1b[0m"));
        } else {
            self.err(&format!("{prompt} [y/N]: "));
        }
        let mut answer = String::new();
        match std::io::stdin().read_line(&mut answer) {
            Ok(_) => matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"),
            Err(error) => {
                self.err(&format!(
                    "could not read confirmation: {error}; declining\n"
                ));
                false
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct SystemClock;
impl Clock for SystemClock {
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

const CAPTURE_LIMIT: usize = 64 * 1024 * 1024;

fn capture(
    command: Command,
    deadline: Duration,
    streams: Vec<UnixStream>,
) -> Result<(i32, Vec<Vec<u8>>)> {
    capture_with_builder(command, deadline, streams, &mut || {
        Ok(std::thread::Builder::new())
    })
}

fn capture_with_builder(
    mut command: Command,
    deadline: Duration,
    streams: Vec<UnixStream>,
    builder: &mut dyn FnMut() -> std::io::Result<std::thread::Builder>,
) -> Result<(i32, Vec<Vec<u8>>)> {
    let failed = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    for stream in &streams {
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    }
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        let mut spawn_error = None;
        for mut stream in streams {
            let failed = &failed;
            let finished = &finished;
            match builder().and_then(|builder| {
                builder.spawn_scoped(scope, move || {
                    let result = read_bounded(&mut stream, finished);
                    if result.is_err() {
                        failed.store(true, Ordering::Release);
                    }
                    result
                })
            }) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    spawn_error = Some(error);
                    break;
                }
            }
        }
        let status = spawn_error.map_or_else(
            || wait(&mut command, deadline, &failed),
            |error| Err(error.into()),
        );
        // Command owns parent copies of the write descriptors until it is dropped.
        drop(command);
        finished.store(true, Ordering::Release);
        let mut output = Vec::new();
        let mut failure = None;
        for worker in workers {
            match join_reader(worker) {
                Ok(bytes) => output.push(bytes),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = failure {
            return match status {
                Err(cause) => Err(cause)
                    .with_context(|| format!("command output capture also failed: {error:#}")),
                Ok(_) => Err(error),
            };
        }
        let (status, registration) = status?;
        registration.complete();
        Ok((status, output))
    })
}

fn join_reader(worker: std::thread::ScopedJoinHandle<'_, Result<Vec<u8>>>) -> Result<Vec<u8>> {
    worker
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("command output reader failed")))
}

fn read_bounded(stream: &mut dyn Read, finished: &AtomicBool) -> Result<Vec<u8>> {
    // The caller owns the I/O adapter; every reader shares the bounded error policy.
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut finishing = None;
    loop {
        if finished.load(Ordering::Acquire) {
            // Allow a raced read to reach EOF after the parent's writers close.
            // Further bytes must not extend the grace period indefinitely.
            let since = finishing.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(1) {
                bail!("command exited with an output stream still open in a descendant");
            }
        }
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                if count > CAPTURE_LIMIT - bytes.len() {
                    bail!("command output exceeded 64 MiB; refusing an incomplete observation");
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

pub(super) fn spawn_worker<'scope, 'env, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> Result<std::thread::ScopedJoinHandle<'scope, T>> {
    #[cfg(test)]
    crate::tests::unit::support::WORKER_STARTS.update(|n| n + 1);
    std::thread::Builder::new()
        .spawn_scoped(scope, work)
        .context("could not start a submission worker")
}

#[cfg(test)]
#[path = "../tests/process.rs"]
mod tests;
