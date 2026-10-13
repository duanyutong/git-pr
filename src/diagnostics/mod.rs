//! Opt-in timing at execution boundaries; no submission decisions belong here.

pub(crate) mod errors;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt::Write as _,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::commands::{CommandOutput, CommandRunner, StructuredOutput, Terminal};

#[derive(Clone, Debug, Default)]
pub(crate) struct Diagnostics(Option<Arc<Session>>);

#[derive(Debug)]
struct Session {
    start: Instant,
    terminal: Arc<dyn Terminal>,
    counters: Mutex<Counters>,
}

#[derive(Debug, Default)]
struct Counters {
    next_id: u64,
    totals: BTreeMap<&'static str, (u64, Duration)>,
}

#[derive(Debug)]
pub(crate) struct Span<'a>(Option<Active<'a>>);

#[derive(Debug)]
struct Active<'a> {
    session: &'a Session,
    start: Instant,
    id: u64,
    kind: &'static str,
    group: &'static str,
    label: String,
}

impl Diagnostics {
    pub(crate) fn new(terminal: Arc<dyn Terminal>) -> Self {
        Self(Some(Arc::new(Session {
            start: Instant::now(),
            terminal,
            counters: Mutex::default(),
        })))
    }

    pub(crate) fn start(&self, kind: &'static str, label: impl FnOnce() -> String) -> Span<'_> {
        self.start_group(kind, kind, label)
    }

    fn start_group(
        &self,
        kind: &'static str,
        group: &'static str,
        label: impl FnOnce() -> String,
    ) -> Span<'_> {
        Span(self.0.as_ref().map(|session| {
            let label = label().replace(char::is_control, "?");
            let mut counters = session
                .counters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            counters.next_id += 1;
            let id = counters.next_id;
            drop(counters);
            session.terminal.err(&format!(
                "[+{:8.3}s #{id}] start {kind}: {label}\n",
                session.start.elapsed().as_secs_f64()
            ));
            Active {
                session,
                start: Instant::now(),
                id,
                kind,
                group,
                label,
            }
        }))
    }

    pub(crate) fn phase<T>(
        &self,
        label: &'static str,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let span = self.start_group("phase", label, || label.into());
        let result = action();
        span.finish(|| if result.is_ok() { "ok" } else { "error" }.into());
        result
    }

    pub(crate) fn summary(&self, success: bool) {
        let Some(session) = &self.0 else { return };
        let mut text = format!(
            "Timing summary: {} in {:.3}s elapsed\n",
            if success { "completed" } else { "failed" },
            session.start.elapsed().as_secs_f64()
        );
        let counters = session
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (kind, (count, duration)) in &counters.totals {
            let _ = writeln!(
                text,
                "  {kind}: {count} operations, {:.3}s accumulated",
                duration.as_secs_f64()
            );
        }
        drop(counters);
        text.push_str("  Durations overlap across phases and concurrent operations; do not add them to obtain elapsed time.\n");
        session.terminal.err(&text);
    }
}

impl Span<'_> {
    pub(crate) fn finish(self, status: impl FnOnce() -> String) {
        let Some(active) = self.0 else { return };
        let elapsed = active.start.elapsed();
        let mut counters = active
            .session
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (count, total) = counters.totals.entry(active.group).or_default();
        *count += 1;
        *total += elapsed;
        drop(counters);
        active.session.terminal.err(&format!(
            "[+{:8.3}s #{}] end {}: {} ({:.3}s; {})\n",
            active.session.start.elapsed().as_secs_f64(),
            active.id,
            active.kind,
            active.label,
            elapsed.as_secs_f64(),
            status()
        ));
    }
}

#[derive(Debug)]
pub(crate) struct TimedRunner {
    pub inner: Arc<dyn CommandRunner>,
    pub diagnostics: Diagnostics,
}

impl CommandRunner for TimedRunner {
    fn configure_timeouts(&self, read: Duration, command: Duration) {
        self.inner.configure_timeouts(read, command);
    }

    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        let span = self
            .diagnostics
            .start("command", || command_label(program, args));
        let result = self.inner.execute(program, args, cwd, env);
        span.finish(|| command_status(result.as_ref().map(|output| output.status)));
        result
    }

    fn execute_data(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
        input: &[u8],
    ) -> Result<StructuredOutput> {
        let span = self
            .diagnostics
            .start("command", || command_label(program, args));
        let result = self.inner.execute_data(program, args, cwd, env, input);
        span.finish(|| command_status(result.as_ref().map(|output| output.status)));
        result
    }

    fn edit_messages(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
        messages: &[(String, String)],
    ) -> Result<CommandOutput> {
        let span = self
            .diagnostics
            .start("command", || command_label(program, args));
        let result = self.inner.edit_messages(program, args, cwd, env, messages);
        span.finish(|| command_status(result.as_ref().map(|output| output.status)));
        result
    }
}

fn command_status(result: Result<i32, &anyhow::Error>) -> String {
    result.map_or_else(|_| "error".into(), |status| format!("exit {status}"))
}

fn command_label(program: &str, args: &[String]) -> String {
    // Only operation names are useful here. Payloads, refs, configuration values,
    // credentials and arbitrary options remain exclusive to detailed diagnostics.
    let mut arguments = args.iter().map(String::as_str).peekable();
    loop {
        match arguments.peek().copied() {
            Some("-c" | "--config" | "--config-toml") => {
                arguments.next();
                arguments.next();
            }
            Some("--ignore-working-copy" | "--no-pager" | "--no-optional-locks") => {
                arguments.next();
            }
            _ => break,
        }
    }
    let words = arguments.take(3).take_while(|arg| {
        matches!(
            *arg,
            "config"
                | "rev-parse"
                | "rev-list"
                | "op"
                | "remote"
                | "symbolic-ref"
                | "branch"
                | "status"
                | "log"
                | "show"
                | "show-ref"
                | "for-each-ref"
                | "cat-file"
                | "merge-base"
                | "diff"
                | "diff-tree"
                | "push"
                | "update-ref"
                | "worktree"
                | "list"
                | "branchless"
                | "reword"
                | "restack"
                | "query"
                | "util"
                | "snapshot"
                | "root"
                | "describe"
                | "bookmark"
                | "set"
                | "delete"
                | "git"
                | "export"
                | "import"
                | "find-generic-password"
        )
    });
    let mut label = match program {
        "git" | "jj" | "security" => program.to_owned(),
        _ => "subprocess".into(),
    };
    for word in words {
        label.push(' ');
        label.push_str(word);
    }
    label
}
