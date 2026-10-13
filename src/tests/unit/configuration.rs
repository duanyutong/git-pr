//! Framed configuration values and conversion fallback, without filesystem access.
use super::Values;
use crate::tests::unit::support::World;

#[test]
fn pattern_only_queries_return_matching_effective_settings() {
    let world = World::new(0);
    {
        let mut memory = world.0.lock().unwrap();
        memory
            .config
            .insert("branch.Topic.pushRemote".into(), "publish".into());
        memory
            .config
            .insert("branch.Other.remote".into(), "fetch-only".into());
    }
    let values =
        Values::read_patterns(&world.commands(), &[], None, &[r"branch\..*\.pushremote"]).unwrap();
    assert_eq!(values.get("branch.Topic.pushRemote"), Some("publish"));
    assert_eq!(values.get("branch.Other.remote"), None);
    assert_eq!(values.get("user.name"), None);
}

#[test]
fn absence_status_with_unexpected_stdout_is_not_a_successful_empty_query() {
    use crate::commands::{CommandOutput, CommandRunner, StructuredOutput};
    use std::{collections::BTreeMap, ffi::OsString, path::Path, sync::Arc};

    #[derive(Debug)]
    struct UnexpectedStdout;
    impl CommandRunner for UnexpectedStdout {
        fn execute(
            &self,
            _: &str,
            _: &[String],
            _: &Path,
            _: &BTreeMap<OsString, OsString>,
        ) -> anyhow::Result<CommandOutput> {
            anyhow::bail!("unexpected scalar configuration query")
        }

        fn execute_data(
            &self,
            _: &str,
            _: &[String],
            _: &Path,
            _: &BTreeMap<OsString, OsString>,
            _: &[u8],
        ) -> anyhow::Result<StructuredOutput> {
            Ok(StructuredOutput {
                status: 1,
                stdout: b"git-pr.tags\nuntrusted\0".to_vec(),
                stderr: Vec::new(),
            })
        }
    }
    let mut commands = World::new(0).commands();
    commands.runner = Arc::new(UnexpectedStdout);
    let error = Values::read(&commands, &["git-pr.tags"], None)
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("failed to read Git configuration")
    );
    assert!(error.to_string().contains("exit code 1"));
}

#[test]
fn configuration_records_preserve_effective_values_and_framing() {
    let values = Values::decode(
        "git-pr.stackup\ntrue\0git-pr.stackup\nfalse\0user.name\n  A\nB  \0git-pr.tags\0",
    )
    .unwrap();
    assert_eq!(values.boolean("Git-Pr.StackUp"), Some(false));
    assert_eq!(values.string("USER.name"), "A\nB");
    assert_eq!(values.get("git-pr.tags"), Some(""));
    assert_eq!(values.get("absent"), None);
    assert!(Values::decode("truncated").is_err());
    let world = World::new(0);
    assert!(
        Values::read(&world.commands(), &[], None)
            .unwrap()
            .get("any")
            .is_none()
    );
    assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
}

#[test]
fn typed_batch_failures_fall_back_to_git_effective_scalar_conversion() {
    let world = World::new(0);
    let keys = ["git-pr.stackUp", "git-pr.stackEmoji"];
    Values::read(&world.commands(), &keys, Some("bool")).unwrap();
    let call = world.trace().last().unwrap()["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    let batch = format!("git {call}");
    world.reply(&batch, 128, "superseded invalid Boolean");
    world.reply("git config --type=bool --get git-pr.stackUp", 0, "true");
    let values = Values::read(&world.commands(), &keys, Some("bool")).unwrap();
    assert_eq!(values.boolean("git-pr.stackUp"), Some(true));
    assert_eq!(values.boolean("git-pr.stackEmoji"), None);
    world.fail("git config --type=bool --get git-pr.stackUp");
    assert!(Values::read(&world.commands(), &keys, Some("bool")).is_err());
    world.fail(&batch);
    assert!(Values::read(&world.commands(), &keys, Some("bool")).is_err());
}

#[test]
fn malformed_bulk_and_effective_scalar_values_are_not_absence() {
    let world = World::new(0);
    let keys = ["git-pr.stackUp"];
    Values::read(&world.commands(), &keys, Some("bool")).unwrap();
    let call = world.trace().last().unwrap()["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    let batch = format!("git {call}");
    world.reply(&batch, 128, "invalid Boolean");
    world.reply(
        "git config --type=bool --get git-pr.stackUp",
        128,
        "bad effective value",
    );
    assert!(
        format!(
            "{:#}",
            Values::read(&world.commands(), &keys, Some("bool"))
                .err()
                .unwrap()
        )
        .contains("failed to read Git configuration")
    );
    world.reply(&batch, 0, "truncated");
    assert!(Values::read(&world.commands(), &keys, Some("bool")).is_err());
}

#[test]
fn invalid_utf8_configuration_output_is_rejected() {
    use crate::commands::{CommandOutput, CommandRunner, StructuredOutput};
    use std::{collections::BTreeMap, ffi::OsString, path::Path, sync::Arc};
    #[derive(Debug)]
    struct InvalidUtf8;
    impl CommandRunner for InvalidUtf8 {
        fn execute(
            &self,
            _: &str,
            _: &[String],
            _: &Path,
            _: &BTreeMap<OsString, OsString>,
        ) -> anyhow::Result<CommandOutput> {
            anyhow::bail!("unexpected unstructured command")
        }
        fn execute_data(
            &self,
            _: &str,
            _: &[String],
            _: &Path,
            _: &BTreeMap<OsString, OsString>,
            _: &[u8],
        ) -> anyhow::Result<StructuredOutput> {
            Ok(StructuredOutput {
                status: 0,
                stdout: vec![255],
                stderr: Vec::new(),
            })
        }
    }
    let mut commands = World::new(0).commands();
    commands.runner = Arc::new(InvalidUtf8);
    assert!(Values::read(&commands, &["git-pr.tags"], None).is_err());
}

#[test]
fn stack_orientation_migration_preserves_values_and_reports_conflicts() {
    use crate::cli::Flags;
    use std::{collections::BTreeMap, fmt::Write as _};
    for current in [None, Some(false), Some(true)] {
        for legacy in [None, Some(false), Some(true)] {
            let mut records = String::new();
            for (key, value) in [("git-pr.stackup", current), ("git-pr.reverse", legacy)] {
                if let Some(value) = value {
                    write!(records, "{key}\n{value}\0").unwrap();
                }
            }
            let values = if records.is_empty() {
                Values::default()
            } else {
                Values::decode(&records).unwrap()
            };
            let world = World::new(0);
            let result = super::super::stack_up(&Flags::default(), &values, &world.commands());
            if current.is_some() && legacy.is_some() && current != legacy {
                let error = result.unwrap_err().to_string();
                assert!(error.contains("conflicting git-pr.stackUp"));
                assert!(error.contains("remove git-pr.reverse"));
            } else {
                assert_eq!(result.unwrap(), current.or(legacy).unwrap_or(true));
                assert_eq!(
                    world.0.lock().unwrap().stderr.matches("deprecated").count(),
                    usize::from(legacy.is_some())
                );
            }
            for args in [
                vec!["--stack-up=false"],
                vec!["--reverse=false"],
                vec!["--stack-up=false", "--reverse=false"],
            ] {
                let flags = Flags::parse(args.iter().map(|s| (*s).into()).collect()).unwrap();
                assert!(!super::super::stack_up(&flags, &values, &world.commands()).unwrap());
                world.0.lock().unwrap().trace.clear();
                super::super::boolean_settings(&flags, &BTreeMap::default(), &world.commands())
                    .unwrap();
                let trace = serde_json::to_string(&world.trace()).unwrap();
                assert!(
                    !trace.contains("stackup") && !trace.contains("reverse"),
                    "{trace}"
                );
            }
        }
    }
}

#[test]
fn conflicting_stack_orientation_stops_configuration_before_publication() {
    let world = World::new(1);
    world.0.lock().unwrap().config.extend([
        ("git-pr.stackUp".into(), "true".into()),
        ("git-pr.reverse".into(), "false".into()),
    ]);
    let error = world.run(&[]).unwrap_err().to_string();
    assert!(error.contains("conflicting git-pr.stackUp"), "{error}");
    assert!(world.trace().iter().all(|event| {
        event["event"] != "http"
            && ["rev-parse", "config"].contains(&event["args"][0].as_str().unwrap_or(""))
    }));
}
