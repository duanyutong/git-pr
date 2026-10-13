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
    world: Arc<World>,
}

impl Recorder {
    pub(crate) fn new(world: Arc<World>) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::default(),
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
    fn progress(&self, label: &str, done: usize, total: usize) {
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
        assert!(events.iter().any(|event| event.label
            == format!("Creating Feature {}", index + 1)
            && event.done == index
            && event.total == 3));
        assert!(
            events
                .iter()
                .any(|event| event.label == format!("Created #{}", index + 101)
                    && event.done == index + 1)
        );
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
