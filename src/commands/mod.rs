pub(crate) mod cancellation;
mod system;
pub(crate) use system::{Console, ProcessRunner, SystemClock};

use std::fmt::Write as _;
use std::{
    collections::BTreeMap, ffi::OsString, fmt::Debug, path::PathBuf, sync::Arc, time::Duration,
};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug, Default)]
pub(crate) struct CommandOutput {
    pub status: i32,
    pub text: String,
}

impl CommandOutput {
    pub(crate) fn success(self) -> Result<String> {
        if self.status != 0 {
            if self.text.is_empty() {
                bail!("exit code {}", self.status);
            }
            bail!(
                "exit code {}\n{}",
                self.status,
                crate::diagnostics::errors::redact_url_credentials(self.text.trim())
            );
        }
        Ok(self.text.trim().into())
    }
}

#[derive(Debug, Default)]
pub(crate) struct StructuredOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub(crate) trait CommandRunner: Debug + Send + Sync {
    fn configure_timeouts(&self, _read: Duration, _command: Duration) {}

    fn execute_data(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<OsString, OsString>,
        input: &[u8],
    ) -> Result<StructuredOutput> {
        if !input.is_empty() {
            bail!("command adapter does not support input");
        }
        let output = self.execute(program, args, cwd, env)?;
        Ok(StructuredOutput {
            status: output.status,
            stdout: if output.status == 0 {
                output.text.as_bytes().to_vec()
            } else {
                Vec::new()
            },
            stderr: if output.status == 0 {
                Vec::new()
            } else {
                output.text.into_bytes()
            },
        })
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
        let _ = (program, args, cwd, env, messages);
        bail!("command adapter does not support bulk message editing")
    }

    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput>;
}

pub(crate) trait Terminal: Debug + Send + Sync {
    fn out(&self, text: &str);
    fn err(&self, text: &str);
    fn confirm(&self, prompt: &str) -> bool;
    fn section(&self, _label: &str) {}
    fn stage(&self, _label: &str) {}
    fn finish(&self, _success: bool) {}
    fn detail(&self, text: &str) {
        self.out(text);
    }
    fn progress(&self, _label: &str, _done: usize, _total: usize) {}
    fn selection(&self, _commits: &[&crate::model::Commit], _stack_up: bool) -> bool {
        false
    }
    fn results(
        &self,
        _commits: &[&crate::model::Commit],
        _repo: &crate::model::Repository,
        _stack_up: bool,
    ) -> bool {
        false
    }
    fn start_activity(&self, _label: &str, _wait: Option<Duration>) -> u64 {
        0
    }
    fn end_activity(&self, _id: u64) {}
    fn quota(&self, _resource: &str, _remaining: u64, _limit: u64, _reset: Duration) {}
    fn failure(&self) -> Option<String> {
        None
    }
}

pub(crate) trait Clock: Debug + Send + Sync {
    fn sleep(&self, duration: Duration);
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

#[derive(Debug)]
pub(crate) struct Activity<'a> {
    terminal: &'a dyn Terminal,
    id: u64,
}

impl Drop for Activity<'_> {
    fn drop(&mut self) {
        self.terminal.end_activity(self.id);
    }
}

#[derive(Debug)]
pub(crate) struct Progress<'a> {
    commands: &'a Commands,
    label: &'static str,
    completed: usize,
    total: usize,
}

impl Progress<'_> {
    pub(crate) fn targets(&self, numbers: impl Iterator<Item = u64>) {
        self.commands
            .progress_prs(self.label, numbers, self.completed, self.total);
    }

    pub(crate) fn advance(&mut self, count: usize) {
        self.completed += count;
        self.commands
            .terminal
            .progress(self.label, self.completed, self.total);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Commands {
    pub runner: Arc<dyn CommandRunner>,
    pub terminal: Arc<dyn Terminal>,
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub verbose: bool,
    pub diagnostics: crate::diagnostics::Diagnostics,
}

impl Commands {
    pub(crate) fn progress(&self, label: &'static str, total: usize) -> Progress<'_> {
        self.terminal.progress(label, 0, total);
        Progress {
            commands: self,
            label,
            completed: 0,
            total,
        }
    }
    pub(crate) fn activity(&self, label: &str, wait: Option<Duration>) -> Activity<'_> {
        Activity {
            terminal: self.terminal.as_ref(),
            id: self.terminal.start_activity(label, wait),
        }
    }

    pub(crate) fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        let _activity = self.activity(&format!("{program} {}", args.first().unwrap_or(&"")), None);
        if self.verbose {
            self.debug(&format!(
                "{program} {}",
                args.iter()
                    .map(|s| if s.contains(' ') {
                        format!("{s:?}")
                    } else {
                        (*s).into()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        let args: Vec<_> = args.iter().map(|s| (*s).to_owned()).collect();
        let result = self
            .runner
            .execute(program, &args, &self.cwd, &self.env)
            .and_then(CommandOutput::success)
            .with_context(|| self.failure_context(program, &args));
        if self.verbose {
            self.debug(
                &result
                    .as_ref()
                    .map_or_else(ToString::to_string, Clone::clone),
            );
        }
        result
    }
    pub(crate) fn git_data(&self, args: &[&str], input: &[u8]) -> Result<String> {
        self.data("git", args, input)
    }
    pub(crate) fn jj_data(&self, args: &[&str]) -> Result<String> {
        self.data("jj", args, &[]).map(|s| s.trim().to_owned())
    }
    pub(crate) fn data(&self, program: &str, args: &[&str], input: &[u8]) -> Result<String> {
        let _activity = self.activity(&format!("{program} {}", args.first().unwrap_or(&"")), None);
        let args: Vec<_> = args.iter().map(|s| (*s).to_owned()).collect();
        let output = self
            .runner
            .execute_data(program, &args, &self.cwd, &self.env, input)
            .with_context(|| self.failure_context(program, &args))?;
        if output.status != 0 {
            bail!(
                "{}: exit code {}\nstderr:\n{}\nstdout:\n{}",
                self.failure_context(program, &args),
                output.status,
                crate::diagnostics::errors::redact_url_credentials(&String::from_utf8_lossy(
                    &output.stderr
                )),
                crate::diagnostics::errors::redact_url_credentials(&String::from_utf8_lossy(
                    &output.stdout
                ))
            );
        }
        if self.verbose {
            self.debug(&String::from_utf8_lossy(&output.stderr));
        }
        String::from_utf8(output.stdout).with_context(|| {
            format!(
                "{}: command output is not valid UTF-8",
                self.failure_context(program, &args)
            )
        })
    }
    fn failure_context(&self, program: &str, args: &[String]) -> String {
        // Arguments can contain complete commit messages; identify the operation
        // without echoing arbitrary arguments or standard input into diagnostics.
        format!(
            "command `{program} {}` failed in {}",
            args.first().map_or("", String::as_str),
            self.cwd.display()
        )
    }
    pub(crate) fn git(&self, args: &[&str]) -> Result<String> {
        self.run("git", args)
    }
    pub(crate) fn jj(&self, args: &[&str]) -> Result<String> {
        self.run("jj", args)
    }
    pub(crate) fn out(&self, text: &str) {
        self.terminal.out(text);
    }
    pub(crate) fn detail(&self, text: &str) {
        self.terminal.detail(text);
    }
    pub(crate) fn progress_prs(
        &self,
        label: &str,
        numbers: impl Iterator<Item = u64>,
        done: usize,
        total: usize,
    ) {
        let numbers = numbers
            .map(|number| format!("#{number}"))
            .collect::<Vec<_>>()
            .join(", ");
        self.terminal
            .progress(&format!("{label} {numbers}"), done, total);
    }
    pub(crate) fn warn(&self, text: &str) {
        let text = crate::diagnostics::errors::redact_url_credentials(text);
        self.terminal
            .err(&format!("\x1b[33m{}\x1b[0m\n", text.trim_end_matches('\n')));
    }
    pub(crate) fn debug(&self, text: &str) {
        if self.verbose && !text.is_empty() {
            let text = crate::diagnostics::errors::redact_url_credentials(text);
            let lines: Vec<_> = text.trim_end_matches('\n').split('\n').collect();
            let mut result = String::from("\x1b[38;5;245m");
            for (i, line) in lines.iter().enumerate() {
                let marker = if lines.len() == 1 {
                    " │ "
                } else if i == 0 {
                    " ┌ "
                } else if i + 1 == lines.len() {
                    " └ "
                } else {
                    " │ "
                };
                let _ = writeln!(result, "{marker}{line}");
            }
            result.push_str("\x1b[0m");
            self.terminal.err(&result);
        }
    }
}

#[cfg(test)]
pub(crate) fn parallel<T: Send, R: Send>(
    items: Vec<T>,
    function: impl Fn(T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    parallel_cached(items, |_| None, function)
}

pub(crate) fn parallel_cached<T: Send, R: Send>(
    items: Vec<T>,
    cached: impl Fn(&T) -> Option<R>,
    function: impl Fn(T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    enum Worker<'scope, R> {
        Ready(R),
        Pending(std::thread::ScopedJoinHandle<'scope, Result<R>>),
    }
    // Cached operations require no worker; join every scheduled operation before failure.
    const WORKERS: usize = 8;
    let mut remaining = items.into_iter();
    let mut output = Vec::new();
    loop {
        let batch: Vec<_> = remaining.by_ref().take(WORKERS).collect();
        if batch.is_empty() {
            return Ok(output);
        }
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .into_iter()
                .map(|item| {
                    cached(&item).map_or_else(
                        || system::spawn_worker(scope, || function(item)).map(Worker::Pending),
                        |value| Ok(Worker::Ready(value)),
                    )
                })
                .collect();
            let joined: Vec<_> = handles
                .into_iter()
                .map(|h| {
                    h.and_then(|h| match h {
                        Worker::Ready(value) => Ok(value),
                        Worker::Pending(handle) => handle
                            .join()
                            .map_err(|_| anyhow::anyhow!("worker terminated unexpectedly"))
                            .and_then(|result| result),
                    })
                })
                .collect();
            joined.into_iter().collect::<Result<Vec<_>>>()
        })?;
        output.extend(results);
    }
}
