use super::*;
use crate::{
    commands::{Clock as _, Terminal},
    tests::unit::support::World,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn oid(number: u64) -> String {
    format!("{number:040x}")
}

fn fixture(head: u64) -> Arc<World> {
    let world = World::new(0);
    let mut memory = world.0.lock().unwrap();
    memory.prs.insert(201, json!({"number":201,"state":"open","head":{"ref":"outside","sha":oid(head)},"base":{"ref":"topic/1"}}));
    for number in 0..=5 {
        memory.parents.insert(
            oid(number),
            if number == 0 {
                Vec::new()
            } else {
                vec![oid(number - 1)]
            },
        );
    }
    drop(memory);
    world
}

fn verify(
    world: &Arc<World>,
    approvals: &Approvals,
    terminal: Option<Arc<dyn Terminal>>,
    auto_accept: bool,
) -> Result<Instant> {
    let mut config = world.config();
    config.auto_accept = auto_accept;
    let mut commands = world.commands();
    commands.explain = true;
    if let Some(terminal) = terminal {
        commands.terminal = terminal;
    }
    let github = GitHub {
        session: crate::github::Session::default(),
        config: &config,
        commands: &commands,
        clock: Some(world.as_ref()),
        transport: world.as_ref(),
        template: "",
    };
    approvals.verify(
        &github,
        &["topic/1".into()],
        &HashMap::new(),
        &BTreeMap::from([("topic/1", oid(3).as_str())]),
    )
}

fn confirmations(world: &World) -> usize {
    world
        .trace()
        .iter()
        .filter(|event| event["kind"] == "confirm")
        .count()
}

#[test]
fn divergent_and_unavailable_outside_heads_do_not_require_confirmation_when_refuted() {
    for head in [4, 99] {
        let world = fixture(head);
        verify(&world, &Approvals::default(), None, false).unwrap();
        assert_eq!(confirmations(&world), 0);
        assert!(
            world
                .0
                .lock()
                .unwrap()
                .stderr
                .contains("indirect merge: false")
        );
    }
}

#[test]
fn reachable_outside_heads_require_consent_and_are_reobserved_after_acceptance() {
    for (accept, automatic) in [(false, false), (true, false), (true, true)] {
        let world = fixture(2);
        world.0.lock().unwrap().answers.push_back(accept);
        let result = verify(&world, &Approvals::default(), None, automatic);
        assert_eq!(result.is_ok(), accept);
        assert_eq!(confirmations(&world), usize::from(!automatic));
        let memory = world.0.lock().unwrap();
        assert!(memory.stderr.contains("PR #201") && memory.stderr.contains("indirectly merged"));
        let observations = memory
            .trace
            .iter()
            .filter(|event| event["kind"] == "http")
            .count();
        assert_eq!(observations, if accept { 2 } else { 1 });
        drop(memory);
    }
}

#[test]
fn immutable_proofs_are_reused_but_live_heads_are_always_reobserved() {
    let world = fixture(4);
    let approvals = Approvals::default();
    verify(&world, &approvals, None, false).unwrap();
    world.0.lock().unwrap().trace.clear();

    verify(&world, &approvals, None, false).unwrap();

    let trace = world.trace();
    assert_eq!(
        trace.iter().filter(|event| event["kind"] == "http").count(),
        1
    );
    assert!(!trace.iter().any(|event| {
        event["program"] == "git"
            && matches!(event["args"][1].as_str(), Some("cat-file" | "rev-list"))
    }));
    world.0.lock().unwrap().prs.get_mut(&201).unwrap()["head"]["sha"] = json!(oid(2));
    assert!(
        verify(&world, &approvals, None, false)
            .unwrap_err()
            .to_string()
            .contains("declined")
    );
    assert!(world.trace().iter().any(|event| {
        event["program"] == "git"
            && matches!(event["args"][1].as_str(), Some("cat-file" | "rev-list"))
    }));
    assert_eq!(confirmations(&world), 1);
}

#[test]
fn outside_tracking_is_scoped_and_clears_removed_dependants() {
    let world = fixture(4);
    let approvals = Approvals::default();
    verify(&world, &approvals, None, false).unwrap();
    assert!(approvals.has_outside(&["topic/1".into()]));
    assert!(!approvals.has_outside(&["topic/2".into()]));
    world.0.lock().unwrap().prs.clear();

    verify(&world, &approvals, None, false).unwrap();

    assert!(!approvals.has_outside(&["topic/1".into()]));
}

#[test]
fn inconclusive_ancestry_is_not_cached() {
    let world = fixture(99);
    let approvals = Approvals::default();
    world.0.lock().unwrap().parents.clear();
    assert!(verify(&world, &approvals, None, false).is_err());
    world.0.lock().unwrap().parents.insert(oid(3), Vec::new());

    verify(&world, &approvals, None, false).unwrap();

    assert_eq!(confirmations(&world), 0);
}

#[derive(Debug)]
struct DuringConfirmation {
    world: Arc<World>,
    calls: AtomicUsize,
    change_head: bool,
}

impl Terminal for DuringConfirmation {
    fn out(&self, text: &str) {
        self.world.out(text);
    }
    fn err(&self, text: &str) {
        self.world.err(text);
    }
    fn confirm(&self, prompt: &str) -> bool {
        let accepted = self.world.confirm(prompt);
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        let mut memory = self.world.0.lock().unwrap();
        memory.clock_elapsed += std::time::Duration::from_secs(31);
        if self.change_head {
            memory.prs.get_mut(&201).unwrap()["head"]["sha"] =
                json!(oid(u64::try_from(call).unwrap() % 3 + 1));
        }
        drop(memory);
        accepted
    }
}

#[test]
fn confirmation_waits_refresh_evidence_and_changed_heads_require_new_consent() {
    for change in [false, true] {
        let world = fixture(2);
        world.0.lock().unwrap().answers.extend([true, false]);
        let terminal = Arc::new(DuringConfirmation {
            world: world.clone(),
            calls: AtomicUsize::new(0),
            change_head: change,
        });
        let result = verify(&world, &Approvals::default(), Some(terminal), false);
        if change {
            assert!(result.unwrap_err().to_string().contains("declined"));
            assert_eq!(confirmations(&world), 2);
        } else {
            assert_eq!(result.unwrap(), world.now());
            assert_eq!(confirmations(&world), 1);
        }
    }
}

#[test]
fn changing_evidence_has_a_bounded_confirmation_loop() {
    // A new branch name invalidates consent even when its commit is unchanged.
    #[derive(Debug)]
    struct Renaming(Arc<World>, AtomicUsize);
    impl Terminal for Renaming {
        fn out(&self, text: &str) {
            self.0.out(text);
        }
        fn err(&self, text: &str) {
            self.0.err(text);
        }
        fn confirm(&self, prompt: &str) -> bool {
            let accepted = self.0.confirm(prompt);
            self.0.0.lock().unwrap().prs.get_mut(&201).unwrap()["head"]["ref"] = json!(format!(
                "outside-{}",
                self.1.fetch_add(1, Ordering::Relaxed)
            ));
            accepted
        }
    }
    let world = fixture(2);
    world.0.lock().unwrap().answers.extend([true; 4]);
    let terminal = Arc::new(Renaming(world.clone(), AtomicUsize::new(0)));
    assert!(
        verify(&world, &Approvals::default(), Some(terminal), false)
            .unwrap_err()
            .to_string()
            .contains("changed repeatedly")
    );
    assert_eq!(confirmations(&world), 4);
}

#[test]
fn inconclusive_history_and_invalid_head_objects_stop_without_confirmation() {
    let world = fixture(99);
    world.0.lock().unwrap().parents.clear();
    assert!(
        format!(
            "{:#}",
            verify(&world, &Approvals::default(), None, false).unwrap_err()
        )
        .contains("cannot determine")
    );
    assert_eq!(confirmations(&world), 0);
    for hash in [serde_json::Value::Null, json!("invalid")] {
        world.0.lock().unwrap().prs.get_mut(&201).unwrap()["head"]["sha"] = hash;
        assert!(
            verify(&world, &Approvals::default(), None, false)
                .unwrap_err()
                .to_string()
                .contains("head object")
        );
    }
}

#[test]
fn publication_context_must_match_the_observed_dependant_identity() {
    let world = fixture(4);
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        config: &config,
        commands: &commands,
        clock: Some(world.as_ref()),
        transport: world.as_ref(),
        template: "",
    };
    let approvals = Approvals::default();
    assert!(
        approvals
            .verify(
                &github,
                &["topic/1".into()],
                &HashMap::new(),
                &BTreeMap::new()
            )
            .unwrap_err()
            .to_string()
            .contains("missing proposed")
    );
    assert!(
        github
            .publication_dependants(
                &["topic/1".into()],
                &HashMap::from([(201, ("different", "topic/1"))])
            )
            .unwrap_err()
            .to_string()
            .contains("changed its selected head or base")
    );
    for node in [
        json!({"number":0,"headRefName":"outside"}),
        json!({"number":201,"headRefName":""}),
    ] {
        world.http("POST graphql", vec![crate::tests::unit::support::Response::Output(200,
            json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
        assert!(
            github
                .publication_dependants(&["topic/1".into()], &HashMap::new())
                .unwrap_err()
                .to_string()
                .contains("dependant PR")
        );
    }
}

#[test]
fn an_approved_head_requires_new_consent_for_a_different_proposed_base_object() {
    let world = fixture(2);
    world.0.lock().unwrap().answers.extend([true, false]);
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        config: &config,
        commands: &commands,
        clock: Some(world.as_ref()),
        transport: world.as_ref(),
        template: "",
    };
    let approvals = Approvals::default();
    for (proposed, accepted) in [(3, true), (4, false)] {
        assert_eq!(
            approvals
                .verify(
                    &github,
                    &["topic/1".into()],
                    &HashMap::new(),
                    &BTreeMap::from([("topic/1", oid(proposed).as_str())])
                )
                .is_ok(),
            accepted
        );
    }
    assert_eq!(confirmations(&world), 2);
}

#[test]
fn missing_ancestry_facts_cannot_be_classified_as_non_merges() {
    let candidate = Risk {
        number: 201,
        head: "outside".into(),
        base: "topic/1".into(),
        head_hash: oid(2),
        base_hash: oid(3),
    };
    assert!(
        Approvals::classify(&[candidate], &BTreeMap::new())
            .unwrap_err()
            .to_string()
            .contains("missing verified dependant ancestry")
    );
}
