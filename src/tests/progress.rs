use super::*;
#[derive(Debug, Default)]
struct MemoryTerminal {
    stdout: Mutex<String>,
    stderr: Mutex<String>,
    answers: Mutex<Vec<bool>>,
}
impl Terminal for MemoryTerminal {
    fn out(&self, text: &str) {
        let mut output = self.stdout.lock().unwrap();
        output.push_str(text);
        if !text.ends_with('\n') {
            output.push('\n');
        }
    }
    fn err(&self, text: &str) {
        self.stderr.lock().unwrap().push_str(text);
    }
    fn confirm(&self, prompt: &str) -> bool {
        self.err(prompt);
        self.answers.lock().unwrap().pop().unwrap_or(false)
    }
}

#[test]
fn checked_outcomes_replace_generic_completion_counts_in_plain_and_live_output() {
    for animated in [false, true] {
        let terminal = Arc::new(MemoryTerminal::default());
        let progress = ProgressTerminal::with_animation(terminal.clone(), animated);
        progress.stage_with_completion("Updating descriptions", "Reconciled descriptions");
        progress.progress("Descriptions", 2, 2);
        progress.outcome("PR descriptions: 2 unchanged");
        progress.finish(true);
        drop(progress);
        let stderr = terminal.stderr.lock().unwrap();
        assert!(stderr.contains("PR descriptions: 2 unchanged"));
        assert!(!stderr.contains("Stack context prepared"));
        assert!(!stderr.contains("Reconciled descriptions"));
        assert!(!stderr.contains("· 2/2"));
        if !animated {
            assert!(!stderr.contains('\x1b') && !stderr.contains('\r'));
        }
        drop(stderr);
        assert_eq!(*terminal.stdout.lock().unwrap(), "");
    }
}

#[test]
fn redirected_progress_reports_waits_without_control_sequences() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), false);
    let activity = progress.start_activity("Reading PRs", None);
    progress.quota("GraphQL", 40, 5000, Duration::from_secs(70));
    let wait = progress.start_activity(
        "GitHub rate-limit retry; retry 1/3",
        Some(Duration::from_secs(65)),
    );
    progress.end_activity(wait);
    progress.out("results");
    progress.err("diagnostic\n");
    assert!(!progress.confirm("continue?"));
    assert!(progress.failure().is_none());
    progress.end_activity(activity);
    drop(progress);
    let stderr = terminal.stderr.lock().unwrap();
    assert!(stderr.contains("waiting 1m 05s"));
    assert!(!stderr.contains('\r'));
    assert!(!stderr.contains('\x1b'));
    drop(stderr);
    assert_eq!(*terminal.stdout.lock().unwrap(), "results\n");
}

#[test]
fn live_progress_clears_before_output_and_terminates_its_worker() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), true);
    let activity = progress.start_activity("Updating descriptions", Some(Duration::from_secs(65)));
    progress.quota("GraphQL", 4990, 5000, Duration::from_secs(90));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !terminal.stderr.lock().unwrap().contains("remaining") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(terminal.stderr.lock().unwrap().contains("remaining"));
    progress.out("published results");
    progress.err("durable diagnostic\n");
    terminal.answers.lock().unwrap().push(true);
    assert!(progress.confirm("continue?"));
    progress.end_activity(activity);
    drop(progress);
    let stderr = terminal.stderr.lock().unwrap();
    assert!(stderr.contains("\r\x1b[2K"));
    assert!(stderr.contains("durable diagnostic\n"));
    drop(stderr);
}

#[test]
fn frequent_progress_updates_are_coalesced_without_delaying_completion() {
    #[derive(Debug, Default)]
    struct FrameTerminal {
        terminal: MemoryTerminal,
        frames: Mutex<Vec<Instant>>,
    }
    impl Terminal for FrameTerminal {
        fn out(&self, text: &str) {
            self.terminal.out(text);
        }
        fn err(&self, text: &str) {
            if text.starts_with("\r\x1b[2K") && text.len() > 5 {
                self.frames.lock().unwrap().push(Instant::now());
            }
            self.terminal.err(text);
        }
        fn confirm(&self, prompt: &str) -> bool {
            self.terminal.confirm(prompt)
        }
    }

    let terminal = Arc::new(FrameTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), true);
    progress.stage("Updating descriptions");
    // Observe both initial frames before the burst, including under a busy scheduler.
    for done in 0..2 {
        progress.progress("Descriptions #101", done, 100);
        let expected = format!("{done}/100 Descriptions #101");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !terminal.terminal.stderr.lock().unwrap().contains(&expected)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(terminal.terminal.stderr.lock().unwrap().contains(&expected));
    }
    for done in 2..100 {
        progress.progress("Descriptions #101", done, 100);
        std::thread::sleep(Duration::from_millis(1));
    }
    progress.progress("Descriptions #101", 100, 100);
    // Completion is synchronous; it must not wait for the next permitted animation frame.
    progress.finish(true);
    let frames = terminal.frames.lock().unwrap();
    assert!(frames.len() >= 2);
    assert!(
        frames
            .windows(2)
            .all(|pair| pair[1].duration_since(pair[0]) >= Duration::from_millis(95)),
        "{} redraws in {:?}",
        frames.len(),
        frames.last().unwrap().duration_since(frames[0])
    );
    drop(frames);
    assert!(
        terminal
            .terminal
            .stderr
            .lock()
            .unwrap()
            .contains("✓ 100/100 Descriptions #101")
    );
}

#[test]
fn worker_creation_failure_reports_the_reason_and_retains_plain_output() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_worker(terminal.clone(), true, true, false, |_| {
        Err(std::io::Error::other("fixture thread capacity exhausted"))
    });
    progress.stage("Publishing branches");
    progress.detail("Branch is ready");
    let wait = progress.start_activity("GitHub retry", Some(Duration::from_secs(2)));
    progress.end_activity(wait);
    progress.finish(true);
    assert!(!progress.selection(&[], true, 20));
    assert!(!progress.results(
        &[],
        &Repository::default(),
        true,
        hyperlinks::Mode::Auto,
        &[],
        false
    ));
    drop(progress);
    assert_eq!(*terminal.stdout.lock().unwrap(), "Branch is ready\n");
    assert_eq!(
        *terminal.stderr.lock().unwrap(),
        "Live progress unavailable; using plain output: fixture thread capacity exhausted\nGitHub retry; waiting 2s\n"
    );
}

#[test]
fn completed_stages_fit_small_terminal_widths_and_preserve_status_when_space_allows() {
    for columns in 0..=80 {
        for success in [false, true] {
            let row = stage_result(
                "日本語 publication",
                Duration::from_millis(1250),
                success,
                columns,
                false,
            );
            assert!(row.contains("publication"));
            if columns >= 16 {
                assert!(row.lines().all(|line| line.width() <= columns));
            }
            assert!(!row.contains('\x1b'));
            if columns >= 40 {
                assert!(row.contains(if success { '✓' } else { '✗' }));
                assert!(row.contains("日本語 publication") && row.ends_with("1.2s"));
            }
        }
    }
}

#[test]
fn rendering_is_bounded_for_narrow_terminals_and_expired_timers() {
    let activity = Activity {
        label: "Updating PR descriptions".into(),
        detail: String::new(),
        started: Instant::now(),
        wait: None,
    };
    for width in [0, 1, 2, 15, 80, 200] {
        let line = render(&activity, None, None, 0, width);
        assert!(line.contains("descriptions"));
        if width >= 40 {
            assert!(line.lines().all(|line| line.width() < width));
        }
    }
    assert!(render(&activity, None, None, 9, 200).contains("elapsed"));
    let expired = Activity {
        wait: Some(Duration::ZERO),
        ..activity
    };
    let quota = ("GraphQL 0/5000".into(), Instant::now(), Duration::ZERO);
    let line = render(&expired, None, Some(&quota), 10, 200);
    assert!(line.contains("0s remaining"));
    assert!(line.contains("reset 0s"));
    let narrow = render(&expired, None, Some(&quota), 0, 80);
    assert!(narrow.contains("GraphQL 0/5000") && narrow.contains("reset 0s"));
    let compact = render(&expired, None, Some(&quota), 0, 30);
    assert!(compact.contains("remaining"));
    assert!(compact.contains("GraphQL") && compact.contains("0/5000"));
    assert!(compact.lines().all(|line| line.width() < 30));
    assert_eq!(duration(Duration::from_millis(1)), "1s");
    assert_eq!(duration(Duration::from_secs(120)), "2m 00s");
    let long = Activity {
        label: "x".repeat(120),
        wait: Some(Duration::from_secs(65)),
        ..expired
    };
    assert!(render(&long, None, None, 0, 80).contains("remaining"));
}

#[test]
fn terminal_fixture() {
    if std::env::var_os("GIT_PR_TEST_PROGRESS").is_none() {
        return;
    }
    let terminal = ProgressTerminal::new(Arc::new(crate::commands::Console::default()));
    let interrupt = std::env::var("GIT_PR_TEST_PROGRESS").unwrap() == "interrupt";
    if interrupt {
        use crate::commands::CommandRunner as _;
        let directory = tempfile::tempdir().unwrap();
        crate::commands::ProcessRunner::default()
            .execute("/usr/bin/true", &[], directory.path(), &BTreeMap::default())
            .unwrap();
    }
    let commits = [Commit {
        hash: "12345678".into(),
        title: "Unicode café 日本語".into(),
        pr_number: 42,
        metadata_updated: true,
        ..Commit::default()
    }];
    terminal.stage("Reading commits");
    if !terminal.selection(&commits.iter().collect::<Vec<_>>(), false, 20) {
        terminal.out("Plain selection");
    }
    terminal.stage("Updating descriptions");
    terminal.progress("Descriptions", 1, 1);
    terminal.detail("transient detail");
    let active = terminal.start_activity("Reading PRs", None);
    terminal.quota("GraphQL", 4990, 5000, Duration::from_secs(90));
    let wait = terminal.start_activity(
        "GitHub secondary backoff; retry 1/3",
        Some(Duration::from_secs(2)),
    );
    std::thread::sleep(Duration::from_millis(if interrupt { 30_000 } else { 250 }));
    terminal.end_activity(wait);
    let confirmed = terminal.confirm("Continue?");
    terminal.out(if confirmed { "accepted" } else { "declined" });
    terminal.end_activity(active);
    terminal.finish(true);
    if !terminal.results(
        &commits.iter().collect::<Vec<_>>(),
        &Repository {
            host: "github.com".into(),
            name: "org/repo".into(),
            ..Repository::default()
        },
        false,
        hyperlinks::Mode::parse(
            &std::env::var("GIT_PR_TEST_HYPERLINKS").unwrap_or_else(|_| "auto".into()),
        )
        .unwrap(),
        &[],
        false,
    ) {
        terminal.out("Plain results");
    }
    terminal.out("Final output");
}

#[test]
fn real_terminal_and_redirected_output_preserve_prompts_and_results() {
    let script = r"
import fcntl, os, pty, re, select, signal, struct, subprocess, sys, termios, time
env = dict(os.environ, GIT_PR_TEST_PROGRESS='1', TERM='xterm')
env.pop('CI', None)
env.pop('NO_COLOR', None)
for key in ['TERM_PROGRAM', 'TMUX', 'STY', 'SSH_CONNECTION', 'SSH_TTY', 'VTE_VERSION', 'WT_SESSION', 'KONSOLE_VERSION', 'FORCE_HYPERLINK', 'GIT_PR_TEST_HYPERLINKS']:
    env.pop(key, None)
command = [sys.argv[1], '--exact', 'cli::progress::tests::terminal_fixture', '--nocapture']
def capture(interrupt=False, settings=None, pipe_stdout=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    child_env = dict(env, GIT_PR_TEST_PROGRESS='interrupt' if interrupt else '1')
    child_env.update(settings or {})
    process = subprocess.Popen(command, stdin=slave, stdout=subprocess.PIPE if pipe_stdout else slave, stderr=slave, env=child_env)
    os.close(slave)
    output = b''
    answered = False
    deadline = time.monotonic() + 10
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                output += data
                if interrupt and b'remaining' in output and not answered:
                    process.send_signal(signal.SIGINT)
                    answered = True
                elif not interrupt and b'Continue?' in output and not answered:
                    os.write(master, b'y\n')
                    answered = True
            elif process.poll() is not None:
                break
        assert process.wait(timeout=1) == (130 if interrupt else 0), output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
    if pipe_stdout:
        stdout = process.stdout.read()
        assert b'Plain results' in stdout and b'Plain selection' in stdout, stdout
        assert b'Pull requests' not in stdout and b'\x1b' not in stdout, stdout
    return output
output = capture()
assert b'\r\x1b[2K' in output, output
assert b'remaining' in output and b'accepted' in output and b'Final output' in output, output
assert b'Pull requests' in output and b'1 updated' in output and b'\x1b[33m~\x1b[0m' in output, output
assert b'\x1b]8;;https://github.com/org/repo/pull/42\x1b\\\x1b[36m#42\x1b[0m\x1b]8;;\x1b\\' in output, output
assert 'org/repo · tip last'.encode() in output and '1 commit · tip last'.encode() in output, output
assert b'Plain results' not in output and b'Plain selection' not in output, output
assert 'café 日本語'.encode() in output, output
for settings in [{'NO_COLOR': '1'}, {'TERM': 'dumb', 'GIT_PR_TEST_HYPERLINKS': 'always'}, {'CI': 'true', 'GIT_PR_TEST_HYPERLINKS': 'always'}]:
    plain = capture(settings=settings)
    if 'NO_COLOR' in settings:
        assert not re.search(rb'\x1b\[[0-9;:]*m', plain), plain
        assert b'Pull requests' in plain and b'\x1b]8;;https://github.com/org/repo/pull/42' in plain, plain
    else:
        assert b'\x1b' not in plain, plain
        assert b'Plain results' in plain, plain
assert b'\r\x1b[2K' in capture(pipe_stdout=True)
for program in ['vscode', 'zed', 'iTerm.app', 'WezTerm', 'Windows_Terminal', 'unfamiliar', os.fsdecode(b'\xff')]:
    linked = capture(settings={'TERM_PROGRAM': program, 'NO_COLOR': '1', 'SSH_CONNECTION': 'example', 'SSH_TTY': '/dev/pts/1', 'TMUX': '/tmp/example', 'STY': 'session'})
    assert b'  ~  \x1b]8;;https://github.com/org/repo/pull/42\x1b\\#42\x1b]8;;\x1b\\' in linked, linked
    assert not re.search(rb'\x1b\[[0-9;:]*m', linked), linked
for settings in [
    {'TERM_PROGRAM': 'zed', 'GIT_PR_TEST_HYPERLINKS': 'never'},
    {'TERM_PROGRAM': 'unknown', 'GIT_PR_TEST_HYPERLINKS': 'never', 'FORCE_HYPERLINK': '1'},
]:
    fallback = capture(settings=settings)
    assert b'\x1b]8;' not in fallback and b'https://github.com/org/repo/pull/42' in fallback, fallback
forced = capture(settings={'TERM_PROGRAM': 'unknown', 'GIT_PR_TEST_HYPERLINKS': 'always'})
assert b'\x1b]8;' in forced, forced
capture(settings={'TERM_PROGRAM': 'zed', 'GIT_PR_TEST_HYPERLINKS': 'always'}, pipe_stdout=True)

interrupted = capture(True)
assert b'\r\x1b[2KInterrupted:' in interrupted, interrupted
assert b'Final output' not in interrupted, interrupted
piped = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, env=env, timeout=10, check=True)
assert b'waiting 2s' in piped.stderr, piped.stderr
assert b'\x1b' not in piped.stderr, piped.stderr
assert b'declined' in piped.stdout and b'Final output' in piped.stdout, piped.stdout
";
    let result = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

const LONG_REQUEST: &str =
    "example-org/app/commits/0123456789abcdef0123456789abcdef01234567/pulls?per_page=100&page=1";

#[test]
fn request_details_yield_space_to_stage_counters_and_quota() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal, false);
    progress.stage("Preparing commits");
    progress.quota("REST", 14093, 15000, Duration::from_secs(274));
    let request = progress.start_activity_with_detail("GitHub GET", LONG_REQUEST);
    for columns in [80, 124, 160] {
        let row = render_state(&lock(&progress.shared.state), 0, columns);
        assert_eq!(row.lines().count(), 1, "{row}");
        assert!(
            row.starts_with("  ⠋ Preparing commits · GitHub GET "),
            "{row}"
        );
        assert!(row.width() < columns, "{row}");
        assert!(row.contains("REST 14093/15000 · reset 4m34s"), "{row}");
        if columns < 160 {
            assert!(row.contains('…') && !row.contains(LONG_REQUEST), "{row}");
        } else {
            assert!(row.contains(LONG_REQUEST), "{row}");
        }
    }
    progress.progress("Commits #101", 1, 3);
    for columns in [40, 80, 124] {
        let row = render_state(&lock(&progress.shared.state), 0, columns);
        assert!(row.contains("1/3 Commits #101"), "{row}");
        assert!(row.contains("REST 14093/15000"), "{row}");
        assert!(row.lines().count() <= 2, "{row}");
        assert!(row.lines().all(|line| line.width() < columns), "{row}");
    }
    let wait = progress.start_activity("GitHub retry 1/3", Some(Duration::from_secs(60)));
    let row = render_state(&lock(&progress.shared.state), 0, 124);
    assert!(row.contains("retry 1/3") && row.contains("remaining"));
    assert!(!row.contains("example-org"));
    progress.end_activity(wait);
    progress.end_activity(request);
    assert!(!render_state(&lock(&progress.shared.state), 0, 124).contains("example-org"));
}

#[test]
fn long_request_fixture() {
    if std::env::var_os("GIT_PR_TEST_LONG_REQUEST").is_none() {
        return;
    }
    let terminal = ProgressTerminal::new(Arc::new(crate::commands::Console::default()));
    terminal.stage_with_completion("Preparing commits", "Commits prepared");
    terminal.quota("REST", 14093, 15000, Duration::from_secs(274));
    let request = terminal.start_activity_with_detail("GitHub GET", LONG_REQUEST);
    let mut permission = String::new();
    std::io::stdin().read_line(&mut permission).unwrap();
    assert_eq!(permission, "done\n");
    terminal.end_activity(request);
    terminal.finish(true);
}

#[test]
fn long_requests_stay_coloured_and_aligned_until_cleared_in_real_terminals() {
    let script = r"
import fcntl, os, pty, re, select, struct, subprocess, sys, termios, time
for columns in [40, 80, 124]:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, columns, 0, 0))
    env = dict(os.environ, GIT_PR_TEST_LONG_REQUEST='1', TERM='xterm')
    env.pop('CI', None)
    env.pop('NO_COLOR', None)
    p = subprocess.Popen([sys.argv[1], '--exact', 'cli::progress::tests::long_request_fixture', '--nocapture'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=slave, env=env)
    os.close(slave)
    output = b''
    try:
        deadline = time.monotonic() + 8
        frames = []
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                output += os.read(master, 65536)
            frames = [frame.decode() for frame in re.findall(rb'\x1b\[36m(.*?)\x1b\[0m', output, re.S) if b'REST 14093/15000' in frame]
            if frames: break
        assert frames, output
        for frame in frames:
            lines = frame.splitlines()
            lines = [line for line in lines if line]
            assert lines[0].startswith('  ') and 'Preparing commits' in lines[0], frame
            assert all(len(line) < columns for line in lines), frame
            assert len(lines) <= (2 if columns == 40 else 1), frame
            if columns >= 80: assert '…' in frame, frame
        assert b'page=1' not in output, output
        p.stdin.write(b'done\n')
        p.stdin.flush()
        deadline = time.monotonic() + 5
        while b'Commits prepared' not in output and time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try: data = os.read(master, 65536)
                except OSError: break
                if not data: break
                output += data
        p.communicate(timeout=5)
        assert p.returncode == 0
        assert b'\r\x1b[2K  \x1b[32m' in output, output
        assert b'Commits prepared' in output, output
    finally:
        if p.poll() is None: p.kill(); p.wait()
        os.close(master)
";
    let result = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn selection_preview_respects_limits_and_orientation() {
    for (count, limit, positions) in [
        (0, 20, vec![]),
        (10, 20, (0..10).collect()),
        (20, 20, (0..20).collect()),
        (21, 20, (0..10).chain(11..21).collect()),
        (24, 20, (0..10).chain(14..24).collect()),
        (12, 5, vec![0, 1, 2, 10, 11]),
        (12, 1, vec![0]),
        (12, 0, (0..12).collect()),
        (12, usize::MAX, (0..12).collect()),
    ] {
        for stack_up in [false, true] {
            let terminal = Arc::new(MemoryTerminal::default());
            let progress = ProgressTerminal::with_options(terminal.clone(), true, true, false);
            let commits: Vec<_> = (0..count)
                .map(|n| Commit {
                    hash: format!("{n:08x}"),
                    title: format!("Feature {n}"),
                    ..Commit::default()
                })
                .collect();
            let mut selected: Vec<_> = commits.iter().collect();
            if stack_up {
                selected.reverse();
            }
            assert!(progress.selection(&selected, stack_up, limit));
            drop(progress);
            let stdout = terminal.stdout.lock().unwrap();
            let shown: Vec<usize> = stdout
                .lines()
                .filter_map(|line| line.split_once("  Feature "))
                .map(|(_, n)| n.parse().unwrap())
                .collect();
            let expected: Vec<_> = positions
                .iter()
                .map(|&n| if stack_up { count - 1 - n } else { n })
                .collect();
            assert_eq!(shown, expected, "count={count}, limit={limit}");
            let hidden = count - positions.len();
            if hidden == 0 {
                assert!(!stdout.contains("more commit"));
            } else {
                let suffix = if hidden == 1 { "" } else { "s" };
                assert!(stdout.contains(&format!("  … {hidden} more commit{suffix}\n")));
                assert_eq!(stdout.matches("more commit").count(), 1);
            }
        }
    }
}

#[test]
fn rich_console_compacts_selection_and_reports_every_result_with_accurate_status() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_options(terminal.clone(), true, true, true);
    let mut commits: Vec<_> = (1..=12)
        .map(|n| Commit {
            hash: format!("{n:040x}"),
            title: format!("Feature {n}: café 日本語 👩‍💻"),
            pr_number: n,
            ..Commit::default()
        })
        .collect();
    commits[0].newly_created = true;
    commits[0].metadata_updated = true;
    commits[1].base_updated = true;
    commits[2].metadata_updated = true;
    progress.stage("Reading commits");
    assert!(progress.selection(&commits.iter().rev().collect::<Vec<_>>(), true, 8));
    progress.stage("Publishing branches");
    progress.detail("push -f origin example");
    progress.progress("Branches", 12, 12);
    progress.finish(true);
    let repo = Repository {
        host: "github.example.com".into(),
        name: "org/repo".into(),
        ..Repository::default()
    };
    assert!(progress.results(
        &commits.iter().rev().collect::<Vec<_>>(),
        &repo,
        true,
        hyperlinks::Mode::Never,
        &[],
        false
    ));
    drop(progress);
    let stdout = terminal.stdout.lock().unwrap().clone();
    let stderr = terminal.stderr.lock().unwrap().clone();
    assert!(stdout.contains("4 more commits"));
    assert!(stdout.contains("12 commits · tip first"));
    assert!(stdout.find("Feature 12:").unwrap() < stdout.find("Feature 1:").unwrap());
    assert!(!stdout.contains("push -f"));
    assert!(stdout.contains("1 created · 2 updated · 9 unchanged"));
    assert!(stdout.contains("github.example.com/org/repo · tip first"));
    assert!(stdout.contains("café 日本語 👩‍💻"));
    for number in 1..=12 {
        assert_eq!(
            stdout
                .matches(&format!(
                    "https://github.example.com/org/repo/pull/{number}\x1b[0m\n"
                ))
                .count(),
            1
        );
    }
    let plain = clean(&stdout, false);
    assert!(plain.contains("  +   #1  ?  Feature 1:"));
    assert!(plain.contains("  ~   #2  ?  Feature 2:"));
    assert!(plain.contains("  ~   #3  ?  Feature 3:"));
    assert!(plain.contains("  =  #12  ?  Feature 12:"));
    assert!(plain.find("#12 ").unwrap() < plain.find("#1 ").unwrap());
    assert!(stderr.contains("✓") && stderr.contains("Reading commits"));
    assert!(stderr.contains("Publishing branches") && stderr.contains("0."));
    assert!(stdout.contains("\x1b[1m"));
}

#[test]
fn plain_console_preserves_output_templates_and_removes_generated_colour() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), false);
    progress.stage("Preparing preview");
    progress.detail("ordinary detail");
    progress.out("literal\rtemplate\tvalue");
    progress.err("\x1b[33mWarning\x1b[0m\n");
    assert!(!progress.selection(&[], true, 20));
    assert!(!progress.results(
        &[],
        &Repository::default(),
        false,
        hyperlinks::Mode::Auto,
        &[],
        false
    ));
    progress.finish(false);
    assert_eq!(
        *terminal.stdout.lock().unwrap(),
        "ordinary detail\nliteral\rtemplate\tvalue\n"
    );
    assert_eq!(*terminal.stderr.lock().unwrap(), "Warning\n");
}

#[test]
fn failed_stages_are_durable_and_empty_results_are_explicit() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), true);
    progress.stage("Updating descriptions");
    progress.finish(false);
    progress.finish(false);
    assert!(progress.selection(&[], true, 20));
    assert!(progress.results(
        &[],
        &Repository::default(),
        false,
        hyperlinks::Mode::Auto,
        &[],
        false
    ));
    drop(progress);
    let stderr = terminal.stderr.lock().unwrap().clone();
    assert_eq!(stderr.matches("✗ Updating descriptions").count(), 1);
    assert!(!stderr.contains('✓'));
    let stdout = terminal.stdout.lock().unwrap().clone();
    assert!(stdout.contains("Pull requests\n  tip last"));
    assert!(stdout.contains("Nothing selected for publication"));
}

#[test]
fn rendering_prioritises_waits_and_preserves_stage_progress_between_operations() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal, true);
    progress.stage("Updating descriptions");
    progress.progress("Descriptions", 8, 20);
    progress.detail("Checking content");
    let request = progress.start_activity("GitHub GraphQL", None);
    let wait = progress.start_activity("GitHub retry 1/3", Some(Duration::from_secs(60)));
    let concurrent = progress.start_activity("Another request", None);
    let state = lock(&progress.shared.state);
    let line = render_state(&state, 0, 180);
    assert!(line.contains("GitHub retry 1/3") && line.contains("remaining"));
    assert!(line.contains("8/20 Descriptions"));
    assert!(!line.contains("Another request"));
    drop(state);
    progress.end_activity(wait);
    progress.end_activity(concurrent);
    let state = lock(&progress.shared.state);
    let line = render_state(&state, 1, 180);
    assert!(line.contains("8/20 Descriptions · GitHub GraphQL"));
    drop(state);
    progress.end_activity(request);
    assert!(render_state(&lock(&progress.shared.state), 2, 180).contains("Checking content"));
    progress.finish(true);
    assert_eq!(render_state(&lock(&progress.shared.state), 3, 180), "");
}

#[test]
fn current_items_and_counts_survive_long_stage_labels_and_narrow_displays() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal, false);
    progress.stage("Updating pull request descriptions across the selected stack");
    progress.progress("Descriptions #101, #102", 1, 3);
    progress.quota("GraphQL", 4990, 5000, Duration::from_secs(90));
    let request = progress.start_activity("GitHub GraphQL request with ancillary context", None);
    let state = lock(&progress.shared.state);
    for columns in [40, 64, 80, 120] {
        let line = render_state(&state, 0, columns);
        assert!(line.contains("1/3 Descriptions #101, #102"), "{line}");
        assert!(line.lines().all(|row| row.width() < columns));
        assert!(line.contains("4990/5000"));
    }
    let compact = render_state(&state, 0, 40);
    assert!(compact.contains("GraphQL") && compact.contains("4990/5000"));
    drop(state);
    let wait = progress.start_activity("GitHub retry 1/3", Some(Duration::from_secs(60)));
    let line = render_state(&lock(&progress.shared.state), 0, 80);
    assert!(line.contains("1/3 Descriptions #101, #102"));
    assert!(line.contains("GitHub retry 1/3") && line.contains("remaining"));
    let compact = render_state(&lock(&progress.shared.state), 0, 40);
    assert!(compact.contains("1/3 Descriptions #101"));
    assert!(compact.contains("remaining"));
    progress.end_activity(wait);
    progress.end_activity(request);
    progress.progress("Descriptions #103", 9, 3);
    let line = render_state(&lock(&progress.shared.state), 0, 80);
    assert!(line.contains("3/3 Descriptions #103"));
    assert!(!line.contains("#101"));
    progress.stage("Checking branches");
    let line = render_state(&lock(&progress.shared.state), 0, 80);
    assert!(line.contains("Checking branches"));
    assert!(!line.contains("Descriptions") && !line.contains("3/3"));
    progress.progress("No requests", 0, 0);
    assert!(render_state(&lock(&progress.shared.state), 0, 80).contains("0/0 No requests"));
    progress.finish(true);
    assert!(lock(&progress.shared.state).progress.is_none());
    assert_eq!(render_state(&lock(&progress.shared.state), 0, 80), "");
}

#[test]
fn stage_summaries_retain_final_counts_and_partial_failure_without_stale_counts() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), true);
    progress.stage("Updating descriptions");
    progress.progress("Descriptions", 3, 3);
    progress.stage("Checking branches");
    progress.finish(true);
    progress.stage("Updating descriptions");
    progress.progress("Descriptions #102", 1, 3);
    progress.finish(false);
    progress.stage("Reconciling pull requests");
    progress.progress("Created #103", 3, 3);
    progress.progress("Bases #104, #105", 0, 2);
    progress.progress("Bases", 2, 2);
    progress.finish(true);
    drop(progress);
    let stderr = terminal.stderr.lock().unwrap().clone();
    assert!(stderr.contains("✓ 3/3 Descriptions"), "{stderr}");
    assert!(
        stderr.contains("Descriptions · Updating descriptions"),
        "{stderr}"
    );
    assert!(stderr.contains("✓ Checking branches"), "{stderr}");
    assert!(!stderr.contains("3/3 Checking branches"));
    assert!(stderr.contains("✗ 1/3 Descriptions #102"), "{stderr}");
    assert_eq!(stderr.matches("✓ 3/3 Descriptions").count(), 1);
    assert!(stderr.contains("✓ 2/2 Bases"), "{stderr}");
    assert!(!stderr.contains("2/2 Reconciling"), "{stderr}");
}

#[test]
fn progress_clipping_preserves_unicode_and_live_timers_at_every_width() {
    let activity = Activity {
        label: "Updating descriptions with a long explanation".into(),
        detail: String::new(),
        started: Instant::now().checked_sub(Duration::from_secs(2)).unwrap(),
        wait: None,
    };
    for label in [
        "Descriptions #101",
        "日本語 #101",
        "👩‍💻 #101",
        "a\u{301} #101",
    ] {
        let progress = Progress {
            label: label.into(),
            done: 1,
            total: 3,
        };
        for columns in 0..=120 {
            let row = render(&activity, Some(&progress), None, 0, columns);
            assert!(row.contains("1/3"));
            if columns >= 80 {
                assert!(row.lines().all(|line| line.width() < columns), "{row}");
            }
            assert!(!row.contains('\x1b'));
            if columns >= 40 {
                assert!(row.contains(&format!("1/3 {label}")), "{row}");
                assert!(row.contains(&format!("1/3 {label}")), "{row}");
            }
        }
    }
}

#[test]
fn incremental_terminal_fixture() {
    if std::env::var_os("GIT_PR_TEST_INCREMENTAL_PROGRESS").is_none() {
        return;
    }
    let terminal = ProgressTerminal::new(Arc::new(crate::commands::Console::default()));
    terminal.stage("Updating pull request descriptions across the selected stack");
    let activity = terminal.start_activity("GitHub GraphQL", None);
    terminal.quota("GraphQL", 4990, 5000, Duration::from_secs(90));
    for (done, label) in [
        (0, "Descriptions #101, #102"),
        (1, "Descriptions #102"),
        (2, "Descriptions #103"),
        (3, "Descriptions complete"),
    ] {
        terminal.progress(label, done, 3);
        // The parent must observe this frame before it permits work to continue.
        let mut permission = String::new();
        std::io::stdin().read_line(&mut permission).unwrap();
        assert_eq!(permission, "next\n");
    }
    terminal.end_activity(activity);
    terminal.finish(true);
    terminal.out("All descriptions updated");
}

#[test]
fn real_terminal_displays_each_current_request_before_work_is_released() {
    let script = r"
import fcntl, os, pty, select, struct, subprocess, sys, termios, time
env = dict(os.environ, GIT_PR_TEST_INCREMENTAL_PROGRESS='1', TERM='xterm', NO_COLOR='1')
env.pop('CI', None)
command = [sys.argv[1], '--exact', 'cli::progress::tests::incremental_terminal_fixture', '--nocapture']
for columns in [40, 80]:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, columns, 0, 0))
    process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, env=env)
    os.close(slave)
    output = b''
    pending = b''
    expected = [b'0/3 Descriptions #101, #102', b'1/3 Descriptions #102', b'2/3 Descriptions #103', b'3/3 Descriptions complete']
    observed = 0
    deadline = time.monotonic() + 10
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                output += data
                pending += data
                if observed < len(expected) and expected[observed] in pending:
                    # Keep the first request blocked long enough to observe its live timer.
                    if observed != 0 or b'4990/5000' in pending:
                        assert b'All descriptions updated' not in output, output
                        assert process.poll() is None, output
                        os.write(master, b'next\n')
                        observed += 1
                        pending = b''
            elif process.poll() is not None:
                break
        try:
            assert process.wait(timeout=1) == 0, output
        except subprocess.TimeoutExpired:
            raise AssertionError((observed, output))
        assert observed == len(expected), (observed, output)
        assert b'All descriptions updated' in output, output
        if columns >= 80: assert b'\r\x1b[2K' in output, output
        else: assert b'GraphQL 4990/5000' in output and b'reset' in output, output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
";
    let result = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn clipping_preserves_graphemes_and_terminal_cell_width() {
    assert_eq!(clip("café", 4), "café");
    assert_eq!(clip("a\u{301}bc", 2), "a\u{301}…");
    assert_eq!(clip("👩‍💻abc", 3), "👩‍💻…");
    assert_eq!(clip("日本語", 3), "日…");
    for label in ["日本語", "👩‍💻界café", "a\u{301}bc"] {
        let activity = Activity {
            label: label.into(),
            detail: String::new(),
            started: Instant::now(),
            wait: None,
        };
        for columns in 0..=100 {
            assert!(clip(label, columns).width() <= columns);
            let frame = render(&activity, None, None, 0, columns);
            assert!(frame.contains(label));
            if columns >= 15 {
                assert!(frame.lines().all(|line| line.width() < columns));
            }
        }
    }
    assert_eq!(
        single_line("title\nnext\tword\r\x1b[2K"),
        "title next word\\r\\u{1b}[2K"
    );
    assert_eq!(clean("\x1b[31mred\x1b[0m", false), "red");
    assert_eq!(clean("\x1b[31mred\x1b[0m", true), "\x1b[31mred\x1b[0m");
    assert_eq!(clean("\x1b]0;title\x07", true), "\\u{1b}]0;title\\u{7}");
    assert_eq!(clean("\x1b[", false), "\\u{1b}[");
}

#[test]
fn rich_sections_and_single_line_titles_preserve_spacing_and_unicode() {
    for columns in [0, 1, 4, 5, 12, 40, 80, 200] {
        let text = heading("Selected stack", columns, false);
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>().join(" "),
            "Selected stack"
        );
        if columns >= 12 {
            assert!(text.lines().all(|line| line.width() <= columns));
        }
    }
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_options(terminal.clone(), true, true, false);
    progress.section("Repository");
    progress.stage("Reading commits");
    progress.finish(true);
    let commits = [Commit { hash: "12345678".into(), title: "A complete title with enough words to wrap across a narrow display without losing any of its content".into(), pr_number: 42, ..Commit::default() }, Commit { pr_number: 43, title: "Next PR".into(), ..Commit::default() }];
    let selected: Vec<_> = commits.iter().collect();
    assert!(progress.selection(&selected, true, 20));
    progress.section("Submission");
    assert!(progress.results(
        &selected,
        &Repository {
            host: "github.com".into(),
            name: "org/repo".into(),
            ..Repository::default()
        },
        true,
        hyperlinks::Mode::Never,
        &[],
        false,
    ));
    drop(progress);
    let stdout = terminal.stdout.lock().unwrap();
    assert!(stdout.contains("  Repository\n"));
    assert!(stdout.contains("\n\n  Submission\n"));
    assert!(stdout.contains("pull/42\n  =  #43  ?  Next PR\n             https://"));
    let results = stdout.split("Pull requests").last().unwrap();
    assert!(results.contains('…') && !stdout.contains('\x1b'));
    assert!(
        results
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("A complete title with enough words")
    );
    drop(stdout);
}

#[test]
fn completion_labels_are_explicit_and_used_only_after_success() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_options(terminal.clone(), true, true, false);
    progress.stage_with_completion("Updating PR labels", "Reconciled labels");
    progress.progress("Labels #101", 10, 10);
    progress.finish(true);
    progress.stage_with_completion("Updating PR descriptions", "Reconciled descriptions");
    progress.progress("Descriptions #102", 1, 3);
    progress.finish(false);
    drop(progress);
    let text = terminal.stderr.lock().unwrap();
    assert!(text.contains("✓ Reconciled labels · 10/10"));
    assert!(text.contains("✗ 1/3 Descriptions #102 · Updating PR descriptions"));
    assert!(!text.contains("Reconciled descriptions"));
    drop(text);
}

#[test]
fn two_row_frames_clear_completely_before_durable_output() {
    let terminal = Arc::new(MemoryTerminal::default());
    let progress = ProgressTerminal::with_animation(terminal.clone(), false);
    lock(&progress.shared.state).painted = 2;
    progress.err("important diagnostic: operation unconfirmed; verify #123\n");
    assert_eq!(
        *terminal.stderr.lock().unwrap(),
        "\r\x1b[2K\x1b[1A\r\x1b[2Kimportant diagnostic: operation unconfirmed; verify #123\n"
    );
    assert_eq!(lock(&progress.shared.state).painted, 0);
}

#[test]
fn independent_width_fixture() {
    if std::env::var_os("GIT_PR_TEST_INDEPENDENT_WIDTH").is_none() {
        return;
    }
    let terminal = ProgressTerminal::new(Arc::new(crate::commands::Console::default()));
    let repo = Repository {
        host: "github.com".into(),
        name: "org/repo".into(),
        ..Repository::default()
    };
    terminal.stage("Updating descriptions");
    for step in 0..3 {
        let mut permission = String::new();
        std::io::stdin().read_line(&mut permission).unwrap();
        assert_eq!(permission, "next\n");
        terminal.quota("GraphQL", 0, 5000, Duration::from_secs(90));
        terminal.progress("Descriptions #101", step, 3);
        let deadline = Instant::now() + Duration::from_secs(3);
        while lock(&terminal.shared.state).painted == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // Let the next paced frame reflect the updated progress and actual window size.
        std::thread::sleep(Duration::from_millis(150));
        let rows = lock(&terminal.shared.state).painted;
        terminal.out(&format!("BEGIN{step}"));
        let commit = Commit { pr_number: 42, title: "A long title with enough words to require ellipsis in all of these terminal panes and no inserted title continuation".into(), ..Commit::default() };
        assert!(terminal.results(
            &[&commit],
            &repo,
            true,
            hyperlinks::Mode::Always,
            &[],
            false
        ));
        terminal.out(&format!("STEP{step} rows={rows}"));
    }
    terminal.finish(true);
}

#[test]
fn actual_stream_widths_and_resizing_preserve_single_rows_and_complete_quota() {
    let script = r"
import fcntl, os, pty, re, select, struct, subprocess, sys, termios, time
out_master, out_slave = pty.openpty()
err_master, err_slave = pty.openpty()
def resize(fd, columns):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, columns, 0, 0))
resize(out_slave, 40)
resize(err_slave, 100)
env = dict(os.environ, GIT_PR_TEST_INDEPENDENT_WIDTH='1', TERM='xterm', NO_COLOR='1')
env.pop('CI', None)
p = subprocess.Popen([sys.argv[1], '--exact', 'cli::progress::tests::independent_width_fixture', '--nocapture'], stdin=subprocess.PIPE, stdout=out_slave, stderr=err_slave, env=env)
os.close(out_slave)
os.close(err_slave)
try:
    for step, (out_width, err_width, expected_rows) in enumerate([(40,100,1), (100,40,2), (60,80,1)]):
        resize(out_master, out_width)
        resize(err_master, err_width)
        p.stdin.write(b'next\n')
        p.stdin.flush()
        out, err = b'', b''
        deadline = time.monotonic() + 8
        marker = ('STEP%d rows=%d' % (step, expected_rows)).encode()
        while marker not in out and time.monotonic() < deadline:
            for fd in select.select([out_master, err_master], [], [], .1)[0]:
                data = os.read(fd, 65536)
                if fd == out_master: out += data
                else: err += data
        assert marker in out, (step, out, err)
        assert b'GraphQL' in err and b'0/5000' in err and b'reset' in err, err
        if step == 1:
            assert b'\x1b[1A' in err, err
            assert re.search(rb'\r\n    GraphQL 0/5000 \xc2\xb7 reset [^\r\n]+', err), err
        body = out.split(('BEGIN%d' % step).encode(),1)[1].split(b'STEP',1)[0]
        body = re.sub(rb'\x1b\]8;;[^\x1b]*\x1b\\', b'', body).decode()
        lines = body.splitlines()
        rows = [line for line in lines if '#42' in line]
        assert len(rows) == 1 and rows[0].endswith('…'), body
        assert all(len(line) < out_width for line in lines), (out_width, body)
    assert p.wait(timeout=5) == 0
finally:
    if p.poll() is None: p.kill(); p.wait()
    os.close(out_master)
    os.close(err_master)
";
    let result = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
