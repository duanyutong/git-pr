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
    network: Option<crate::commands::Network>,
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
    log: Option<Arc<crate::diagnostics::logs::InvocationLog>>,
    show_diagnostics: bool,
}

impl ProgressTerminal {
    pub(crate) fn with_log(
        terminal: Arc<dyn Terminal>,
        log: Option<Arc<crate::diagnostics::logs::InvocationLog>>,
        show_diagnostics: bool,
    ) -> Self {
        let mut progress = Self::new(terminal);
        progress.log = log;
        progress.show_diagnostics = show_diagnostics;
        progress
    }

    fn record(&self, text: &str) {
        if let Some(log) = &self.log {
            log.record(text);
        }
    }

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
            log: None,
            show_diagnostics: true,
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
        self.record(text);
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        if self.shared.rich {
            self.shared.terminal.out(&clean(text, self.shared.colour));
        } else {
            self.shared.terminal.out(text);
        }
    }

    fn err(&self, text: &str) {
        self.record(text);
        let mut state = lock(&self.shared.state);
        self.clear(&mut state);
        self.shared.terminal.err(&clean(text, self.shared.colour));
    }

    fn diagnostic(&self, text: &str) {
        if self.show_diagnostics {
            self.err(text);
        } else {
            self.record(text);
        }
    }

    fn records_diagnostics(&self) -> bool {
        self.log.is_some()
    }

    fn invocation_context(&self, repository: &str, commits: Option<usize>) {
        if let Some(log) = &self.log {
            log.context(repository, commits);
        }
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
        self.record(completed);
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
            self.record(text);
            let mut state = lock(&self.shared.state);
            state.detail = single_line(text.trim());
            state.fallback_reported = false;
            drop(state);
            self.shared.wake.notify_all();
        } else {
            self.out(text);
        }
    }

    fn has_progress(&self) -> bool {
        lock(&self.shared.state).progress.is_some()
    }

    fn clear_progress(&self, label: &str) {
        let mut state = lock(&self.shared.state);
        if state
            .progress
            .as_ref()
            .is_some_and(|progress| progress.label.starts_with(label))
        {
            state.progress = None;
        }
    }

    fn progress(&self, label: &str, done: usize, total: usize) {
        self.record(&format!(
            "Progress: {} {}/{}",
            single_line(label),
            done.min(total),
            total
        ));
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

    fn has_rich_results(&self) -> bool {
        self.shared.rich
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

    fn start_network_activity(
        &self,
        label: &str,
        network: crate::commands::Network,
        detail: &str,
    ) -> u64 {
        self.record(&format!(
            "Network activity started: {} · {} · {}",
            single_line(label),
            network.label(),
            single_line(detail)
        ));
        let id = self.begin_activity(label, &format!("{} · {detail}", network.label()), None);
        if let Some(activity) = lock(&self.shared.state).activities.get_mut(&id) {
            activity.network = Some(network);
        }
        id
    }

    fn start_activity_with_detail(&self, label: &str, detail: &str) -> u64 {
        self.begin_activity(label, detail, None)
    }

    fn end_activity(&self, id: u64) {
        let mut state = lock(&self.shared.state);
        if let Some(activity) = state.activities.remove(&id)
            && let Some(network) = activity.network
        {
            let notice = format!(
                "{} · {} · {:.1}s elapsed",
                activity.label,
                network.label(),
                activity.started.elapsed().as_secs_f64()
            );
            self.record(&format!("Network activity ended: {notice}"));
            if !self.shared.animated
                && activity.wait.is_none()
                && activity.started.elapsed() >= Duration::from_secs(1)
            {
                self.shared.terminal.err(&format!("{notice}\n"));
            }
        }
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
        let network = (wait.is_some() && label.starts_with("GitHub"))
            .then_some(crate::commands::Network::Api);
        state.activities.insert(
            id,
            Activity {
                label,
                detail: single_line(detail),
                wait,
                started: Instant::now(),
                network,
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
            network: None,
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
        let (columns, rows) = dimensions();
        paint_frame(shared, &mut state, frame, columns, rows);
        drop(state);
        frame = (frame + 1) % 10;
        next_frame = Instant::now() + Duration::from_millis(100);
    }
}

fn paint_frame(shared: &Shared, state: &mut State, frame: usize, columns: usize, rows: usize) {
    let text = render_state(state, frame, columns);
    clear_rows(shared.terminal.as_ref(), state);
    if state.painted_width != columns {
        state.fallback_reported = false;
    }
    let lines: Vec<_> = text.lines().collect();
    if lines.len() < rows && lines.iter().all(|line| line.width() < columns) {
        let colour = if current_activity(state).is_some_and(|activity| activity.network.is_some()) {
            "34"
        } else {
            "36"
        };
        shared.terminal.err(&format!(
            "\r\x1b[2K{}",
            paint(&lines.join("\r\n"), colour, shared.colour)
        ));
        state.painted = lines.len();
    } else if !state.fallback_reported {
        // Preserve facts in narrow panes without making transient descriptions or
        // an animation frame permanent terminal history.
        if let Some(mut activity) = current_activity(state) {
            activity.detail.clear();
            let notice = if activity.network.is_some() {
                render_network(
                    &activity,
                    state.progress.as_ref(),
                    state
                        .quota
                        .as_ref()
                        .filter(|_| activity.network != Some(crate::commands::Network::Git)),
                    usize::MAX,
                )
            } else {
                render_text(
                    &activity,
                    state.progress.as_ref(),
                    state.quota.as_ref(),
                    usize::MAX,
                )
            };
            for line in complete_lines(&notice, columns.saturating_sub(3)) {
                shared.terminal.err(&format!("  {line}\n"));
            }
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
            state
                .quota
                .as_ref()
                .filter(|_| activity.network != Some(crate::commands::Network::Git)),
            frame,
            width,
        )
    })
}

fn current_activity(state: &State) -> Option<Activity> {
    let latest = state.activities.last_key_value().map(|(_, value)| value);
    let wait = state.activities.values().rev().find(|a| a.wait.is_some());
    let active = wait
        .or_else(|| {
            state
                .activities
                .values()
                .rev()
                .find(|activity| activity.network.is_some())
        })
        .or(state.stage.as_ref())
        .or(latest)?;
    let mut label = if wait.is_none() && state.progress.is_some() && active.network.is_none() {
        String::new()
    } else {
        active.label.clone()
    };
    let mut detail = String::new();
    if wait.is_none() && active.network.is_some() {
        detail.clone_from(&active.detail);
        if !state.detail.is_empty() {
            detail.push_str(" · ");
            detail.push_str(&state.detail);
        }
    } else if wait.is_none() {
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
        network: active.network.or_else(|| {
            wait.and_then(|_| {
                state
                    .activities
                    .values()
                    .rev()
                    .find_map(|activity| activity.network)
            })
        }),
    })
}

fn render(
    activity: &Activity,
    progress: Option<&Progress>,
    quota: Option<&(String, Instant, Duration)>,
    frame: usize,
    width: usize,
) -> String {
    let text = if activity.network.is_some() {
        render_network(activity, progress, quota, width)
    } else {
        render_text(activity, progress, quota, width)
    };
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][frame % 10];
    text.lines()
        .flat_map(|line| complete_lines(line, width.saturating_sub(5)))
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

fn render_network(
    activity: &Activity,
    progress: Option<&Progress>,
    quota: Option<&(String, Instant, Duration)>,
    width: usize,
) -> String {
    let available = width.saturating_sub(5);
    let label = progress.map_or(activity.label.as_str(), |progress| progress.label.as_str());
    let count = progress.map_or_else(String::new, |progress| {
        format!("{}/{} ", progress.done, progress.total)
    });
    let elapsed = activity.wait.map_or_else(
        || format!(" · {} elapsed", duration(activity.started.elapsed())),
        |wait| {
            format!(
                " · {} remaining",
                duration(wait.saturating_sub(activity.started.elapsed()))
            )
        },
    );
    let primary = format!(
        "{count}{}{elapsed}",
        clip(
            label,
            available.saturating_sub(count.width() + elapsed.width())
        )
    );
    let primary = if primary.width() <= available {
        primary
    } else {
        format!(
            "{count}{}",
            clip(label, available.saturating_sub(count.width()))
        )
    };
    let mut detail = if activity.wait.is_some() {
        let full = format!("Waiting · {}", activity.label);
        if full.width() <= available {
            full
        } else {
            format!(
                "Waiting · {}",
                activity
                    .label
                    .strip_prefix("GitHub ")
                    .unwrap_or(&activity.label)
            )
        }
    } else {
        activity.detail.clone()
    };
    if activity.network == Some(crate::commands::Network::Api)
        && let Some((label, _, _)) = quota
    {
        let resource = label.split_whitespace().next().unwrap_or_default();
        if activity.wait.is_some()
            || activity
                .detail
                .starts_with(&format!("GitHub API · {resource}"))
        {
            let candidate = format!("{detail} · {label}");
            if candidate.width() <= available {
                detail = candidate;
            }
        }
    }
    if detail.is_empty() {
        primary
    } else {
        format!("{primary}\n{}", clip(&detail, available))
    }
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
    let combined = format!("{core}{suffix}");
    let mut detailed = combined.clone();

    if activity.wait.is_none() {
        let _ = write!(detailed, " · {} elapsed", duration(elapsed));
    }
    if detailed.width() <= available {
        detailed
    } else if combined.width() <= available || suffix.is_empty() {
        combined
    } else {
        // Quota and reset belong together; do not let a greedy word wrap strand
        // the resource name or split the reset deadline across the primary row.
        format!("{core}\n{}", suffix.trim_start_matches(" · "))
    }
}

fn width() -> usize {
    dimensions().0
}

fn dimensions() -> (usize, usize) {
    rustix::termios::tcgetwinsize(std::io::stderr())
        .ok()
        .map_or((80, 24), |size| {
            (
                if size.ws_col == 0 {
                    80
                } else {
                    usize::from(size.ws_col)
                },
                if size.ws_row == 0 {
                    24
                } else {
                    usize::from(size.ws_row)
                },
            )
        })
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

pub(crate) fn single_line(text: &str) -> String {
    clean(text, false).replace(['\n', '\t'], " ")
}

/// Retain generated SGR styling only where enabled; make other terminal controls inert.
pub(crate) fn clean(text: &str, colour: bool) -> String {
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
