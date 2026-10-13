//! Terminal presentation, capability detection and a single event-driven renderer.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::IsTerminal as _,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    commands::Terminal,
    model::{Commit, Repository},
};

#[derive(Debug)]
struct Activity {
    label: String,
    started: Instant,
    wait: Option<Duration>,
}

#[derive(Debug)]
struct Progress {
    label: String,
    done: usize,
    total: usize,
}

#[derive(Debug, Default)]
struct State {
    next: u64,
    activities: BTreeMap<u64, Activity>,
    stage: Option<Activity>,
    detail: String,
    progress: Option<Progress>,
    quota: Option<(String, Instant, Duration)>,
    painted: bool,
    paused: bool,
    stopped: bool,
}

#[derive(Debug)]
struct Shared {
    terminal: Arc<dyn Terminal>,
    state: Mutex<State>,
    wake: Condvar,
    animated: bool,
    rich: bool,
    colour: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn colour_enabled() -> bool {
    std::io::stderr().is_terminal()
        && std::env::var_os("CI").is_none()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        && std::env::var("TERM").is_ok_and(|v| v != "dumb")
}

#[derive(Debug)]
pub(crate) struct ProgressTerminal {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ProgressTerminal {
    pub(crate) fn new(terminal: Arc<dyn Terminal>) -> Self {
        let animated = std::io::stderr().is_terminal()
            && std::env::var_os("CI").is_none()
            && std::env::var("TERM").is_ok_and(|term| term != "dumb");
        Self::with_options(
            terminal,
            animated,
            animated && std::io::stdout().is_terminal(),
            colour_enabled(),
        )
    }

    #[cfg(test)]
    fn with_animation(terminal: Arc<dyn Terminal>, animated: bool) -> Self {
        Self::with_options(terminal, animated, animated, false)
    }

    fn with_options(terminal: Arc<dyn Terminal>, animated: bool, rich: bool, colour: bool) -> Self {
        Self::with_worker(terminal, animated, rich, colour, |shared| {
            std::thread::Builder::new()
                .name("git-pr-progress".into())
                .spawn(move || animate(&shared))
        })
    }

    fn with_worker(
        terminal: Arc<dyn Terminal>,
        animated: bool,
        rich: bool,
        colour: bool,
        spawn: impl FnOnce(Arc<Shared>) -> std::io::Result<JoinHandle<()>>,
    ) -> Self {
        let mut shared = Arc::new(Shared {
            terminal,
            state: Mutex::default(),
            wake: Condvar::new(),
            animated,
            rich,
            colour,
        });
        let worker = if animated {
            spawn(Arc::clone(&shared))
                .map_err(|error| {
                    shared.terminal.err(&format!(
                        "Live progress unavailable; using plain output: {error}\n"
                    ));
                })
                .ok()
        } else {
            None
        };
        if worker.is_none()
            && let Some(shared) = Arc::get_mut(&mut shared)
        {
            shared.animated = false;
            shared.rich = false;
        }
        Self {
            shared,
            worker: Mutex::new(worker),
        }
    }

    fn clear(&self, state: &mut State) {
        if state.painted {
            self.shared.terminal.err("\r\x1b[2K");
            state.painted = false;
        }
    }

    fn end_stage(&self, state: &mut State, success: bool) {
        self.clear(state);
        if let Some(stage) = state.stage.take()
            && self.shared.animated
        {
            let label = state.progress.as_ref().map_or_else(
                || stage.label.clone(),
                |progress| {
                    format!(
                        "{}/{} {} · {}",
                        progress.done, progress.total, progress.label, stage.label
                    )
                },
            );
            self.shared.terminal.err(&format!(
                "{}\n",
                stage_result(
                    &label,
                    stage.started.elapsed(),
                    success,
                    width().saturating_sub(1),
                    self.shared.colour,
                )
            ));
        }
    }
}

impl Drop for ProgressTerminal {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        state.stopped = true;
        self.clear(&mut state);
        drop(state);
        self.shared.wake.notify_all();
        let worker = lock(&self.worker).take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

impl Terminal for ProgressTerminal {
    fn out(&self, text: &str) {
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        if self.shared.rich {
            self.shared.terminal.out(&clean(text, self.shared.colour));
        } else {
            self.shared.terminal.out(text);
        }
    }

    fn err(&self, text: &str) {
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        self.shared.terminal.err(&clean(text, self.shared.colour));
    }

    fn confirm(&self, prompt: &str) -> bool {
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        state.paused = true;
        drop(state);
        let answer = self.shared.terminal.confirm(&clean(prompt, false));
        lock(&self.shared.state).paused = false;
        self.shared.wake.notify_all();
        answer
    }

    fn failure(&self) -> Option<String> {
        self.shared.terminal.failure()
    }

    fn section(&self, label: &str) {
        if self.shared.rich {
            self.write_rich(&format!(
                "\n{}\n\n",
                heading(label, width().saturating_sub(1), self.shared.colour)
            ));
        }
    }

    fn stage(&self, label: &str) {
        let mut state = lock(&self.shared.state);
        self.end_stage(&mut state, true);
        state.stage = Some(Activity {
            label: single_line(label),
            started: Instant::now(),
            wait: None,
        });
        state.detail.clear();
        state.progress = None;
        drop(state);
        self.shared.wake.notify_all();
    }

    fn finish(&self, success: bool) {
        let mut state = lock(&self.shared.state);
        self.end_stage(&mut state, success);
        state.activities.clear();
        state.detail.clear();
        state.progress = None;
    }

    fn detail(&self, text: &str) {
        if self.shared.rich {
            lock(&self.shared.state).detail = single_line(text.trim());
            self.shared.wake.notify_all();
        } else {
            self.out(text);
        }
    }

    fn progress(&self, label: &str, done: usize, total: usize) {
        lock(&self.shared.state).progress = Some(Progress {
            label: single_line(label),
            done: done.min(total),
            total,
        });
        self.shared.wake.notify_all();
    }

    fn selection(&self, commits: &[&Commit], stack_up: bool) -> bool {
        if !self.shared.rich {
            return false;
        }
        let width = width().saturating_sub(1);
        let mut text = format!(
            "\n{}\n{}\n\n",
            heading("Selected stack", width, self.shared.colour),
            paint(
                &clip(
                    &format!(
                        "  {} commit{} · {} at the top",
                        commits.len(),
                        if commits.len() == 1 { "" } else { "s" },
                        if stack_up { "tip" } else { "base" }
                    ),
                    width
                ),
                "2",
                self.shared.colour
            )
        );
        for (i, commit) in commits.iter().enumerate() {
            if commits.len() > 8 && (4..commits.len() - 4).contains(&i) {
                if i == 4 {
                    let _ = writeln!(text, "  … {} more commits", commits.len() - 8);
                }
                continue;
            }
            let row = format!("  {}  {}", commit.short_hash(), single_line(&commit.title));
            let _ = writeln!(text, "{}", clip(&row, width));
        }
        self.write_rich(&text);
        true
    }

    fn results(&self, commits: &[&Commit], repo: &Repository, stack_up: bool) -> bool {
        if !self.shared.rich {
            return false;
        }
        let created = commits.iter().filter(|c| c.newly_created).count();
        let updated = commits
            .iter()
            .filter(|c| !c.newly_created && (c.metadata_updated || c.base_updated))
            .count();
        let columns = width().saturating_sub(1);
        let mut text = format!(
            "\n{}\n{}\n{}\n\n",
            heading("Pull requests", columns, self.shared.colour),
            paint(
                &clip(
                    &format!(
                        "  {} PR{} · {} at the top",
                        commits.len(),
                        if commits.len() == 1 { "" } else { "s" },
                        if stack_up { "tip" } else { "base" }
                    ),
                    columns
                ),
                "2",
                self.shared.colour
            ),
            clip(
                &format!(
                    "  {created} created · {updated} updated · {} unchanged",
                    commits.len() - created - updated
                ),
                columns
            )
        );
        if commits.is_empty() {
            text.push_str("  Nothing selected for publication.\n");
        }
        for (index, commit) in commits.iter().enumerate() {
            if index != 0 {
                text.push('\n');
            }
            let (status, code) = if commit.newly_created {
                ("created", "32")
            } else if commit.metadata_updated || commit.base_updated {
                ("updated", "36")
            } else {
                ("unchanged", "2")
            };
            let row = clip(&format!("  #{}  [{status}]", commit.pr_number), columns);
            let _ = writeln!(text, "{}", paint(&row, code, self.shared.colour));
            write_title(&mut text, &single_line(&commit.title), columns);
            let _ = writeln!(
                text,
                "    https://{}/{}/pull/{}",
                single_line(&repo.host),
                single_line(&repo.name),
                commit.pr_number
            );
        }
        text.push('\n');
        self.write_rich(&text);
        true
    }

    fn start_activity(&self, label: &str, wait: Option<Duration>) -> u64 {
        let mut state = lock(&self.shared.state);
        state.next = state.next.wrapping_add(1);
        let id = state.next;
        let label = single_line(label);
        if !self.shared.animated
            && let Some(wait) = wait.filter(|wait| *wait >= Duration::from_secs(1))
        {
            self.shared
                .terminal
                .err(&format!("{label}; waiting {}\n", duration(wait)));
        }
        state.activities.insert(
            id,
            Activity {
                label,
                wait,
                started: Instant::now(),
            },
        );
        drop(state);
        self.shared.wake.notify_all();
        id
    }

    fn end_activity(&self, id: u64) {
        let mut state = lock(&self.shared.state);
        state.activities.remove(&id);
        if state.activities.is_empty() && state.stage.is_none() {
            self.clear(&mut state);
        }
        drop(state);
        self.shared.wake.notify_all();
    }

    fn quota(&self, resource: &str, remaining: u64, limit: u64, reset: Duration) {
        lock(&self.shared.state).quota = Some((
            format!("{} {remaining}/{limit}", single_line(resource)),
            Instant::now(),
            reset,
        ));
    }
}

impl ProgressTerminal {
    fn write_rich(&self, text: &str) {
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        self.shared.terminal.out(text);
    }
}

fn animate(shared: &Shared) {
    let mut frame = 0;
    let mut next_frame = Instant::now();
    let mut state = lock(&shared.state);
    loop {
        if state.stopped || crate::commands::cancellation::interrupting() {
            return;
        }
        if state.paused || (state.activities.is_empty() && state.stage.is_none()) {
            state = shared
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            continue;
        }
        let remaining = next_frame.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            let (next, _) = shared
                .wake
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            continue;
        }
        let text = render_state(&state, frame, width());
        shared
            .terminal
            .err(&format!("\r\x1b[2K{}", paint(&text, "36", shared.colour)));
        frame = (frame + 1) % 10;
        // Notifications update state immediately; bursts must not accelerate repainting.
        next_frame = Instant::now() + Duration::from_millis(100);
        state.painted = true;
    }
}

fn stage_result(
    label: &str,
    elapsed: Duration,
    success: bool,
    columns: usize,
    colour: bool,
) -> String {
    let mark = if success { "✓" } else { "✗" };
    let code = if success { "32" } else { "31" };
    let elapsed = format!("{:.1}s", elapsed.as_secs_f64());
    let label_columns = columns.saturating_sub(elapsed.width() + 6).min(56);
    let label = clip(label, label_columns);
    let padding = " ".repeat(label_columns.saturating_sub(label.width()));
    let row = format!("  {mark} {label}{padding}  {elapsed}");
    if row.width() > columns {
        clip(&row, columns)
    } else {
        format!(
            "  {} {label}{padding}  {}",
            paint(mark, code, colour),
            paint(&elapsed, "2", colour)
        )
    }
}

fn render_state(state: &State, frame: usize, width: usize) -> String {
    let latest = state.activities.last_key_value().map(|(_, value)| value);
    let wait = state.activities.values().rev().find(|a| a.wait.is_some());
    let Some(active) = wait.or(state.stage.as_ref()).or(latest) else {
        return String::new();
    };
    let mut label = active.label.clone();
    if wait.is_none() {
        if let Some(detail) = latest.filter(|a| !std::ptr::eq(*a, active)) {
            let _ = write!(label, " · {}", detail.label);
        } else if !state.detail.is_empty() {
            let _ = write!(label, " · {}", state.detail);
        }
    }
    render(
        &Activity {
            label,
            started: active.started,
            wait: active.wait,
        },
        state.progress.as_ref(),
        state.quota.as_ref(),
        frame,
        width,
    )
}

fn render(
    activity: &Activity,
    progress: Option<&Progress>,
    quota: Option<&(String, Instant, Duration)>,
    frame: usize,
    width: usize,
) -> String {
    let elapsed = activity.started.elapsed();
    let compact = progress.is_some() && width < 64;
    let timer = activity.wait.map_or_else(
        || {
            if compact {
                format!("{}s", seconds(elapsed))
            } else {
                format!("{} elapsed", duration(elapsed))
            }
        },
        |wait| {
            let remaining = wait.saturating_sub(elapsed);
            if compact {
                format!("{}s wait", seconds(remaining))
            } else {
                format!("{} remaining", duration(remaining))
            }
        },
    );
    let available = width.saturating_sub(1);
    let (label, count) = progress.map_or_else(
        || (activity.label.clone(), String::new()),
        |progress| {
            (
                format!("{} · {}", progress.label, activity.label),
                format!("{}/{} ", progress.done, progress.total),
            )
        },
    );
    let mut suffix = format!(" · {timer}");
    if let Some((label, observed, reset)) = quota {
        let quota = format!(
            " · {label} · resets in {}",
            duration(reset.saturating_sub(observed.elapsed()))
        );
        // Preserve the item identity and count before adding ancillary quota metadata.
        let label_columns = progress.map_or(10, |progress| progress.label.width().min(32));
        if suffix.width() + quota.width() + count.width() + label_columns + 4 <= available {
            suffix.push_str(&quota);
        }
    }
    let label = clip(
        &label,
        available.saturating_sub(suffix.width() + count.width() + 4),
    );
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][frame % 10];
    clip(&format!("  {spinner} {count}{label}{suffix}"), available)
}

fn width() -> usize {
    rustix::termios::tcgetwinsize(std::io::stderr())
        .ok()
        .filter(|size| size.ws_col != 0)
        .map_or(80, |size| usize::from(size.ws_col))
}

fn heading(label: &str, columns: usize, colour: bool) -> String {
    let columns = columns.min(80);
    let label = clip(&format!("  {}", single_line(label)), columns);
    let remaining = columns.saturating_sub(label.width());
    let rule = if remaining > 1 {
        format!(" {}", "─".repeat(remaining - 1))
    } else {
        String::new()
    };
    format!(
        "{}{}",
        paint(&label, "1", colour),
        paint(&rule, "2", colour)
    )
}

fn write_title(output: &mut String, title: &str, columns: usize) {
    let indent = columns.min(4);
    let available = columns.saturating_sub(indent);
    if available == 0 {
        return;
    }
    let mut line = String::new();
    let mut used = 0;
    for word in title.split_whitespace() {
        if used != 0 && used + 1 + word.width() > available {
            let _ = writeln!(output, "{:indent$}{line}", "");
            line.clear();
            used = 0;
        }
        if used != 0 {
            line.push(' ');
            used += 1;
        }
        for grapheme in word.graphemes(true) {
            let grapheme = if grapheme.width() > available {
                "…"
            } else {
                grapheme
            };
            if used + grapheme.width() > available {
                let _ = writeln!(output, "{:indent$}{line}", "");
                line.clear();
                used = 0;
            }
            line.push_str(grapheme);
            used += grapheme.width();
        }
    }
    if !line.is_empty() {
        let _ = writeln!(output, "{:indent$}{line}", "");
    }
}

fn paint(text: &str, code: &str, colour: bool) -> String {
    if colour {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.into()
    }
}

fn clip(text: &str, columns: usize) -> String {
    if text.width() <= columns {
        return text.into();
    }
    if columns == 0 {
        return String::new();
    }
    let mut used = 0;
    let prefix: String = text
        .graphemes(true)
        .take_while(|grapheme| {
            used += grapheme.width();
            used < columns
        })
        .collect();
    format!("{prefix}…")
}

fn single_line(text: &str) -> String {
    clean(text, false).replace(['\n', '\t'], " ")
}

/// Retain generated SGR styling only where enabled; make other terminal controls inert.
fn clean(text: &str, colour: bool) -> String {
    let mut rest = text;
    let mut output = String::with_capacity(text.len());
    while let Some(c) = rest.chars().next() {
        if let Some(sequence) = rest.strip_prefix("\x1b[") {
            let end = sequence
                .bytes()
                .take(64)
                .position(|c| !c.is_ascii_digit() && !matches!(c, b';' | b':'));
            if let Some(end) = end
                && sequence.as_bytes()[end] == b'm'
            {
                let length = end + 3;
                if colour {
                    output.push_str(&rest[..length]);
                }
                rest = &rest[length..];
                continue;
            }
        }
        if c.is_control() && !matches!(c, '\n' | '\t') {
            output.extend(c.escape_default());
        } else {
            output.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    output
}

fn seconds(value: Duration) -> u64 {
    value
        .as_secs()
        .saturating_add(u64::from(value.subsec_nanos() > 0))
}

fn duration(value: Duration) -> String {
    let seconds = seconds(value);
    if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
#[path = "../tests/progress.rs"]
mod tests;
