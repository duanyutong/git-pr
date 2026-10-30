//! Independent terminal-screen contracts for transient output and its lifecycle.

use super::*;
use crate::cli::Flags;
use crate::tests::unit::support::World;
use proptest::prelude::*;
use std::num::NonZeroU16;

const SPINNERS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
const COMPLETION: &str = "Native stack already up to date";
const SCREEN_ROWS: u16 = 24;

struct ScreenTerminal {
    parser: Mutex<vt100::Parser>,
    output: Mutex<String>,
}

impl std::fmt::Debug for ScreenTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenTerminal").finish_non_exhaustive()
    }
}

impl ScreenTerminal {
    fn new(columns: u16) -> Arc<Self> {
        Self::with_size(columns, SCREEN_ROWS)
    }

    fn with_size(columns: u16, rows: u16) -> Arc<Self> {
        Arc::new(Self {
            parser: Mutex::new(vt100::Parser::new(
                NonZeroU16::new(rows).unwrap(),
                NonZeroU16::new(columns).unwrap(),
                0,
            )),
            output: Mutex::default(),
        })
    }

    fn contents(&self) -> String {
        lock(&self.parser).screen().contents()
    }

    fn resize(&self, columns: u16) {
        lock(&self.parser).set_size(
            NonZeroU16::new(SCREEN_ROWS).unwrap(),
            NonZeroU16::new(columns).unwrap(),
        );
    }
}

impl Terminal for ScreenTerminal {
    fn out(&self, text: &str) {
        self.err(text);
        if !text.ends_with('\n') {
            self.err("\n");
        }
    }

    fn err(&self, text: &str) {
        lock(&self.output).push_str(text);
        // Match the ONLCR newline processing of the native pseudo-terminal.
        lock(&self.parser).process(text.replace('\n', "\r\n").as_bytes());
    }

    fn confirm(&self, prompt: &str) -> bool {
        self.err(prompt);
        false
    }
}

fn controlled(screen: &Arc<ScreenTerminal>, colour: bool) -> ProgressTerminal {
    ProgressTerminal {
        shared: Arc::new(Shared {
            terminal: screen.clone(),
            state: Mutex::default(),
            wake: Condvar::new(),
            animated: true,
            rich: true,
            colour,
        }),
        worker: Mutex::new(None),
    }
}

fn repaint(progress: &ProgressTerminal, columns: u16) {
    paint_frame(
        &progress.shared,
        &mut lock(&progress.shared.state),
        1,
        usize::from(columns),
        usize::from(SCREEN_ROWS),
    );
}

fn assert_no_transient(screen: &str) {
    assert!(
        !screen.chars().any(|character| SPINNERS.contains(character)),
        "{screen}"
    );
}

fn visible_lines(screen: &str) -> Vec<&str> {
    screen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn assert_summary_line(line: &str, label: &str) {
    let remainder = line
        .strip_prefix(label)
        .unwrap_or_else(|| panic!("expected {label:?}: {line}"));
    let elapsed = remainder.trim();
    assert!(
        elapsed.is_empty()
            || elapsed.strip_suffix('s').is_some_and(|seconds| {
                seconds
                    .parse::<f64>()
                    .is_ok_and(|seconds| seconds.is_finite() && seconds >= 0.0)
            }),
        "unexpected summary residue: {line}"
    );
}

#[derive(Debug, Default)]
struct WorkflowDetails(Mutex<Vec<String>>);

impl Terminal for WorkflowDetails {
    fn out(&self, _: &str) {}
    fn err(&self, _: &str) {}
    fn confirm(&self, _: &str) -> bool {
        false
    }
    fn detail(&self, text: &str) {
        lock(&self.0).push(text.into());
    }
}

fn workflow_details() -> Vec<String> {
    let world = World::new(9);
    let details = Arc::new(WorkflowDetails::default());
    for _ in 0..2 {
        let mut app = world.app();
        app.commands.terminal = details.clone();
        app.execute(
            &Flags::parse(vec!["--github-stack".into(), "--yes".into()]).unwrap(),
            &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
        )
        .unwrap();
    }
    let recorded = lock(&details.0).clone();
    assert!(
        recorded
            .iter()
            .any(|text| text.starts_with("GitHub Stack unchanged:"))
    );
    assert!(
        recorded
            .iter()
            .any(|text| text.starts_with("GitHub Stack updated:"))
    );
    recorded
}

#[test]
fn workflow_details_leave_only_durable_output_on_the_terminal_screen() {
    for text in workflow_details() {
        for columns in [40, 80, 124] {
            for colour in [false, true] {
                let screen = ScreenTerminal::new(columns);
                let progress = controlled(&screen, colour);
                progress.out("EARLIER OUTPUT KEPT");
                progress.stage_with_completion("Linking native stack", COMPLETION);
                progress.quota("REST", 4835, 5000, Duration::from_secs(1128));
                progress.detail(&text);
                repaint(&progress, columns);
                let state = lock(&progress.shared.state);
                assert!(
                    state.painted > 0 && state.painted <= 2,
                    "{text}: {}",
                    screen.contents()
                );
                assert!(!state.fallback_reported, "{text}");
                drop(state);
                assert_eq!(
                    lock(&screen.parser).screen().cell(1, 2).unwrap().fgcolor(),
                    if colour {
                        vt100::Color::Idx(6)
                    } else {
                        vt100::Color::Default
                    },
                    "{text}"
                );
                progress.finish(true);
                progress.out("RESULTS KEPT");
                let contents = screen.contents();
                assert_no_transient(&contents);
                assert!(!contents.contains("GitHub Stack"), "{contents}");
                assert!(contents.contains(COMPLETION), "{contents}");
                assert!(contents.contains("EARLIER OUTPUT KEPT"), "{contents}");
                assert!(contents.contains("RESULTS KEPT"), "{contents}");
                let lines = visible_lines(&contents);
                assert_eq!(lines.len(), 3, "{contents}");
                assert_eq!(lines[0], "EARLIER OUTPUT KEPT");
                assert_summary_line(lines[1], &format!("✓ {COMPLETION}"));
                assert_eq!(lines[2], "RESULTS KEPT");
            }
        }
    }
}

#[test]
fn wrapped_creation_wait_is_replaced_by_the_current_request_and_cleared_on_completion() {
    for columns in [40, 80, 124] {
        for colour in [false, true] {
            let screen = ScreenTerminal::new(columns);
            let progress = controlled(&screen, colour);
            progress.out("EARLIER OUTPUT KEPT");
            progress.stage_with_completion("Reconciling pull requests", "Pull requests reconciled");
            progress.progress(
                "Creating feat(example): extract reusable validation utilities",
                1,
                4,
            );
            progress.quota("REST", 4700, 5000, Duration::from_secs(90));
            let wait =
                progress.start_activity("GitHub mutation spacing", Some(Duration::from_secs(1)));
            repaint(&progress, columns);
            assert!(
                !lock(&progress.shared.state).fallback_reported,
                "{}",
                screen.contents()
            );
            assert!(screen.contents().contains("mutation spacing"));
            progress.end_activity(wait);
            progress.progress("Creating feat(example): finish", 3, 4);
            let request = progress.start_activity("GitHub POST", None);
            repaint(&progress, columns);
            let current = screen.contents();
            assert!(
                !current.contains("1/4") && !current.contains("mutation spacing"),
                "{current}"
            );
            assert!(
                current.contains("3/4") && current.contains("GitHub POST"),
                "{current}"
            );
            progress.end_activity(request);
            progress.progress("Creating PR", 4, 4);
            progress.finish(true);
            progress.out("RESULTS KEPT");
            let completed = screen.contents();
            assert_no_transient(&completed);
            assert!(
                !completed.contains("Creating") && !completed.contains("REST"),
                "{completed}"
            );
            assert!(
                completed.contains("EARLIER OUTPUT KEPT") && completed.contains("RESULTS KEPT"),
                "{completed}"
            );
            assert!(
                completed.contains("Pull requests reconciled"),
                "{completed}"
            );
        }
    }
}

#[test]
fn creation_wait_terminal_fixture() {
    if std::env::var_os("GIT_PR_TEST_CREATION_WAIT").is_none() {
        return;
    }
    let terminal = ProgressTerminal::new(Arc::new(crate::commands::Console::default()));
    terminal.out("EARLIER OUTPUT KEPT");
    terminal.stage_with_completion("Reconciling pull requests", "Pull requests reconciled");
    terminal.progress(
        "Creating feat(example): extract reusable validation utilities",
        1,
        4,
    );
    terminal.quota("REST", 4700, 5000, Duration::from_secs(90));
    let wait = terminal.start_activity("GitHub mutation spacing", Some(Duration::from_secs(1)));
    let mut permission = String::new();
    std::io::stdin().read_line(&mut permission).unwrap();
    assert_eq!(permission, "next\n");
    terminal.end_activity(wait);
    terminal.progress("Creating feat(example): finish", 3, 4);
    let request = terminal.start_activity("GitHub POST", None);
    permission.clear();
    std::io::stdin().read_line(&mut permission).unwrap();
    assert_eq!(permission, "done\n");
    terminal.end_activity(request);
    terminal.progress("Creating PR", 4, 4);
    terminal.finish(true);
    terminal.out("RESULTS KEPT");
}

#[test]
fn native_creation_wait_frames_leave_no_stale_status() {
    #[derive(serde::Deserialize)]
    struct Capture {
        columns: u16,
        current: Vec<u8>,
        completed: Vec<u8>,
    }
    let script = r"
import fcntl, json, os, pty, re, select, struct, subprocess, sys, termios, time
captures = []
for columns in [40, 80, 124]:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, columns, 0, 0))
    env = dict(os.environ, TERM='xterm', GIT_PR_TEST_CREATION_WAIT='1')
    env.pop('CI', None)
    env.pop('NO_COLOR', None)
    process = subprocess.Popen([sys.argv[1], '--exact', 'cli::progress::screen_tests::creation_wait_terminal_fixture', '--nocapture'], stdin=subprocess.PIPE, stdout=slave, stderr=slave, env=env)
    os.close(slave)
    output = b''
    def receive(predicate):
        global output
        deadline = time.monotonic() + 8
        while not predicate(output) and time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try: data = os.read(master, 65536)
                except OSError: break
                if not data: break
                output += data
        assert predicate(output), output[-3000:]
    def has_frame(counter):
        return lambda data: any(counter in frame and b'4700/5000' in frame for frame in re.findall(rb'\x1b\[36m(.*?)\x1b\[0m', data, re.S))
    try:
        receive(has_frame(b'1/4'))
        process.stdin.write(b'next\n'); process.stdin.flush()
        receive(has_frame(b'3/4'))
        current = list(output)
        process.stdin.write(b'done\n'); process.stdin.flush()
        receive(lambda data: b'RESULTS KEPT' in data)
        process.communicate(timeout=5)
        assert process.returncode == 0
        captures.append({'columns': columns, 'current': current, 'completed': list(output)})
    finally:
        if process.poll() is None: process.kill(); process.wait()
        os.close(master)
print(json.dumps(captures))
";
    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captures: Vec<Capture> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(captures.len(), 3);
    for capture in captures {
        let screen = ScreenTerminal::new(capture.columns);
        lock(&screen.parser).process(&capture.current);
        let current = screen.contents();
        assert!(
            !current.contains("1/4") && !current.contains("mutation spacing"),
            "{current}"
        );
        assert!(current.contains("3/4"), "{current}");
        let screen = ScreenTerminal::new(capture.columns);
        lock(&screen.parser).process(&capture.completed);
        let completed = screen.contents();
        assert_no_transient(&completed);
        assert!(
            !completed.contains("Creating") && !completed.contains("4700/5000"),
            "{completed}"
        );
        assert!(
            completed.contains("EARLIER OUTPUT KEPT") && completed.contains("RESULTS KEPT"),
            "{completed}"
        );
    }
}

#[test]
fn oversized_operational_facts_never_leave_a_permanent_spinner() {
    let screen = ScreenTerminal::new(20);
    let progress = controlled(&screen, false);
    progress.stage("Checking publication UNBREAKABLE_IDENTITY_WIDER_THAN_THE_PANE");
    progress.detail("TRANSIENT DETAIL MUST DISAPPEAR");
    repaint(&progress, 20);
    assert!(lock(&progress.shared.state).fallback_reported);
    progress.finish(true);
    let contents = screen.contents();
    assert_no_transient(&contents);
    assert!(!contents.contains("TRANSIENT"), "{contents}");
}

#[test]
fn narrow_live_status_keeps_progress_and_quota_on_deliberate_rows() {
    for (label, columns) in [
        ("Descriptions #101", 40),
        ("Descriptions #101, #102", 40),
        ("Descriptions #101, #102", 50),
        ("Descriptions #101, #102", 64),
    ] {
        let screen = ScreenTerminal::new(columns);
        let progress = controlled(&screen, true);
        progress.out("EARLIER OUTPUT KEPT");
        progress.stage("Updating descriptions");
        progress.progress(label, 1, 3);
        progress.quota("GraphQL", 4990, 5000, Duration::from_secs(90));
        repaint(&progress, columns);
        let live = screen.contents();
        let lines = visible_lines(&live);
        assert_eq!(lines.len(), 3, "{live}");
        assert_eq!(lines[1], format!("⠙ 1/3 {label}"), "{live}");
        assert!(lines[2].starts_with("GraphQL 4990/5000 · reset "), "{live}");
        assert_eq!(lock(&progress.shared.state).painted, 2);
        assert!(!lock(&progress.shared.state).fallback_reported);
        progress.finish(true);
        progress.out("RESULTS KEPT");
        let completed = screen.contents();
        assert_no_transient(&completed);
        assert!(!completed.contains("GraphQL"), "{completed}");
        assert!(completed.contains("EARLIER OUTPUT KEPT"), "{completed}");
        assert!(completed.contains("RESULTS KEPT"), "{completed}");
    }
}

#[test]
fn multiline_live_status_wraps_complete_messages_before_the_terminal_does() {
    let terminal = ScreenTerminal::new(30);
    let progress = controlled(&terminal, false);
    progress.stage("Updating descriptions across the selected pull requests");
    progress.progress("Descriptions #101, #102", 1, 3);
    progress.quota("GraphQL", 4990, 5000, Duration::from_secs(90));
    paint_frame(
        &progress.shared,
        &mut lock(&progress.shared.state),
        0,
        30,
        usize::from(SCREEN_ROWS),
    );
    let output = lock(&terminal.output).clone();
    let frame = output.strip_prefix("\r\x1b[2K").unwrap();
    assert!(frame.lines().all(|line| line.width() < 30), "{output}");
    assert!(frame.lines().count() > 2, "{output}");
    let text = frame.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(text.contains("1/3 Descriptions #101, #102"), "{text}");
    assert!(text.contains("GraphQL 4990/5000 · reset"), "{text}");
    assert!(!lock(&progress.shared.state).fallback_reported);
    assert!(lock(&progress.shared.state).painted > 2);
    progress.finish(true);
    assert_no_transient(&terminal.contents());
    assert!(
        !terminal.contents().contains("GraphQL"),
        "{}",
        terminal.contents()
    );
}

#[test]
fn transient_frames_respect_the_available_terminal_height() {
    for rows in [3, 4, 12] {
        let screen = ScreenTerminal::with_size(30, rows);
        let progress = controlled(&screen, false);
        progress.stage("Working");
        progress.progress("Processing selected pull requests", 1, 4);
        progress.quota("REST", 4700, 5000, Duration::from_secs(90));
        let required = render_state(&lock(&progress.shared.state), 0, 30)
            .lines()
            .count();
        paint_frame(
            &progress.shared,
            &mut lock(&progress.shared.state),
            0,
            30,
            usize::from(rows),
        );
        let state = lock(&progress.shared.state);
        assert_eq!(state.fallback_reported, required >= usize::from(rows));
        assert!(state.painted < usize::from(rows));
        drop(state);
        progress.finish(true);
        assert_no_transient(&screen.contents());
    }
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn arbitrary_details_cannot_escape_the_transient_screen_region(
        columns in 40_u16..180,
        resized in 40_u16..180,
        tokens in prop::collection::vec(prop_oneof![
            Just("branch/long-name "), Just("日本語 "), Just("🦀"), Just("e\u{301}"), Just("x"),
            Just("\n"), Just("\t"), Just("\x1b[2K")
        ], 0..150),
        colour in any::<bool>(),
        success in any::<bool>(),
        exit in 0_u8..4,
    ) {
        let screen = ScreenTerminal::new(columns);
        let progress = controlled(&screen, colour);
        progress.out("EARLIER OUTPUT KEPT");
        progress.stage_with_completion("Working", "Completed");
        progress.progress("PR #101", 1, 3);
        progress.quota("REST", 4835, 5000, Duration::from_secs(1128));
        progress.detail(&tokens.concat());
        repaint(&progress, columns);
        screen.resize(resized);
        progress.detail(&format!("Changed {}", tokens.concat()));
        repaint(&progress, resized);
        prop_assert!(!lock(&progress.shared.state).fallback_reported);
        let live = screen.contents();
        prop_assert!(live.contains("1/3 PR #101"), "{live}");
        prop_assert!(live.contains("4835/5000"), "{live}");
        match exit {
            0 => progress.finish(success),
            1 => progress.out("DURABLE OUTPUT KEPT"),
            2 => progress.err("DURABLE WARNING KEPT\n"),
            _ => { prop_assert!(!progress.confirm("Continue?\n")); },
        }
        drop(progress);
        let contents = screen.contents();
        assert_no_transient(&contents);
        prop_assert!(contents.contains("EARLIER OUTPUT KEPT"), "{contents}");
        prop_assert!(!contents.contains("Changed"), "{contents}");
        if exit == 1 { prop_assert!(contents.contains("DURABLE OUTPUT KEPT"), "{contents}"); }
        if exit == 2 { prop_assert!(contents.contains("DURABLE WARNING KEPT"), "{contents}"); }
        let lines = visible_lines(&contents);
        prop_assert_eq!(lines.len(), 2, "{}", contents);
        prop_assert_eq!(lines[0], "EARLIER OUTPUT KEPT");
        match exit {
            0 => assert_summary_line(lines[1], if success { "✓ Completed · 1/3" } else { "✗ 1/3 PR #101 · Working" }),
            1 => prop_assert_eq!(lines[1], "DURABLE OUTPUT KEPT"),
            2 => prop_assert_eq!(lines[1], "DURABLE WARNING KEPT"),
            _ => prop_assert_eq!(lines[1], "Continue?"),
        }
    }
}

fn property_config() -> proptest::test_runner::Config {
    let mut config = proptest::test_runner::Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 128;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        config.rng_seed = proptest::test_runner::RngSeed::Fixed(0x4750_5241_5353_5552);
    }
    config.failure_persistence = Some(Box::new(
        proptest::test_runner::FileFailurePersistence::Direct(
            "proptest-regressions/tests/terminal_screen.txt",
        ),
    ));
    config.max_shrink_iters = 4096;
    config
}

#[test]
fn workflow_detail_terminal_fixture() {
    let Ok(detail) = std::env::var("GIT_PR_TEST_SCREEN_DETAIL") else {
        return;
    };
    let terminal = ProgressTerminal::new(Arc::new(crate::commands::Console::default()));
    terminal.out("EARLIER OUTPUT KEPT");
    terminal.stage_with_completion("Linking native stack", COMPLETION);
    terminal.quota("REST", 4835, 5000, Duration::from_secs(1128));
    terminal.detail(&detail);
    let mut permission = String::new();
    std::io::stdin().read_line(&mut permission).unwrap();
    assert_eq!(permission, "done\n");
    terminal.finish(true);
    terminal.out("RESULTS KEPT");
}

#[test]
fn native_terminal_capture_has_no_transient_screen_residue() {
    #[derive(serde::Deserialize)]
    struct Capture {
        columns: u16,
        bytes: Vec<u8>,
    }
    let detail = workflow_details()
        .into_iter()
        .find(|text| text.starts_with("GitHub Stack unchanged:"))
        .unwrap();
    let script = r"
import fcntl, json, os, pty, select, struct, subprocess, sys, termios, time
captures = []
for columns in [40, 80, 124]:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, columns, 0, 0))
    env = dict(os.environ, TERM='xterm')
    env.pop('CI', None)
    env.pop('NO_COLOR', None)
    p = subprocess.Popen([sys.argv[1], '--exact', 'cli::progress::screen_tests::workflow_detail_terminal_fixture', '--nocapture'], stdin=subprocess.PIPE, stdout=slave, stderr=slave, env=env)
    os.close(slave)
    output = b''
    try:
        deadline = time.monotonic() + 8
        while b'4835/5000' not in output and time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                output += os.read(master, 65536)
        assert b'REST' in output and b'4835/5000' in output, output[-2000:]
        assert b'\x1b[36m' in output, output
        p.stdin.write(b'done\n')
        p.stdin.flush()
        deadline = time.monotonic() + 5
        while b'RESULTS KEPT' not in output and time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try: data = os.read(master, 65536)
                except OSError: break
                if not data: break
                output += data
        p.communicate(timeout=5)
        assert p.returncode == 0
        assert b'RESULTS KEPT' in output, output
        captures.append({'columns': columns, 'bytes': list(output)})
    finally:
        if p.poll() is None: p.kill(); p.wait()
        os.close(master)
print(json.dumps(captures))
";
    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .env("GIT_PR_TEST_SCREEN_DETAIL", detail)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let captures: Vec<Capture> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(captures.len(), 3);
    for capture in captures {
        let screen = ScreenTerminal::new(capture.columns);
        lock(&screen.parser).process(&capture.bytes);
        let contents = screen.contents();
        assert_no_transient(&contents);
        assert!(!contents.contains("GitHub Stack"), "{contents}");
        assert!(contents.contains(COMPLETION), "{contents}");
        assert!(contents.contains("EARLIER OUTPUT KEPT"), "{contents}");
        assert!(contents.contains("RESULTS KEPT"), "{contents}");
        let body = contents
            .split_once("EARLIER OUTPUT KEPT")
            .unwrap()
            .1
            .split_once("RESULTS KEPT")
            .unwrap()
            .0;
        let lines = visible_lines(body);
        assert_eq!(lines.len(), 1, "{contents}");
        assert_summary_line(lines[0], &format!("✓ {COMPLETION}"));
    }
}

#[test]
fn redirected_workflow_detail_retains_complete_text() {
    let world = World::new(0);
    let progress = ProgressTerminal::with_options(world.clone(), false, false, false);
    for text in workflow_details() {
        progress.detail(&text);
        let memory = lock(&world.0);
        assert!(memory.stdout.contains(&text), "{text}");
        assert!(!memory.stdout.contains('\x1b'));
        drop(memory);
    }
}

#[test]
fn nested_request_wait_and_stage_transition_leave_no_screen_residue() {
    let screen = ScreenTerminal::new(80);
    let progress = controlled(&screen, true);
    progress.out("EARLIER OUTPUT KEPT");
    progress.stage_with_completion("Preparing commits", "Commits prepared");
    progress.detail(&"Generic descriptive detail ".repeat(40));
    let request =
        progress.start_activity_with_detail("GitHub GET", &"example/endpoint/".repeat(40));
    repaint(&progress, 80);
    let wait = progress.start_activity("Retry 1/3", Some(Duration::from_secs(65)));
    repaint(&progress, 80);
    assert!(screen.contents().contains("Retry 1/3"));
    assert!(screen.contents().contains("remaining"));
    progress.end_activity(wait);
    progress.end_activity(request);
    progress.stage_with_completion("Linking native stack", COMPLETION);
    progress.detail(&"GitHub Stack unchanged: branch/example ".repeat(40));
    repaint(&progress, 80);
    progress.finish(true);
    let contents = screen.contents();
    assert_no_transient(&contents);
    assert!(contents.contains("EARLIER OUTPUT KEPT"), "{contents}");
    assert!(contents.contains("Commits prepared"), "{contents}");
    assert!(contents.contains(COMPLETION), "{contents}");
    assert!(
        !contents.contains("Generic") && !contents.contains("endpoint"),
        "{contents}"
    );
    assert!(!contents.contains("GitHub Stack"), "{contents}");
}

#[test]
fn compound_emoji_and_combining_details_respect_grapheme_widths() {
    // vt100 models codepoint cells rather than joined emoji; these layout
    // assertions complement its independent screen-lifecycle oracle.
    for text in ["👩‍💻", "e\u{301}", "日本語"] {
        let screen = ScreenTerminal::new(80);
        let progress = controlled(&screen, false);
        progress.stage("Working");
        progress.quota("REST", 4835, 5000, Duration::from_secs(1128));
        progress.detail(&text.repeat(300));
        for columns in 40..=200 {
            let row = render_state(&lock(&progress.shared.state), 0, columns);
            assert!(row.lines().all(|line| line.width() < columns), "{row}");
            assert!(row.lines().count() <= 2, "{row}");
            assert!(
                !row.contains("👩…") && !row.contains("👩\u{200d}…"),
                "{row}"
            );
        }
    }
}
