//! Terminal presentation, capability detection and a single event-driven renderer.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::IsTerminal as _,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr as _;

use super::{
    hyperlinks,
    layout::{clip, complete_lines},
    results,
};

use crate::{
    commands::Terminal,
    model::{Commit, Repository},
};

#[derive(Debug)]
struct Activity {
    label: String,
    detail: String,
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
    completed_stage: Option<String>,
    detail: String,
    progress: Option<Progress>,
    quota: Option<(String, Instant, Duration)>,
    painted: usize,
    fallback_reported: bool,
    painted_width: usize,
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
        clear_rows(self.shared.terminal.as_ref(), state);
    }

    fn end_stage(&self, state: &mut State, success: bool) {
        self.clear(state);
        let completed = state.completed_stage.take().filter(|_| success);
        if let Some(stage) = state.stage.take()
            && self.shared.animated
        {
            let label = completed.as_deref().unwrap_or(&stage.label);
            let label = state.progress.as_ref().map_or_else(
                || label.to_owned(),
                |progress| {
                    if completed.is_some() {
                        format!("{label} · {}/{}", progress.done, progress.total)
                    } else {
                        format!(
                            "{}/{} {} · {label}",
                            progress.done, progress.total, progress.label
                        )
                    }
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
    fn output_columns(&self) -> Option<usize> {
        std::io::stdout()
            .is_terminal()
            .then(|| output_width().saturating_sub(1))
    }

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
                heading(label, output_width().saturating_sub(1), self.shared.colour)
            ));
        }
    }

    fn stage(&self, label: &str) {
        self.begin_stage(label, None);
    }

    fn stage_with_completion(&self, label: &str, completed: &str) {
        self.begin_stage(label, Some(completed));
    }

    fn finish(&self, success: bool) {
        let mut state = lock(&self.shared.state);
        self.end_stage(&mut state, success);
        state.activities.clear();
        state.detail.clear();
        state.progress = None;
    }

    fn outcome(&self, completed: &str) {
        if self.shared.animated {
            let mut state = lock(&self.shared.state);
            state.completed_stage = Some(single_line(completed));
            state.progress = None;
        } else {
            self.shared
                .terminal
                .err(&format!("{}\n", clean(completed, self.shared.colour)));
        }
    }

    fn detail(&self, text: &str) {
        if self.shared.rich {
            let mut state = lock(&self.shared.state);
            state.detail = single_line(text.trim());
            state.fallback_reported = false;
            drop(state);
            self.shared.wake.notify_all();
        } else {
            self.out(text);
        }
    }

    fn progress(&self, label: &str, done: usize, total: usize) {
        let mut state = lock(&self.shared.state);
        state.fallback_reported = false;
        state.progress = Some(Progress {
            label: single_line(label),
            done: done.min(total),
            total,
        });
        drop(state);
        self.shared.wake.notify_all();
    }

    fn selection(&self, commits: &[&Commit], stack_up: bool, preview_limit: usize) -> bool {
        if !self.shared.rich {
            return false;
        }
        let width = output_width().saturating_sub(1);
        let mut text = format!(
            "\n{}\n{}\n\n",
            heading("Selected stack", width, self.shared.colour),
            paint(
                &format!(
                    "  {} commit{} · {}",
                    commits.len(),
                    if commits.len() == 1 { "" } else { "s" },
                    results::orientation(stack_up)
                ),
                "2",
                self.shared.colour
            )
        );
        let first = preview_limit.div_ceil(2);
        let last = commits.len().saturating_sub(preview_limit / 2);
        for (i, commit) in commits.iter().enumerate() {
            if preview_limit > 0 && commits.len() > preview_limit && (first..last).contains(&i) {
                if i == first {
                    let hidden = commits.len() - preview_limit;
                    let _ = writeln!(
                        text,
                        "  … {hidden} more commit{}",
                        if hidden == 1 { "" } else { "s" }
                    );
                }
                continue;
            }
            let prefix = format!("  {}  ", commit.short_hash());
            let title = clip(
                &single_line(&commit.title),
                width.saturating_sub(prefix.width()),
            );
            let _ = writeln!(text, "{prefix}{title}");
        }
        self.write_rich(&text);
        true
    }

    fn results(
        &self,
        commits: &[&Commit],
        repo: &Repository,
        stack_up: bool,
        mode: hyperlinks::Mode,
        summaries: &[crate::model::pull_request::PullRequestSummary],
        use_icon_font: bool,
    ) -> bool {
        if !self.shared.rich {
            return false;
        }
        self.write_rich(&results::render(
            commits,
            repo,
            stack_up,
            results::Options {
                columns: output_width().saturating_sub(1),
                colour: self.shared.colour,
                hyperlinks: mode.enabled(),
                summaries,
                use_icon_font,
            },
        ));
        true
    }

    fn start_activity(&self, label: &str, wait: Option<Duration>) -> u64 {
        self.begin_activity(label, "", wait)
    }

    fn start_activity_with_detail(&self, label: &str, detail: &str) -> u64 {
        self.begin_activity(label, detail, None)
    }

    fn end_activity(&self, id: u64) {
        let mut state = lock(&self.shared.state);
        state.activities.remove(&id);
        state.fallback_reported = false;
        if state.activities.is_empty() && state.stage.is_none() {
            self.clear(&mut state);
        }
        drop(state);
        self.shared.wake.notify_all();
    }

    fn quota(&self, resource: &str, remaining: u64, limit: u64, reset: Duration) {
        let mut state = lock(&self.shared.state);
        state.fallback_reported = false;
        state.quota = Some((
            format!("{} {remaining}/{limit}", single_line(resource)),
            Instant::now(),
            reset,
        ));
    }
}

impl ProgressTerminal {
    fn begin_activity(&self, label: &str, detail: &str, wait: Option<Duration>) -> u64 {
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
        state.fallback_reported = false;
        state.activities.insert(
            id,
            Activity {
                label,
                detail: single_line(detail),
                wait,
                started: Instant::now(),
            },
        );
        drop(state);
        self.shared.wake.notify_all();
        id
    }

    fn begin_stage(&self, label: &str, completed: Option<&str>) {
        let mut state = lock(&self.shared.state);
        self.end_stage(&mut state, true);
        state.fallback_reported = false;
        state.stage = Some(Activity {
            label: single_line(label),
            detail: String::new(),
            started: Instant::now(),
            wait: None,
        });
        state.completed_stage = completed.map(single_line);
        state.detail.clear();
        state.progress = None;
        drop(state);
        self.shared.wake.notify_all();
    }

    fn write_rich(&self, text: &str) {
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        self.shared.terminal.out(text);
    }
}

fn clear_rows(terminal: &dyn Terminal, state: &mut State) {
    if state.painted != 0 {
        terminal.err("\r\x1b[2K");
        for _ in 1..state.painted {
            terminal.err("\x1b[1A\r\x1b[2K");
        }
        state.painted = 0;
    }
}

fn animate(shared: &Shared) {
    let mut frame = 0;
    let mut next_frame = Instant::now();
    loop {
        let mut state = lock(&shared.state);
        if state.stopped || crate::commands::cancellation::interrupting() {
            return;
        }
        if state.paused || (state.activities.is_empty() && state.stage.is_none()) {
            drop(
                shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            continue;
        }
        let remaining = next_frame.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            drop(
                shared
                    .wake
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            continue;
        }
        paint_frame(shared, &mut state, frame, width());
        drop(state);
        frame = (frame + 1) % 10;
        next_frame = Instant::now() + Duration::from_millis(100);
    }
}

fn paint_frame(shared: &Shared, state: &mut State, frame: usize, columns: usize) {
    let text = render_state(state, frame, columns);
    clear_rows(shared.terminal.as_ref(), state);
    if state.painted_width != columns {
        state.fallback_reported = false;
    }
    let lines: Vec<_> = text.lines().collect();
    if lines.len() <= 2 && lines.iter().all(|line| line.width() < columns) {
        shared.terminal.err(&format!(
            "\r\x1b[2K{}",
            paint(&lines.join("\r\n"), "36", shared.colour)
        ));
        state.painted = lines.len();
    } else if !state.fallback_reported {
        // Preserve facts in narrow panes without making transient descriptions or
        // an animation frame permanent terminal history.
        if let Some(mut activity) = current_activity(state) {
            activity.detail.clear();
            let notice = render_text(
                &activity,
                state.progress.as_ref(),
                state.quota.as_ref(),
                usize::MAX,
            );
            shared.terminal.err(&format!("  {notice}\n"));
        }
        state.fallback_reported = true;
    }
    state.painted_width = columns;
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
    if label.width() + elapsed.width() + 6 > columns {
        // Completion labels and counts are facts; preserve them on deliberate rows.
        let available = columns.saturating_sub(4);
        let lines = complete_lines(label, available);
        return lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    format!("  {} {line}", paint(mark, code, colour))
                } else {
                    format!("    {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let padding = " ".repeat(label_columns.saturating_sub(label.width()));
    format!(
        "  {} {label}{padding}  {}",
        paint(mark, code, colour),
        paint(&elapsed, "2", colour)
    )
}

fn render_state(state: &State, frame: usize, width: usize) -> String {
    current_activity(state).map_or_else(String::new, |activity| {
        render(
            &activity,
            state.progress.as_ref(),
            state.quota.as_ref(),
            frame,
            width,
        )
    })
}

fn current_activity(state: &State) -> Option<Activity> {
    let latest = state.activities.last_key_value().map(|(_, value)| value);
    let wait = state.activities.values().rev().find(|a| a.wait.is_some());
    let active = wait.or(state.stage.as_ref()).or(latest)?;
    let mut label = if wait.is_none() && state.progress.is_some() {
        String::new()
    } else {
        active.label.clone()
    };
    let mut detail = String::new();
    if wait.is_none() {
        if let Some(detail) = latest.filter(|a| !std::ptr::eq(*a, active)) {
            if !label.is_empty() {
                label.push_str(" · ");
            }
            label.push_str(&detail.label);
        } else if !state.detail.is_empty() {
            detail = format!("· {}", state.detail);
        }
        if let Some(latest) = latest {
            if !detail.is_empty() && !latest.detail.is_empty() {
                detail.push(' ');
            }
            detail.push_str(&latest.detail);
        }
    }
    Some(Activity {
        label,
        detail,
        started: active.started,
        wait: active.wait,
    })
}

fn render(
    activity: &Activity,
    progress: Option<&Progress>,
    quota: Option<&(String, Instant, Duration)>,
    frame: usize,
    width: usize,
) -> String {
    let text = render_text(activity, progress, quota, width);
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][frame % 10];
    complete_lines(&text, width.saturating_sub(5))
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                format!("  {spinner} {line}")
            } else {
                format!("    {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_text(
    activity: &Activity,
    progress: Option<&Progress>,
    quota: Option<&(String, Instant, Duration)>,
    width: usize,
) -> String {
    let elapsed = activity.started.elapsed();
    let available = width.saturating_sub(5);
    let mut core = progress.map_or_else(
        || activity.label.clone(),
        |progress| format!("{}/{} {}", progress.done, progress.total, progress.label),
    );
    if progress.is_some() && !activity.label.is_empty() {
        let _ = write!(core, " · {}", activity.label);
    }
    let mut suffix = String::new();
    if let Some(wait) = activity.wait {
        let _ = write!(
            suffix,
            " · {} remaining",
            duration(wait.saturating_sub(elapsed))
        );
    }
    if let Some((label, observed, reset)) = quota {
        let _ = write!(
            suffix,
            " · {label} · reset {}",
            duration(reset.saturating_sub(observed.elapsed())).replace(' ', "")
        );
    }
    if !activity.detail.is_empty() {
        let budget = available.saturating_sub(core.width() + suffix.width() + 1);
        if budget != 0 {
            core.push(' ');
            core.push_str(&clip(&activity.detail, budget));
        }
    }
    core.push_str(&suffix);
    let mut detailed = core.clone();

    if activity.wait.is_none() {
        let _ = write!(detailed, " · {} elapsed", duration(elapsed));
    }
    if detailed.width() <= available {
        detailed
    } else {
        core
    }
}

fn width() -> usize {
    rustix::termios::tcgetwinsize(std::io::stderr())
        .ok()
        .filter(|size| size.ws_col != 0)
        .map_or(80, |size| usize::from(size.ws_col))
}

fn output_width() -> usize {
    rustix::termios::tcgetwinsize(std::io::stdout())
        .ok()
        .filter(|size| size.ws_col != 0)
        .map_or(80, |size| usize::from(size.ws_col))
}

fn heading(label: &str, columns: usize, colour: bool) -> String {
    complete_lines(&single_line(label), columns.saturating_sub(2))
        .iter()
        .map(|line| paint(&format!("  {line}"), "1", colour))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn paint(text: &str, code: &str, colour: bool) -> String {
    if colour {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.into()
    }
}

pub(super) fn single_line(text: &str) -> String {
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

#[cfg(test)]
#[path = "../tests/terminal_screen.rs"]
mod screen_tests;
