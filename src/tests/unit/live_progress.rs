//! Progress reports confirmed work and identifies targets before external effects.

use super::support::{World, commit};
use crate::{
    cli::Flags,
    commands::Terminal,
    github::http::{Request, Transport},
    github::{GitHub, Session},
    model::Commit,
};
use anyhow::Result;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Progress {
    pub label: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug)]
pub(crate) struct Recorder {
    pub events: Mutex<Vec<Progress>>,
    active: Mutex<Option<String>>,
    world: Arc<World>,
}

impl Recorder {
    pub(crate) fn new(world: Arc<World>) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::default(),
            active: Mutex::default(),
            world,
        })
    }

    fn latest(&self) -> Progress {
        self.events.lock().unwrap().last().unwrap().clone()
    }
}

impl Terminal for Recorder {
    fn out(&self, text: &str) {
        self.world.out(text);
    }
    fn err(&self, text: &str) {
        self.world.err(text);
    }
    fn confirm(&self, prompt: &str) -> bool {
        self.world.confirm(prompt)
    }
    fn has_progress(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }
    fn clear_progress(&self, label: &str) {
        let mut active = self.active.lock().unwrap();
        if active
            .as_ref()
            .is_some_and(|value| value.starts_with(label))
        {
            *active = None;
        }
    }
    fn progress(&self, label: &str, done: usize, total: usize) {
        *self.active.lock().unwrap() = Some(label.into());
        assert!(done <= total);
        self.events.lock().unwrap().push(Progress {
            label: label.into(),
            done,
            total,
        });
    }
}

#[derive(Debug)]
struct ObservedTransport {
    world: Arc<World>,
    progress: Arc<Recorder>,
    dispatches: Mutex<Vec<(Vec<u64>, Progress)>>,
}

impl Transport for ObservedTransport {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let payload: Value = request
            .body
            .map_or(Value::Null, |body| serde_json::from_slice(body).unwrap());
        if payload["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("mutation"))
        {
            let numbers: Vec<u64> = payload["variables"]
                .as_object()
                .unwrap()
                .values()
                .map(|input| {
                    input["pullRequestId"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("PR_")
                        .parse()
                        .unwrap()
                })
                .collect();
            let progress = self.progress.latest();
            for number in &numbers {
                assert!(
                    progress.label.contains(&format!("#{number}")),
                    "{progress:?}"
                );
            }
            assert!(progress.done < progress.total);
            self.dispatches.lock().unwrap().push((numbers, progress));
        }
        self.world.request(request)
    }
}

#[test]
fn description_progress_names_each_batch_before_dispatch_and_counts_acknowledged_targets() {
    let world = World::new(12);
    world.run(&["--no-stack"]).unwrap();
    let recorder = Recorder::new(world.clone());
    let mut commands = world.commands();
    commands.terminal = recorder.clone();
    let config = world.config();
    let transport = ObservedTransport {
        world: world.clone(),
        progress: recorder.clone(),
        dispatches: Mutex::default(),
    };
    let github = GitHub {
        config: &config,
        commands: &commands,
        transport: &transport,
        session: Session::default(),
        clock: Some(world.as_ref()),
        template: "",
    };
    let commits: Vec<_> = (1..=12)
        .map(|number| Commit {
            pr_number: number + 100,
            title: format!("Updated {number}"),
            ..commit(number)
        })
        .collect();
    github
        .update_content(&commits.iter().collect::<Vec<_>>(), &mut |_, _| Ok(None))
        .unwrap();
    let dispatches = transport.dispatches.lock().unwrap().clone();
    assert_eq!(
        dispatches
            .iter()
            .map(|(numbers, progress)| (numbers.len(), progress.done, progress.total))
            .collect::<Vec<_>>(),
        vec![(5, 0, 12), (5, 5, 12), (2, 10, 12)]
    );
    assert_eq!(
        recorder.latest(),
        Progress {
            label: "Descriptions".into(),
            done: 12,
            total: 12
        }
    );
    recorder.events.lock().unwrap().clear();
    transport.dispatches.lock().unwrap().clear();
    github
        .update_content(&commits.iter().collect::<Vec<_>>(), &mut |_, _| Ok(None))
        .unwrap();
    assert!(transport.dispatches.lock().unwrap().is_empty());
    let events = recorder.events.lock().unwrap().clone();
    for done in 1..=12 {
        assert!(
            events.iter().any(|event| event.label == "Descriptions"
                && event.done == done
                && event.total == 12)
        );
    }
}

#[test]
fn submission_reports_creation_labels_and_base_checks_without_changing_publication() {
    let world = World::new(3);
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert("git-pr.addLabel".into(), "reviewed".into());
    let recorder = Recorder::new(world.clone());
    let mut app = world.app();
    app.commands.terminal = recorder.clone();
    app.execute(
        &Flags::parse(vec!["--no-stack".into()]).unwrap(),
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
    .unwrap();
    let events = recorder.events.lock().unwrap().clone();
    for index in 0..3 {
        assert!(
            events.iter().any(|event| event.label == "PRs created"
                && event.done == index
                && event.total == 3)
        );
        assert!(events.iter().any(|event| event.label == "PRs created"
            && event.done == index + 1
            && event.total == 3));
        assert!(
            events
                .iter()
                .any(|event| event.label == format!("Labels #{}", index + 101)
                    && event.done == index
                    && event.total == 3)
        );
    }
    assert!(
        events
            .iter()
            .any(|event| event.label == "Labels" && event.done == 3 && event.total == 3)
    );
    recorder.events.lock().unwrap().clear();
    let mut app = world.app();
    app.commands.terminal = recorder.clone();
    app.execute(
        &Flags::parse(vec!["--no-stack".into()]).unwrap(),
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
    .unwrap();
    let events = recorder.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|event| event.label == "Bases #101, #102, #103"
                && event.done == 0
                && event.total == 3)
    );
    assert!(
        events
            .iter()
            .any(|event| event.label == "Bases" && event.done == 3 && event.total == 3)
    );
    assert_eq!(world.0.lock().unwrap().prs.len(), 3);
}

#[test]
fn failed_base_and_label_checks_do_not_finish_their_counters() {
    let world = World::new(2);
    world.run(&["--no-stack"]).unwrap();
    let recorder = Recorder::new(world.clone());
    let mut commands = world.commands();
    commands.terminal = recorder.clone();
    let mut config = world.config();
    config.labels = vec!["reviewed".into()];
    let github = GitHub {
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        session: Session::default(),
        clock: Some(world.as_ref()),
        template: "",
    };
    let commits: Vec<_> = (1..=2)
        .map(|number| Commit {
            pr_number: number + 100,
            ..commit(number)
        })
        .collect();
    assert!(
        github
            .verify_bases(
                commits
                    .iter()
                    .map(|commit| (commit, "unexpected"))
                    .collect()
            )
            .is_err()
    );
    assert_eq!(
        recorder.latest(),
        Progress {
            label: "Bases #101, #102".into(),
            done: 0,
            total: 2
        }
    );
    world.http(
        "POST issues/101/labels",
        vec![super::support::Response::Output(403, "denied".into())],
    );
    assert!(
        github
            .update_labels(
                &commits
                    .iter()
                    .map(|commit| (commit, false))
                    .collect::<Vec<_>>()
            )
            .is_err()
    );
    assert_eq!(
        recorder.latest(),
        Progress {
            label: "Labels #101".into(),
            done: 0,
            total: 2
        }
    );
}

#[test]
fn explicit_fetch_activity_retains_the_existing_command_boundary() {
    let world = World::new(1);
    world.reply("git fetch origin", 0, "");
    world.commands().git(&["fetch", "origin"]).unwrap();
    assert_eq!(
        world
            .trace()
            .iter()
            .filter(|event| event["args"][0] == "fetch")
            .count(),
        1
    );
}

#[test]
fn git_fallback_progress_checks_ten_selected_branches_without_counting_the_helper() {
    let world = World::new(1);
    world.run(&["--no-stack"]).unwrap();
    {
        let mut memory = world.0.lock().unwrap();
        memory.commits = (1..=10).map(commit).collect();
        memory.branch = "topic/10".into();
    }
    let recorder = Recorder::new(world.clone());
    let mut app = world.app();
    app.commands.terminal = recorder.clone();
    app.execute(
        &Flags::parse(vec!["--no-stack".into()]).unwrap(),
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
    .unwrap();
    let events = recorder
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.label == "Remote branches checked")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        vec![
            Progress {
                label: "Remote branches checked".into(),
                done: 0,
                total: 10
            },
            Progress {
                label: "Remote branches checked".into(),
                done: 10,
                total: 10
            },
        ]
    );
    assert_eq!(world.0.lock().unwrap().prs.len(), 10);
}

#[test]
fn git_ref_progress_advances_only_validated_batches_and_clears_after_rejection() {
    let world = World::new(1);
    let config = world.config();
    let recorder = Recorder::new(world.clone());
    let mut commands = world.commands();
    commands.terminal = recorder.clone();
    let git = crate::git::Git {
        commands: &commands,
        repo: &config.repo,
    };
    let mut references = (1..=129).map(|i| format!("topic/{i}")).collect::<Vec<_>>();
    references.push("git-pr/transition/101".into());
    assert_eq!(git.recovery_refs("origin", &references).unwrap().len(), 130);
    let counts = recorder
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|event| (event.done, event.total))
        .collect::<Vec<_>>();
    assert_eq!(counts, vec![(0, 129), (128, 129), (129, 129)]);
    assert!(!recorder.has_progress());
    recorder.events.lock().unwrap().clear();
    world.reply(
        "git ls-remote --refs -- origin refs/heads/topic/1 refs/heads/topic/2",
        0,
        "malformed",
    );
    assert!(git.recovery_refs("origin", &references[..2]).is_err());
    assert_eq!(
        *recorder.events.lock().unwrap(),
        vec![Progress {
            label: "Remote branches checked".into(),
            done: 0,
            total: 2
        }]
    );
    assert!(!recorder.has_progress());
}
