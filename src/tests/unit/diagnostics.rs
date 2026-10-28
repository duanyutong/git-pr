use super::support::World;
use crate::{
    cli::Flags,
    commands::{CommandOutput, CommandRunner, StructuredOutput},
    diagnostics::{Diagnostics, TimedRunner},
};
use std::{collections::BTreeMap, ffi::OsString, path::Path, sync::Arc, time::Duration};

#[test]
fn verbosity_is_bounded_and_explicit_false_resets_it() {
    for (args, expected) in [
        (vec![], 0),
        (vec!["-v"], 1),
        (vec!["--verbose"], 1),
        (vec!["--debug"], 2),
        (vec!["--verbose", "-v"], 2),
        (vec!["--debug", "--verbose"], 2),
        (vec!["--debug", "--verbose=false"], 0),
        (vec!["-vv", "--debug=false"], 0),
        (vec!["--debug=false", "--verbose"], 1),
        (vec!["HEAD", "--debug"], 0),
        (vec!["-vv"], 2),
        (vec!["-v", "-v"], 2),
        (vec!["-v", "-v", "-v"], 2),
        (vec!["-vv", "-v=false"], 0),
        (vec!["-vv=false", "-v"], 1),
        (vec!["-v", "--", "-v"], 1),
        (vec!["HEAD", "-vv"], 0),
    ] {
        assert_eq!(
            Flags::parse(args.into_iter().map(str::to_owned).collect())
                .unwrap()
                .verbosity(),
            expected
        );
    }
    assert!(Flags::parse(vec!["-vv=invalid".into()]).is_err());
}

fn effects(world: &World) -> Vec<String> {
    let mut trace: Vec<_> = world
        .trace()
        .into_iter()
        .map(|mut event| {
            // Mutation spacing compensates for real elapsed request time.
            if event["kind"] == "wait" {
                event["millis"] = 0.into();
            }
            event.to_string()
        })
        .collect();
    trace.sort();
    trace
}

#[test]
fn diagnostics_preserve_submission_effects_in_both_native_modes() {
    for mode in ["--no-stack", "--github-stack"] {
        let plain = World::new(2);
        plain.run(&[mode]).unwrap();
        let expected = effects(&plain);
        for verbosity in ["-v", "--verbose", "-vv", "--debug"] {
            let timed = World::new(2);
            timed.run(&[verbosity, mode]).unwrap();
            assert_eq!(effects(&timed), expected);
            let ordinary = plain.0.lock().unwrap();
            let mut measured = timed.0.lock().unwrap();
            assert_eq!(ordinary.prs, measured.prs);
            assert_eq!(ordinary.stdout, measured.stdout);
            assert!(measured.stderr.contains("start command: git rev-parse"));
            assert!(measured.stderr.contains("start command: git config"));
            assert!(
                measured
                    .stderr
                    .contains("start phase: PR descriptions and stack sections")
            );
            assert!(measured.stderr.contains("Timing summary: completed"));
            assert!(measured.stderr.contains("selected base"));
            assert_eq!(
                measured.stderr.contains("\"query\""),
                matches!(verbosity, "-vv" | "--debug")
            );
            assert!(!measured.stderr.contains("unit-token"));
            measured.trace.clear();
            measured.stderr.clear();
            drop(measured);
            drop(ordinary);
            timed.run(&[verbosity, mode]).unwrap();
            let measured = timed.0.lock().unwrap();
            assert!(
                measured
                    .stderr
                    .contains("title and description already current")
            );
            assert!(
                measured
                    .stderr
                    .contains("Checked 2 PR descriptions; no changes needed")
            );
            assert!(
                measured
                    .stderr
                    .contains("Checked 2 PR labels; no changes needed")
            );
            assert_eq!(
                measured.stderr.matches("start command: git push").count(),
                0
            );
            assert_eq!(measured.stderr.matches("start wait:").count(), 0);
            assert_eq!(
                measured
                    .stderr
                    .matches("start phase: PR descriptions and stack sections")
                    .count(),
                1
            );
        }
        assert!(!plain.0.lock().unwrap().stderr.contains("Timing summary"));
    }
}

#[test]
fn checkpoints_and_failed_discovery_receive_complete_timing_summaries() {
    for args in [
        vec!["-v", "--stop-after=validate"],
        vec!["-v", "--default-tags=ready"],
        vec!["-v", "--dry-run"],
    ] {
        let world = World::new(1);
        world.run(&args).unwrap();
        assert!(
            world
                .0
                .lock()
                .unwrap()
                .stderr
                .contains("Timing summary: completed")
        );
    }
    let world = World::new(1);
    world.run(&["-v", "--stop-after=invalid"]).unwrap_err();
    assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
    let stderr = world.0.lock().unwrap().stderr.clone();
    assert!(stderr.contains("start phase: configuration and repository discovery"));
    assert!(stderr.contains("; error)"));
    assert!(stderr.contains("Timing summary: failed"));
}

#[test]
fn disabled_diagnostics_do_not_format_or_emit_and_parallel_events_pair_by_id() {
    let silent = Diagnostics::default();
    silent
        .start("command", || panic!("disabled label formatted"))
        .finish(|| panic!("disabled status formatted"));
    silent.summary(true);
    assert_eq!(silent.phase("silent", || Ok(7)).unwrap(), 7);
    let world = World::new(0);
    let diagnostics = Diagnostics::new(world.clone());
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                let span = diagnostics.start("command", || "safe\nlabel\x1b".into());
                span.finish(|| "ok".into());
            });
        }
    });
    diagnostics.summary(true);
    let stderr = world.0.lock().unwrap().stderr.clone();
    for id in 1..=16 {
        assert_eq!(
            stderr
                .matches(&format!("#{id}] start command: safe?label?"))
                .count(),
            1
        );
        assert_eq!(
            stderr
                .matches(&format!("#{id}] end command: safe?label?"))
                .count(),
            1
        );
    }
    assert!(stderr.contains("command: 16 operations"));
}

#[derive(Debug)]
struct Runner;
impl CommandRunner for Runner {
    fn configure_timeouts(&self, read: Duration, command: Duration) {
        assert_eq!(read, Duration::from_secs(3));
        assert_eq!(command, Duration::from_secs(4));
    }
    fn execute(
        &self,
        _: &str,
        args: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
    ) -> anyhow::Result<CommandOutput> {
        if args.last().is_some_and(|arg| arg == "fail") {
            anyhow::bail!("private error payload");
        }
        Ok(CommandOutput {
            status: 7,
            text: "private output".into(),
        })
    }
    fn execute_data(
        &self,
        _: &str,
        _: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
        input: &[u8],
    ) -> anyhow::Result<StructuredOutput> {
        assert_eq!(input, b"private input");
        Ok(StructuredOutput {
            status: 0,
            stdout: b"private bytes".to_vec(),
            stderr: Vec::new(),
        })
    }
    fn edit_messages(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
        messages: &[(String, String)],
    ) -> anyhow::Result<CommandOutput> {
        assert_eq!(messages, [("oid".into(), "private message".into())]);
        self.execute(program, args, cwd, env)
    }
}

#[test]
fn runner_forwards_results_and_counts_bulk_editing_once_without_payloads() {
    let world = World::new(0);
    let runner = TimedRunner {
        inner: Arc::new(Runner),
        diagnostics: Diagnostics::new(world.clone()),
    };
    let cwd = Path::new("/unused");
    let env = BTreeMap::new();
    runner.configure_timeouts(Duration::from_secs(3), Duration::from_secs(4));
    let args = ["-c", "secret=private", "log", "--", "private ref"].map(str::to_owned);
    let output = runner.execute("git", &args, cwd, &env).unwrap();
    assert_eq!((output.status, output.text.as_str()), (7, "private output"));
    assert!(runner.execute("git", &["fail".into()], cwd, &env).is_err());
    assert_eq!(
        runner
            .execute_data(
                "jj",
                &["--ignore-working-copy".into(), "log".into()],
                cwd,
                &env,
                b"private input"
            )
            .unwrap()
            .stdout,
        b"private bytes"
    );
    runner
        .edit_messages(
            "git",
            &["reword".into()],
            cwd,
            &env,
            &[("oid".into(), "private message".into())],
        )
        .unwrap();
    runner.diagnostics.summary(true);
    let stderr = world.0.lock().unwrap().stderr.clone();
    assert_eq!(stderr.matches("start command:").count(), 4);
    assert!(stderr.contains("start command: git log"));
    assert!(stderr.contains("start command: jj log"));
    assert!(stderr.contains("exit 7"));
    assert!(stderr.contains("; error)"));
    assert!(!stderr.contains("private"));
}

#[test]
fn unfamiliar_executables_receive_a_generic_label_without_paths_or_arguments() {
    let world = World::new(0);
    let runner = TimedRunner {
        inner: Arc::new(Runner),
        diagnostics: Diagnostics::new(world.clone()),
    };
    let output = runner
        .execute(
            "/private/credential-helper",
            &["opaque-secret-argument".into()],
            Path::new("/unused"),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(output.status, 7);
    let stderr = world.0.lock().unwrap().stderr.clone();
    assert!(stderr.contains("start command: subprocess"));
    assert!(!stderr.contains("credential-helper") && !stderr.contains("opaque-secret-argument"));
}
