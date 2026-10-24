//! Bounded transition and fault campaigns using isolated, observable boundaries.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::support::World;
use crate::{
    cli::Flags,
    commands::{CommandOutput, CommandRunner},
    github::http::{Request, Transport},
    submission::Application,
};

fn submission_effect(event: &Value) -> bool {
    matches!(
        event["args"][0].as_str(),
        Some("push" | "reword" | "describe")
    ) || matches!(event["method"].as_str(), Some("PATCH" | "DELETE"))
        || event["method"] == "POST"
            && (event["path"] != "graphql"
                || event["body"]["query"]
                    .as_str()
                    .is_some_and(|query| query.starts_with("mutation")))
}

fn run(app: &Application, args: &[&str]) -> Result<()> {
    let flags = Flags::parse(args.iter().map(|arg| (*arg).into()).collect()).unwrap();
    app.clone().execute(
        &flags,
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
}

#[derive(Clone, Copy, Debug)]
enum Transition {
    Preview,
    Repeat,
    AmendTip,
    SelectTip,
    HumanEdit,
}

#[test]
fn all_three_step_workflow_sequences_preserve_identity_and_human_text() {
    let actions = [
        Transition::Preview,
        Transition::Repeat,
        Transition::AmendTip,
        Transition::SelectTip,
        Transition::HumanEdit,
    ];
    for first in actions {
        for second in actions {
            for third in actions {
                let sequence = [first, second, third];
                let world = World::new(3);
                world.run(&["--no-stack"]).unwrap();
                let mut human_notes = Vec::new();
                let mut amendment = 0;
                for (step, action) in sequence.iter().enumerate() {
                    world.0.lock().unwrap().trace.clear();
                    match action {
                        Transition::Preview => {
                            let before = {
                                let m = world.0.lock().unwrap();
                                (m.commits.clone(), m.prs.clone(), m.remote_refs.clone())
                            };
                            world.run(&["--no-stack", "--dry-run"]).unwrap();
                            let m = world.0.lock().unwrap();
                            assert_eq!(
                                (&m.commits, &m.prs, &m.remote_refs),
                                (&before.0, &before.1, &before.2),
                                "{sequence:?}"
                            );
                            assert!(!m.trace.iter().any(submission_effect), "{sequence:?}");
                            drop(m);
                        }
                        Transition::Repeat => {
                            world.run(&["--no-stack"]).unwrap();
                            let trace = world.trace();
                            assert!(
                                !trace
                                    .iter()
                                    .any(|event| event["method"] == "POST"
                                        && event["path"] == "pulls"),
                                "{sequence:?}"
                            );
                        }
                        Transition::AmendTip => {
                            amendment += 1;
                            let mut m = world.0.lock().unwrap();
                            m.commits[2].hash = format!("c{amendment:039x}");
                            m.commits[2].title = format!("Amended feature {amendment}");
                        }
                        Transition::SelectTip => {
                            world.run(&["--no-stack", "topic/3"]).unwrap();
                        }
                        Transition::HumanEdit => {
                            let note = format!("Human note at step {step}");
                            let mut m = world.0.lock().unwrap();
                            for pr in m.prs.values_mut() {
                                pr["body"] =
                                    json!(format!("{note}\n\n{}", pr["body"].as_str().unwrap()));
                            }
                            drop(m);
                            human_notes.push(note);
                        }
                    }
                    let m = world.0.lock().unwrap();
                    assert_eq!(m.prs.len(), 3, "{sequence:?}");
                    for number in 101..=103 {
                        assert_eq!(
                            m.prs[&number]["head"]["ref"],
                            format!("topic/{}", number - 100),
                            "{sequence:?}"
                        );
                        assert_eq!(
                            m.prs[&number]["base"]["ref"],
                            if number == 101 {
                                "main".into()
                            } else {
                                format!("topic/{}", number - 101)
                            },
                            "{sequence:?}"
                        );
                        for note in &human_notes {
                            assert!(
                                m.prs[&number]["body"].as_str().unwrap().contains(note),
                                "{sequence:?}: missing {note}"
                            );
                        }
                    }
                    drop(m);
                }
                world.run(&["--no-stack"]).unwrap();
                let m = world.0.lock().unwrap();
                assert_eq!(m.prs.len(), 3);
                assert_eq!(m.prs[&103]["title"], m.commits[2].title);
                for commit in &m.commits {
                    assert_eq!(
                        m.remote_refs
                            .get(&format!("refs/heads/{}", commit.remote_ref())),
                        Some(&commit.hash)
                    );
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Discovery,
    Binding,
    Push,
    Create,
    Content,
    Labels,
    Topology,
}

#[derive(Debug)]
struct BoundaryFault {
    world: Arc<World>,
    boundary: Boundary,
    after_effect: bool,
    fail_at: usize,
    seen: AtomicUsize,
}

impl BoundaryFault {
    fn inject(&self) -> bool {
        self.seen.fetch_add(1, Ordering::SeqCst) + 1 == self.fail_at
    }

    fn record(&self) {
        self.world.0.lock().unwrap().trace.push(json!({"kind":"fault", "boundary":format!("{:?}", self.boundary), "after_effect":self.after_effect}));
    }
}

impl CommandRunner for BoundaryFault {
    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        let targeted = program == "git"
            && matches!(
                (self.boundary, args.first().map(String::as_str)),
                (Boundary::Discovery, Some("symbolic-ref")) | (Boundary::Push, Some("push"))
            );
        let targeted = targeted
            || (program == "git"
                && self.boundary == Boundary::Binding
                && args.starts_with(&["config".into(), "--local".into(), "--add".into()]));
        let failed = targeted && self.inject();
        if failed && !self.after_effect {
            self.record();
            bail!("injected before {:?}", self.boundary);
        }
        let output = self.world.execute(program, args, cwd, env)?;
        if failed {
            self.record();
            bail!("injected lost acknowledgement after {:?}", self.boundary);
        }
        Ok(output)
    }
}

impl Transport for BoundaryFault {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let path = reqwest::Url::parse(request.url).unwrap().path().to_owned();
        let body: Value = request
            .body
            .map_or(Value::Null, |bytes| serde_json::from_slice(bytes).unwrap());
        let mutation = body["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("mutation"));
        let targeted = match self.boundary {
            Boundary::Create => request.method == "POST" && path.ends_with("/pulls"),
            Boundary::Content => mutation,
            Boundary::Labels => request.method == "POST" && path.ends_with("/labels"),
            Boundary::Topology => request.method == "POST" && path.ends_with("/stacks"),
            _ => false,
        };
        let failed = targeted && self.inject();
        if failed && !self.after_effect {
            self.record();
            bail!("injected before {:?}", self.boundary);
        }
        let output = self.world.request(request)?;
        if failed {
            self.record();
            bail!("injected lost acknowledgement after {:?}", self.boundary);
        }
        Ok(output)
    }
}

#[test]
fn before_effect_and_lost_acknowledgement_failures_converge_without_duplicate_prs() {
    for boundary in [
        Boundary::Discovery,
        Boundary::Binding,
        Boundary::Push,
        Boundary::Create,
        Boundary::Content,
        Boundary::Labels,
        Boundary::Topology,
    ] {
        for after_effect in [false, true] {
            let positions = if matches!(boundary, Boundary::Create | Boundary::Labels) {
                3
            } else {
                1
            };
            for fail_at in 1..=positions {
                let world = World::new(if boundary == Boundary::Binding { 1 } else { 3 });
                if boundary == Boundary::Binding {
                    let mut m = world.0.lock().unwrap();
                    m.branchless = true;
                    m.commits[0].attrs.clear();
                }
                if boundary == Boundary::Content {
                    world.run(&["--no-stack"]).unwrap();
                    let mut m = world.0.lock().unwrap();
                    for pr in m.prs.values_mut() {
                        pr["body"] = json!(format!(
                            "Human text retained\n\n{}",
                            pr["body"].as_str().unwrap()
                        ));
                    }
                    for commit in &mut m.commits {
                        commit.title.push_str(" amended");
                    }
                }
                world.0.lock().unwrap().trace.clear();
                let fault = Arc::new(BoundaryFault {
                    world: world.clone(),
                    boundary,
                    after_effect,
                    fail_at,
                    seen: AtomicUsize::new(0),
                });
                let mut app = world.app();
                app.commands.runner = fault.clone();
                app.transport = fault.clone();
                let flags = if boundary == Boundary::Topology {
                    vec!["--github-stack", "--yes", "--add-label=required"]
                } else {
                    vec!["--no-stack", "--add-label=required"]
                };
                let initial = run(&app, &flags);
                assert!(
                    fault.seen.load(Ordering::SeqCst) >= fail_at,
                    "boundary={boundary:?}, after={after_effect}, position={fail_at}: {initial:?}"
                );
                if !after_effect
                    && matches!(
                        boundary,
                        Boundary::Discovery | Boundary::Binding | Boundary::Push
                    )
                {
                    assert!(initial.is_err());
                    assert!(world.0.lock().unwrap().prs.is_empty());
                    assert!(world.0.lock().unwrap().remote_refs.is_empty());
                }
                if boundary == Boundary::Create && !after_effect {
                    assert!(
                        format!("{:#}", run(&app, &flags).unwrap_err()).contains("unknown outcome")
                    );
                    // The injected fixture proves that this request was never dispatched.
                    world
                        .0
                        .lock()
                        .unwrap()
                        .config
                        .remove("git-pr.pendingCreate");
                }
                // Completed effects are retained. A fresh invocation observes before replay.
                run(&app, &flags).unwrap_or_else(|error| {
                    panic!(
                        "boundary={boundary:?}, after={after_effect}, position={fail_at}: {error:#}"
                    )
                });
                let count = world.0.lock().unwrap().commits.len();
                {
                    let m = world.0.lock().unwrap();
                    assert_eq!(m.prs.len(), count, "{boundary:?}/{after_effect}/{fail_at}");
                    for (index, commit) in m.commits.iter().enumerate() {
                        let number = 101 + u64::try_from(index).unwrap();
                        let pr = &m.prs[&number];
                        assert_eq!(pr["head"]["ref"], format!("topic/{}", index + 1));
                        assert_eq!(pr["title"], commit.title);
                        assert_eq!(
                            pr["base"]["ref"],
                            if index == 0 {
                                "main".into()
                            } else {
                                format!("topic/{index}")
                            }
                        );
                        assert!(
                            pr["labels"]
                                .as_array()
                                .unwrap()
                                .contains(&json!("required"))
                        );
                        assert_eq!(
                            m.remote_refs
                                .get(&format!("refs/heads/topic/{}", index + 1)),
                            Some(&commit.hash)
                        );
                        if boundary == Boundary::Content {
                            assert!(pr["body"].as_str().unwrap().contains("Human text retained"));
                        }
                    }
                }
                let trace = world.trace();
                let creations = trace
                    .iter()
                    .filter(|event| event["method"] == "POST" && event["path"] == "pulls")
                    .count();
                assert_eq!(
                    creations,
                    if boundary == Boundary::Content {
                        0
                    } else {
                        count
                    },
                    "replayed creation: {boundary:?}/{after_effect}/{fail_at}"
                );
                if boundary == Boundary::Labels {
                    assert_eq!(
                        trace
                            .iter()
                            .filter(|event| event["method"] == "POST"
                                && event["path"]
                                    .as_str()
                                    .is_some_and(|path| path.ends_with("/labels")))
                            .count(),
                        count,
                        "replayed label addition: {after_effect}/{fail_at}"
                    );
                }
                if boundary == Boundary::Content {
                    let mut applied = trace
                        .iter()
                        .filter(|event| {
                            event["body"]["query"]
                                .as_str()
                                .is_some_and(|query| query.starts_with("mutation"))
                        })
                        .flat_map(|event| {
                            event["body"]["variables"]
                                .as_object()
                                .unwrap()
                                .values()
                                .map(|input| input["pullRequestId"].as_str().unwrap().to_owned())
                        })
                        .collect::<Vec<_>>();
                    applied.sort();
                    assert_eq!(
                        applied,
                        ["PR_101", "PR_102", "PR_103"],
                        "replayed completed content: {after_effect}/{fail_at}"
                    );
                }
                world.0.lock().unwrap().trace.clear();
                run(&app, &flags).unwrap();
                let trace = world.trace();
                assert!(
                    !trace
                        .iter()
                        .any(|event| event["method"] == "POST" && event["path"] == "pulls"),
                    "duplicate creation: {boundary:?}/{after_effect}/{fail_at}"
                );
                assert!(
                    !trace.iter().any(|event| event["method"] == "PATCH"
                        || event["method"] == "DELETE"
                        || event["path"]
                            .as_str()
                            .is_some_and(|path| path.ends_with("/labels"))
                            && event["method"] == "POST"
                        || event["body"]["query"]
                            .as_str()
                            .is_some_and(|query| query.starts_with("mutation"))),
                    "redundant metadata mutation: {boundary:?}/{after_effect}/{fail_at}"
                );
            }
        }
    }
}

#[test]
fn alternate_remote_mock_answers_and_publication_follow_the_selected_name() {
    let world = World::new(2);
    world.reply("git remote -v", 0, "publish https://github.com/org/repo.git (fetch)\npublish https://github.com/org/repo.git (push)");
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert("remote.pushDefault".into(), "publish".into());
    assert_eq!(
        world
            .commands()
            .git(&["symbolic-ref", "refs/remotes/publish/HEAD"])
            .unwrap(),
        "refs/remotes/publish/main"
    );
    world.run(&["--no-stack"]).unwrap();
    let trace = world.trace();
    assert!(trace.iter().any(|event| event["args"][0] == "symbolic-ref"
        && event["args"][1] == "refs/remotes/publish/HEAD"));
    assert!(trace.iter().any(|event| {
        event["args"][0] == "push"
            && event["args"]
                .as_array()
                .unwrap()
                .windows(2)
                .any(|pair| pair == [json!("--"), json!("publish")])
    }));
    assert_eq!(world.0.lock().unwrap().prs.len(), 2);
}

#[test]
fn remote_base_and_lifecycle_changes_stop_publication_before_any_further_pr_effects() {
    for field in ["base", "closed", "merged"] {
        for after_push in [false, true] {
            for number in [101, 102, 103] {
                let world = World::new(3);
                world.run(&["--no-stack"]).unwrap();
                let mut changed = world.0.lock().unwrap().prs[&number].clone();
                match field {
                    "base" => changed["base"]["ref"] = json!("human-chosen-base"),
                    "closed" => changed["state"] = json!("closed"),
                    "merged" => {
                        changed["state"] = json!("closed");
                        changed["merged"] = json!(true);
                    }
                    _ => unreachable!(),
                }
                if after_push {
                    world
                        .0
                        .lock()
                        .unwrap()
                        .changes_on_push
                        .insert(number, Some(changed));
                } else {
                    world.0.lock().unwrap().prs.insert(number, changed);
                }
                world.0.lock().unwrap().trace.clear();
                let error = world.run(&["--no-stack"]).unwrap_err();
                assert!(
                    format!("{error:#}").contains(&format!("#{number}")),
                    "{field}/{after_push}: {error:#}"
                );
                let trace = world.trace();
                let pushes = trace
                    .iter()
                    .filter(|event| event["args"][0] == "push")
                    .count();
                assert_eq!(pushes, usize::from(after_push), "{field}/{after_push}");
                assert!(
                    !trace
                        .iter()
                        .any(|event| event["kind"] == "http" && submission_effect(event)),
                    "{field}/{after_push}"
                );
                let state = world.0.lock().unwrap().prs.clone();
                assert_eq!(state.len(), 3);
                world.0.lock().unwrap().trace.clear();
                assert!(world.run(&["--no-stack"]).is_err());
                assert!(!world.trace().iter().any(submission_effect));
                assert_eq!(world.0.lock().unwrap().prs, state);
            }
        }
    }
}
