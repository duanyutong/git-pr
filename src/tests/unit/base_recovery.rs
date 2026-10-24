use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::*;
use crate::tests::unit::support::{Response, World};

fn oid(number: u64) -> String {
    format!("{number:040x}")
}

fn pr(number: u64, head: &str, hash: u64, base: &str) -> Value {
    json!({"number":number,"node_id":format!("PR_{number}"),"title":"Feature","body":"Human discussion","labels":[],"state":"open","merged":false,"merged_at":null,"merge_commit_sha":null,
        "head":{"ref":head,"sha":oid(hash),"repo":{"full_name":"org/repo"}},
        "base":{"ref":base,"sha":oid(0),"repo":{"full_name":"org/repo"}},"stack":null})
}

fn world(count: u64) -> Arc<World> {
    let w = World::new(count);
    {
        let mut m = w.0.lock().unwrap();
        let mut parent = pr(90, "obsolete", 10, "intermediate");
        parent["state"] = json!("closed");
        parent["merged"] = json!(true);
        parent["merge_commit_sha"] = json!(oid(0));
        parent["stack"] = json!({"number":8});
        m.prs.insert(90, parent);
        m.remote_refs.insert("refs/heads/main".into(), oid(0));
        m.remote_refs.insert("refs/heads/obsolete".into(), oid(10));
        for i in 1..=count {
            m.prs.insert(
                100 + i,
                pr(
                    100 + i,
                    &format!("topic/{i}"),
                    10 + i,
                    if i == 1 { "obsolete" } else { "topic/1" },
                ),
            );
            m.remote_refs
                .insert(format!("refs/heads/topic/{i}"), oid(10 + i));
        }
        drop(m);
    }
    // Two independent histories: a rewritten merge on trunk, and the old stack.
    let parents = BTreeMap::from([
        (0, vec![20]),
        (1, vec![0]),
        (2, vec![1]),
        (10, vec![20]),
        (11, vec![10]),
        (12, vec![11]),
        (20, vec![]),
    ]);
    for ancestor in parents.keys() {
        for descendant in parents.keys() {
            let mut pending = vec![*descendant];
            let mut seen = BTreeSet::new();
            while let Some(next) = pending.pop() {
                if seen.insert(next) {
                    pending.extend(&parents[&next]);
                }
            }
            w.reply(
                &format!(
                    "git merge-base --is-ancestor {} {}",
                    oid(*ancestor),
                    oid(*descendant)
                ),
                i32::from(!seen.contains(ancestor)),
                "",
            );
        }
    }
    w
}

fn effect(event: &Value) -> bool {
    event["args"][0] == "push"
        || event["method"] == "PATCH"
        || event["method"] == "DELETE"
        || (event["method"] == "POST" && event["path"] != "graphql")
}

fn with_github<T>(w: &Arc<World>, f: impl FnOnce(&GitHub<'_>) -> T) -> T {
    let config = w.config();
    let commands = w.commands();
    f(&GitHub {
        session: crate::github::Session::default(),
        config: &config,
        commands: &commands,
        clock: Some(w.as_ref()),
        transport: w.as_ref(),
        template: "",
    })
}

#[test]
fn merged_downstack_recovery_preserves_identity_and_retargets_before_leased_publication() {
    for count in [1, 2] {
        for deleted in [false, true] {
            for backend in ["git", "branchless", "jj"] {
                let w = world(count);
                {
                    let mut m = w.0.lock().unwrap();
                    m.branchless = backend == "branchless";
                    m.jj = backend == "jj";
                    if deleted {
                        m.remote_refs.remove("refs/heads/obsolete");
                    }
                }
                w.run(&["--no-stack"]).unwrap();
                let trace = w.trace();
                let patch = trace
                    .iter()
                    .position(|e| e["method"] == "PATCH" && e["body"]["base"] == "main")
                    .unwrap();
                let push = trace.iter().position(|e| e["args"][0] == "push").unwrap();
                assert!(patch < push);
                let args = trace[push]["args"].as_array().unwrap();
                assert!(!args.contains(&json!("-f")));
                assert!(args.contains(&json!(format!(
                    "--force-with-lease=refs/heads/topic/1:{}",
                    oid(11)
                ))));
                let m = w.0.lock().unwrap();
                assert_eq!(m.prs.len(), usize::try_from(count).unwrap() + 1);
                assert_eq!(m.prs[&101]["base"]["ref"], "main");
                assert_eq!(m.prs[&101]["state"], "open");
                assert_eq!(m.prs[&101]["node_id"], "PR_101");
                assert!(
                    m.prs[&101]["body"]
                        .as_str()
                        .unwrap()
                        .starts_with("Human discussion")
                );
                drop(m);
                w.0.lock().unwrap().trace.clear();
                w.run(&["--no-stack"]).unwrap();
                assert!(
                    !w.trace()
                        .iter()
                        .any(|e| e["method"] == "PATCH" && e["body"].get("base").is_some())
                );
            }
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the refusal matrix keeps evidence cases together"
)]
fn recovery_refuses_unproven_or_unsafe_evidence_without_effects() {
    for case in [
        "unmerged",
        "missing-result",
        "malformed-result",
        "unintegrated",
        "unrelated",
        "reused",
        "ambiguous",
        "missing-parent",
        "missing-trunk",
        "missing-head",
        "changed-head",
        "native",
        "unknown-membership",
        "closed",
        "already-landed",
        "prepared-landed",
        "unbased",
        "dependant",
        "bad-repository",
        "ancestry-error",
    ] {
        let w = world(1);
        {
            let mut m = w.0.lock().unwrap();
            match case {
                "unmerged" => m.prs.get_mut(&90).unwrap()["merged"] = json!(false),
                "missing-result" => m.prs.get_mut(&90).unwrap()["merge_commit_sha"] = Value::Null,
                "malformed-result" => {
                    m.prs.get_mut(&90).unwrap()["merge_commit_sha"] = json!("bad");
                }
                "unintegrated" => m.prs.get_mut(&90).unwrap()["merge_commit_sha"] = json!(oid(10)),
                "reused" => {
                    m.remote_refs.insert("refs/heads/obsolete".into(), oid(20));
                }
                "ambiguous" => {
                    let mut other = m.prs[&90].clone();
                    other["number"] = json!(91);
                    m.prs.insert(91, other);
                }
                "missing-parent" => {
                    m.prs.remove(&90);
                }
                "missing-trunk" => {
                    m.remote_refs.remove("refs/heads/main");
                }
                "missing-head" => {
                    m.remote_refs.remove("refs/heads/topic/1");
                }
                "changed-head" => {
                    m.remote_refs.insert("refs/heads/topic/1".into(), oid(12));
                }
                "native" => m.prs.get_mut(&101).unwrap()["stack"] = json!({"number":9}),
                "unknown-membership" => {
                    m.prs
                        .get_mut(&101)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("stack");
                }
                "closed" => m.prs.get_mut(&101).unwrap()["state"] = json!("closed"),
                "bad-repository" => {
                    m.prs.get_mut(&101).unwrap()["head"]["repo"]["full_name"] = json!("other/repo");
                }
                "dependant" => {
                    m.prs.insert(200, pr(200, "outside", 12, "topic/1"));
                }
                _ => {}
            }
        }
        match case {
            "unrelated" => w.reply(
                &format!("git merge-base --is-ancestor {} {}", oid(10), oid(11)),
                1,
                "",
            ),
            "already-landed" => w.reply(
                &format!("git merge-base --is-ancestor {} {}", oid(11), oid(0)),
                0,
                "",
            ),
            "prepared-landed" => w.reply(
                &format!("git merge-base --is-ancestor {} {}", oid(1), oid(0)),
                0,
                "",
            ),
            "unbased" => w.reply(
                &format!("git merge-base --is-ancestor {} {}", oid(0), oid(1)),
                1,
                "",
            ),
            "ancestry-error" => w.fail(&format!(
                "git merge-base --is-ancestor {} {}",
                oid(0),
                oid(0)
            )),
            _ => {}
        }
        let error = format!("{:#}", w.run(&["--no-stack", "--yes"]).unwrap_err());
        assert!(
            error.contains("no local metadata, pushes or PR changes"),
            "{case}: {error}"
        );
        assert!(!w.trace().iter().any(effect), "{case}: {error}");
    }
}

#[test]
fn preview_and_rewrite_checkpoint_do_not_repair_remote_bases() {
    for flags in [vec!["--dry-run"], vec!["--stop-after", "rewrite"]] {
        let w = world(1);
        w.run(&flags).unwrap();
        assert!(!w.trace().iter().any(effect));
        assert_eq!(w.0.lock().unwrap().prs[&101]["base"]["ref"], "obsolete");
    }
}

#[test]
fn recovery_can_create_an_unpublished_upper_pr_with_an_absence_lease() {
    let w = world(2);
    {
        let mut m = w.0.lock().unwrap();
        m.prs.remove(&102);
        m.remote_refs.remove("refs/heads/topic/2");
    }
    w.run(&["--no-stack"]).unwrap();
    assert!(w.trace().iter().any(|e| {
        e["args"]
            .as_array()
            .is_some_and(|args| args.contains(&json!("--force-with-lease=refs/heads/topic/2:")))
    }));
    assert_eq!(w.0.lock().unwrap().prs[&102]["base"]["ref"], "topic/1");
}

#[test]
fn recovery_revalidates_metadata_before_mutation_and_after_publication() {
    for call in [2, 3, 4, 5, 6] {
        let w = world(1);
        let original = w.0.lock().unwrap().prs[&101].clone();
        let mut replies = Vec::new();
        for index in 1..=call {
            let mut value = original.clone();
            if index >= 4 {
                value["base"]["ref"] = json!("main");
            }
            if index >= 6 {
                value["head"]["sha"] = json!(oid(1));
            }
            if index == call {
                value["node_id"] = json!("CHANGED");
            }
            replies.push(Response::Output(200, value.to_string()));
        }
        w.http("GET pulls/101", replies);
        let error = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
        assert!(error.contains("changed"), "{call}: {error}");
        assert_eq!(w.trace().iter().any(|e| e["args"][0] == "push"), call == 6);
        assert_eq!(w.trace().iter().any(|e| e["method"] == "PATCH"), call >= 4);
    }
}

#[test]
fn unconfirmed_retarget_stops_and_lost_acknowledgement_is_observed_without_replay() {
    for applied in [false, true] {
        let w = world(1);
        w.http(
            "PATCH pulls/101",
            vec![Response::Failure("lost acknowledgement".into())],
        );
        let original = w.0.lock().unwrap().prs[&101].clone();
        let mut after = original.clone();
        if applied {
            after["base"]["ref"] = json!("main");
        }
        w.http(
            "GET pulls/101",
            vec![
                Response::Output(200, original.to_string()),
                Response::Output(200, after.to_string()),
            ],
        );
        with_github(&w, |github| {
            let expected = serde_json::from_value(original).unwrap();
            assert_eq!(github.retarget_recovery(&expected, "main").is_ok(), applied);
        });
        assert_eq!(
            w.trace().iter().filter(|e| e["method"] == "PATCH").count(),
            1
        );
    }
}

#[test]
fn repaired_base_survives_failed_publication_and_retry_preserves_identity() {
    let w = world(1);
    let push = format!(
        "git push --porcelain --no-follow-tags --force-with-lease=refs/heads/topic/1:{} -- origin {}:refs/heads/topic/1",
        oid(11),
        oid(1)
    );
    w.fail(&push);
    let error = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
    assert!(error.contains("failed to push"), "{error}");
    assert_eq!(w.0.lock().unwrap().prs[&101]["base"]["ref"], "main");
    w.0.lock().unwrap().responses.remove(&push);
    w.run(&["--no-stack"]).unwrap();
    assert_eq!(w.0.lock().unwrap().prs.len(), 2);
}

#[test]
fn recovery_observation_policy_rejects_incomplete_and_contradictory_identities() {
    let original: RecoveryPullRequest =
        serde_json::from_value(pr(101, "topic/1", 11, "obsolete")).unwrap();
    for case in [
        "number",
        "node",
        "state",
        "head-ref",
        "head-sha",
        "base-ref",
        "base-sha",
        "repository",
    ] {
        let mut value = original.clone();
        match case {
            "number" => value.number = 0,
            "node" => value.node_id.clear(),
            "state" => value.state = "unknown".into(),
            "head-ref" => value.head.name.clear(),
            "head-sha" => value.head.sha = "bad".into(),
            "base-ref" => value.base.name.clear(),
            "base-sha" => value.base.sha = "z".repeat(40),
            "repository" => value.base.repo.full_name = "other/repo".into(),
            _ => unreachable!(),
        }
        assert!(policy::validate_recovery_observation(&value, 101, "org/repo").is_err());
    }
    assert!(policy::validate_recovery_observation(&original, 0, "org/repo").is_err());
    assert!(crate::git::policy::valid_oid(&"a".repeat(64)));
    for case in [
        "closed",
        "merged",
        "merged-at",
        "number",
        "node",
        "head",
        "base",
        "stack",
    ] {
        let mut value = original.clone();
        match case {
            "closed" => value.state = "closed".into(),
            "merged" => value.merged = true,
            "merged-at" => value.merged_at = Some("date".into()),
            "number" => value.number = 102,
            "node" => value.node_id = "OTHER".into(),
            "head" => value.head.sha = oid(12),
            "base" => value.base.name = "main".into(),
            "stack" => value.stack = Some(crate::model::pull_request::NativeStack { number: 8 }),
            _ => unreachable!(),
        }
        assert!(policy::same_recovery_head(&original, &value, "obsolete").is_err());
    }
}

#[test]
fn recovery_destination_and_ref_observations_reject_ambiguity_and_malformed_values() {
    let w = world(1);
    with_github(&w, |github| {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        for response in [
            "",
            "https://github.com/org/repo.git\nhttps://github.com/org/repo.git",
            "https://other.test/org/repo.git",
            "https://github.com/other/repo.git",
            "/local/path",
        ] {
            w.reply("git remote get-url --push --all -- origin", 0, response);
            assert!(git.recovery_destination().is_err(), "{response}");
        }
        for response in [
            "garbled".into(),
            "bad\trefs/heads/topic/1".into(),
            format!(
                "{}\trefs/heads/topic/1\n{}\trefs/heads/topic/1",
                oid(11),
                oid(11)
            ),
        ] {
            w.reply(
                "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/topic/1",
                0,
                &response,
            );
            assert!(
                git.recovery_refs("https://github.com/org/repo.git", &["topic/1".into()])
                    .is_err()
            );
        }
        assert!(
            git.push_with_leases(
                &[&crate::tests::unit::support::commit(1)],
                Some(&BTreeMap::new())
            )
            .is_err()
        );
    });
    assert!(!w.trace().iter().any(effect));
}

fn state(w: &Arc<World>) -> State {
    State::discover(&mut w.config(), &w.commands()).unwrap()
}

fn observe(w: &Arc<World>, github: &GitHub<'_>, state: &State) -> (Recovery, Vec<String>) {
    let refs: Vec<_> = state
        .stack
        .iter()
        .map(|commit| commit.remote_ref().to_owned())
        .collect();
    github.observe(&refs).unwrap();
    let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
    let recovery = Recovery::observe(github, &state.stack, 0, &bases).unwrap();
    w.0.lock().unwrap().trace.clear();
    (recovery, bases)
}

#[test]
fn recovery_revalidates_selection_refs_parent_evidence_and_identity_before_any_effect() {
    for case in [
        "selection",
        "refs",
        "parent",
        "destination",
        "new-pr",
        "removed-pr",
        "node",
        "dependant",
    ] {
        let w = world(2);
        if case == "new-pr" {
            w.0.lock().unwrap().prs.remove(&102);
        }
        let mut state = state(&w);
        with_github(&w, |github| {
            let (recovery, bases) = observe(&w, github, &state);
            match case {
                "selection" => {
                    state.stack.remove(0);
                }
                "refs" => {
                    w.0.lock()
                        .unwrap()
                        .remote_refs
                        .remove("refs/heads/obsolete");
                }
                "parent" => {
                    w.0.lock().unwrap().prs.get_mut(&90).unwrap()["merge_commit_sha"] =
                        json!(oid(20));
                }
                "destination" => w.reply(
                    "git remote get-url --push --all -- origin",
                    0,
                    "git@github.com:org/repo.git",
                ),
                "new-pr" => {
                    w.0.lock()
                        .unwrap()
                        .prs
                        .insert(102, pr(102, "topic/2", 12, "topic/1"));
                }
                "removed-pr" => {
                    w.0.lock().unwrap().prs.remove(&102);
                }
                "node" => {
                    w.0.lock().unwrap().prs.get_mut(&102).unwrap()["node_id"] = json!("OTHER");
                }
                "dependant" => {
                    w.0.lock()
                        .unwrap()
                        .prs
                        .insert(200, pr(200, "other", 12, "topic/2"));
                }
                _ => unreachable!(),
            }
            assert!(recovery.apply(github, &state, &bases).is_err(), "{case}");
            assert!(!w.trace().iter().any(effect), "{case}");
        });
    }
}

#[test]
fn refs_changed_during_retarget_stop_publication_without_rolling_back_the_base() {
    let w = world(1);
    let state = state(&w);
    with_github(&w, |github| {
        let (recovery, bases) = observe(&w, github, &state);
        let refs =
            w.0.lock()
                .unwrap()
                .remote_refs
                .iter()
                .map(|(reference, oid)| format!("{oid}\t{reference}"))
                .collect::<Vec<_>>()
                .join("\n");
        w.0.lock().unwrap().responses.insert(
            "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/main refs/heads/obsolete refs/heads/topic/1".into(),
            [Response::Output(0, refs.clone()), Response::Output(0, refs.replace(&oid(11), &oid(12)))].into(),
        );
        let error = format!("{:#}", recovery.apply(github, &state, &bases).unwrap_err());
        assert!(error.contains("journal retained"), "{error}");
        assert_eq!(w.0.lock().unwrap().prs[&101]["base"]["ref"], "main");
        assert!(!w.trace().iter().any(|e| e["args"][0] == "push"));
    });
}

#[test]
fn recovery_stages_an_unsafe_partial_push_state_for_an_upper_pr() {
    let w = world(2);
    w.reply(
        &format!("git merge-base --is-ancestor {} {}", oid(12), oid(1)),
        0,
        "",
    );
    w.run(&["--no-stack"]).unwrap();
    assert!(
        w.trace()
            .iter()
            .any(|event| event["body"]["base"] == "git-pr/transition/101")
    );
    assert_eq!(w.0.lock().unwrap().prs[&102]["state"], "open");
}

#[test]
fn failed_recovery_reads_stop_before_rewriting_or_mutation() {
    for command in [
        "git remote get-url --push --all -- origin".into(),
        "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/main refs/heads/obsolete refs/heads/topic/1".into(),
        format!("git merge-base --is-ancestor {} {}", oid(10), oid(11)),
        format!("git merge-base --is-ancestor {} {}", oid(0), oid(1)),
        format!("git merge-base --is-ancestor {} {}", oid(11), oid(0)),
    ] {
        let w = world(1);
        w.fail(&command);
        assert!(w.run(&["--no-stack"]).is_err(), "{command}");
        assert!(!w.trace().iter().any(effect));
    }
    let w = world(1);
    let mut closed = w.0.lock().unwrap().prs[&101].clone();
    closed["state"] = json!("closed");
    w.http(
        "GET pulls/101",
        vec![Response::Output(200, closed.to_string())],
    );
    assert!(w.run(&["--no-stack"]).is_err());
    assert!(!w.trace().iter().any(effect));
}

#[test]
fn recovery_requires_complete_observations_and_a_trunk_transition() {
    let w = world(1);
    let state = state(&w);
    with_github(&w, |github| {
        assert!(Recovery::observe(github, &state.stack, 0, &["main".into()]).is_err());
        github.observe(&["topic/1".into()]).unwrap();
        assert!(Recovery::observe(github, &state.stack, 0, &["other".into()]).is_err());
        let (mut recovery, _) = observe(&w, github, &state);
        recovery.bases = vec!["absent".into()];
        assert!(
            recovery
                .check_final_graph(
                    Git {
                        commands: github.commands,
                        repo: &github.config.repo
                    },
                    &state.stack
                )
                .is_err()
        );
        w.http(
            "POST graphql",
            vec![Response::Failure("observation unavailable".into())],
        );
        assert!(recovery.apply(github, &state, &["main".into()]).is_err());
        assert!(github.recovery_parent("obsolete").is_err());
        assert!(!w.trace().iter().any(effect));
    });
}

#[test]
fn recovery_adapter_rechecks_native_membership_and_read_availability() {
    for native in [false, true] {
        let w = world(1);
        if native {
            w.0.lock().unwrap().prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
        } else {
            w.http(
                "GET pulls/101",
                vec![Response::Failure("unavailable".into())],
            );
        }
        let expected = serde_json::from_value(w.0.lock().unwrap().prs[&101].clone()).unwrap();
        with_github(&w, |github| {
            assert!(github.retarget_recovery(&expected, "main").is_err());
        });
        assert!(!w.trace().iter().any(effect));
    }
}

#[derive(Debug)]
struct AfterRetarget {
    world: Arc<World>,
    failure: &'static str,
}

impl crate::github::http::Transport for AfterRetarget {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let patch = request.method == "PATCH";
        let result = crate::github::http::Transport::request(self.world.as_ref(), request)?;
        if patch {
            match self.failure {
                "destination" => self.world.fail("git remote get-url --push --all -- origin"),
                "refs" => self.world.fail("git ls-remote --refs -- https://github.com/org/repo.git refs/heads/main refs/heads/obsolete refs/heads/topic/1"),
                "dependants" => { self.world.0.lock().unwrap().prs.insert(200, pr(200, "outside", 12, "topic/1")); },
                "head" => {
                    let current = self.world.0.lock().unwrap().prs[&101].clone();
                    self.world.http("GET pulls/101", vec![Response::Output(200, current.to_string()), Response::Failure("read unavailable".into())]);
                },
                _ => unreachable!(),
            }
        }
        Ok(result)
    }
}

#[test]
fn recovery_retains_repaired_bases_when_final_verification_is_unavailable() {
    for failure in ["destination", "refs", "dependants", "head"] {
        let w = world(1);
        let state = state(&w);
        let transport = AfterRetarget {
            world: w.clone(),
            failure,
        };
        with_github(&w, |original| {
            let github = GitHub {
                session: crate::github::Session::default(),
                transport: &transport,
                ..*original
            };
            let (recovery, bases) = observe(&w, &github, &state);
            assert!(
                recovery.apply(&github, &state, &bases).is_err(),
                "{failure}"
            );
        });
        assert_eq!(w.0.lock().unwrap().prs[&101]["base"]["ref"], "main");
        assert!(!w.trace().iter().any(|event| event["args"][0] == "push"));
    }
}

fn transition_world(order: &[u64], published: &[u64]) -> Arc<World> {
    let w = World::new(0);
    let mut parents = BTreeMap::from([(0, vec![])]);
    {
        let mut m = w.0.lock().unwrap();
        m.remote_refs.insert("refs/heads/main".into(), oid(0));
        let mut previous = 0;
        for number in published {
            let hash = 10 + number;
            parents.insert(hash, vec![previous]);
            let base = if previous == 0 {
                "main".into()
            } else {
                format!("topic/{}", previous - 10)
            };
            m.prs.insert(
                100 + number,
                pr(100 + number, &format!("topic/{number}"), hash, &base),
            );
            m.remote_refs
                .insert(format!("refs/heads/topic/{number}"), oid(hash));
            previous = hash;
        }
        previous = 0;
        for number in order {
            let hash = 20 + number;
            parents.insert(hash, vec![previous]);
            let mut commit = crate::tests::unit::support::commit(*number);
            commit.hash = oid(hash);
            m.commits.push(commit);
            previous = hash;
        }
        m.branch = format!("topic/{}", order.last().unwrap());
    }
    for ancestor in parents.keys() {
        for descendant in parents.keys() {
            let mut pending = vec![*descendant];
            let mut seen = BTreeSet::new();
            while let Some(next) = pending.pop() {
                if seen.insert(next) {
                    pending.extend(&parents[&next]);
                }
            }
            w.reply(
                &format!(
                    "git merge-base --is-ancestor {} {}",
                    oid(*ancestor),
                    oid(*descendant)
                ),
                i32::from(!seen.contains(ancestor)),
                "",
            );
        }
    }
    w
}

#[test]
fn insertions_at_every_position_and_permutations_preserve_existing_pr_identities() {
    for order in [
        vec![4, 1, 2, 3],
        vec![1, 4, 2, 3],
        vec![1, 2, 4, 3],
        vec![1, 2, 3, 4],
        vec![2, 1, 3],
        vec![3, 2, 1],
        vec![2, 3, 1],
    ] {
        for native in [false, true] {
            for backend in ["git", "branchless", "jj"] {
                let w = transition_world(&order, &[1, 2, 3]);
                {
                    let mut m = w.0.lock().unwrap();
                    m.branchless = backend == "branchless";
                    m.jj = backend == "jj";
                    if native {
                        for pr in m.prs.values_mut() {
                            pr["stack"] = json!({"number":9});
                        }
                    }
                }
                w.run(&[if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                }])
                .unwrap();
                let m = w.0.lock().unwrap();
                for number in [1, 2, 3] {
                    assert_eq!(
                        m.prs[&(100 + number)]["node_id"],
                        format!("PR_{}", 100 + number)
                    );
                    assert_eq!(m.prs[&(100 + number)]["state"], "open");
                }
                for (index, number) in order.iter().enumerate() {
                    let pr = m
                        .prs
                        .values()
                        .find(|pr| pr["head"]["ref"] == format!("topic/{number}"))
                        .unwrap();
                    assert_eq!(
                        pr["base"]["ref"],
                        if index == 0 {
                            "main".into()
                        } else {
                            format!("topic/{}", order[index - 1])
                        }
                    );
                    assert_eq!(pr["stack"].is_object(), native);
                }
                assert!(!m.config.contains_key(JOURNAL));
                assert!(
                    !m.remote_refs
                        .keys()
                        .any(|reference| reference.starts_with("refs/heads/git-pr/transition/"))
                );
                drop(m);
            }
        }
    }
}

#[test]
fn splitting_published_history_uses_a_direct_transition() {
    let w = transition_world(&[1, 2, 3], &[1, 2, 3]);
    {
        let mut m = w.0.lock().unwrap();
        m.prs.remove(&101);
        m.prs.get_mut(&102).unwrap()["base"]["ref"] = json!("main");
        for (number, commit) in (1..=3).zip(&mut m.commits) {
            commit.hash = oid(10 + number);
        }
    }
    w.run(&["--no-stack"]).unwrap();
    assert!(!w.trace().iter().any(|e| {
        e["body"]["base"]
            .as_str()
            .is_some_and(|base| base.starts_with("git-pr/transition/"))
    }));
    assert_eq!(w.0.lock().unwrap().prs[&102]["base"]["ref"], "topic/1");
}

#[derive(Debug)]
struct Interruption {
    world: Arc<World>,
    remaining: std::sync::atomic::AtomicUsize,
    halted: std::sync::atomic::AtomicBool,
    all_operations: bool,
}

fn command_effect(args: &[String]) -> bool {
    args.first().is_some_and(|arg| arg == "push")
        || args.starts_with(&["config".into(), "--local".into()])
}

impl Interruption {
    fn check(&self) -> anyhow::Result<()> {
        if self.halted.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("simulated process interruption");
        }
        Ok(())
    }
    fn applied(&self, effect: bool) -> anyhow::Result<()> {
        if effect
            && self
                .remaining
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
                == 1
        {
            self.halted.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.check()
    }
}

impl crate::commands::CommandRunner for Interruption {
    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    ) -> anyhow::Result<crate::commands::CommandOutput> {
        self.check()?;
        if self.all_operations {
            self.applied(true)?;
        }
        let result = self.world.execute(program, args, cwd, env)?;
        self.applied(!self.all_operations && command_effect(args))?;
        Ok(result)
    }
}

impl crate::github::http::Transport for Interruption {
    fn request(&self, request: crate::github::http::Request<'_>) -> anyhow::Result<Vec<u8>> {
        self.check()?;
        if self.all_operations {
            self.applied(true)?;
        }
        let mutation = matches!(request.method, "POST" | "PATCH" | "DELETE")
            && !request.url.ends_with("/graphql");
        let result = self.world.request(request)?;
        self.applied(!self.all_operations && mutation)?;
        Ok(result)
    }
}

#[test]
fn transitions_resume_after_interruption_at_every_mutation_boundary() {
    for native in [false, true] {
        for order in [&[2, 1, 3][..], &[1, 4, 2, 3][..]] {
            let flags = [if native {
                "--github-stack"
            } else {
                "--no-stack"
            }];
            let baseline = transition_world(order, &[1, 2, 3]);
            if native {
                for pr in baseline.0.lock().unwrap().prs.values_mut() {
                    pr["stack"] = json!({"number":9});
                }
            }
            baseline.run(&flags).unwrap();
            let count = baseline
                .trace()
                .iter()
                .filter(|event| {
                    event["args"].as_array().is_some_and(|args| {
                        command_effect(
                            &args
                                .iter()
                                .map(|arg| arg.as_str().unwrap().to_owned())
                                .collect::<Vec<_>>(),
                        )
                    }) || matches!(event["method"].as_str(), Some("POST" | "PATCH" | "DELETE"))
                        && event["path"] != "graphql"
                })
                .count();
            for boundary in 1..=count {
                let w = transition_world(order, &[1, 2, 3]);
                if native {
                    for pr in w.0.lock().unwrap().prs.values_mut() {
                        pr["stack"] = json!({"number":9});
                    }
                }
                let interruption = Arc::new(Interruption {
                    world: w.clone(),
                    remaining: boundary.into(),
                    halted: false.into(),
                    all_operations: false,
                });
                let mut app = w.app();
                app.commands.runner = interruption.clone();
                app.transport = interruption;
                let parsed =
                    crate::cli::Flags::parse(flags.iter().map(|flag| (*flag).into()).collect())
                        .unwrap();
                assert!(
                    app.execute(
                        &parsed,
                        &BTreeMap::from([("HOME".into(), "/unit/home".into())])
                    )
                    .is_err()
                );
                // A newly created PR carries the remote objects in a real observation.
                {
                    let mut m = w.0.lock().unwrap();
                    let refs = m.remote_refs.clone();
                    for pr in m.prs.values_mut() {
                        if pr["head"]["sha"].is_null() {
                            pr["head"]["sha"] = json!(
                                refs[&format!(
                                    "refs/heads/{}",
                                    pr["head"]["ref"].as_str().unwrap()
                                )]
                            );
                            pr["base"]["sha"] = json!(oid(0));
                            pr["merged_at"] = Value::Null;
                            pr["merge_commit_sha"] = Value::Null;
                            if pr["stack"].is_null() {
                                pr["stack"] = Value::Null;
                            }
                        }
                    }
                }
                resolve_undispatched_creation(&w, &flags);
                w.run(&flags).unwrap_or_else(|error| {
                    panic!("native={native}, order={order:?}, boundary={boundary}: {error:#}")
                });
                let m = w.0.lock().unwrap();
                assert!(!m.config.contains_key(JOURNAL));
                assert!(
                    !m.remote_refs
                        .keys()
                        .any(|reference| reference.starts_with("refs/heads/git-pr/transition/"))
                );
                assert!(m.prs.values().all(|pr| pr["state"] == "open"));
                drop(m);
            }
        }
    }
}

#[test]
fn native_transitions_require_complete_membership_and_confirm_dissolution() {
    for case in [
        "partial-selection",
        "multiple",
        "partial-unstack",
        "unconfirmed-unstack",
        "unavailable-topology",
    ] {
        let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
        for pr in w.0.lock().unwrap().prs.values_mut() {
            pr["stack"] = json!({"number":9});
        }
        match case {
            "partial-selection" => {
                w.0.lock().unwrap().prs.insert(104, {
                    let mut extra = pr(104, "other", 13, "main");
                    extra["stack"] = json!({"number":9});
                    extra
                });
            }
            "multiple" => {
                w.0.lock().unwrap().prs.get_mut(&103).unwrap()["stack"] = json!({"number":10});
            }
            "partial-unstack" => w.http(
                "POST stacks/9/unstack",
                vec![Response::Output(200, "{}".into())],
            ),
            "unconfirmed-unstack" => w.http(
                "POST stacks/9/unstack",
                vec![Response::Output(200, String::new())],
            ),
            "unavailable-topology" => w.http(
                "GET stacks/9",
                vec![Response::Failure("unavailable".into())],
            ),
            _ => unreachable!(),
        }
        assert!(w.run(&["--no-stack"]).is_err(), "{case}");
        assert!(
            !w.trace()
                .iter()
                .any(|e| e["args"][0] == "push" || e["method"] == "PATCH"),
            "{case}"
        );
    }
    let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
    for pr in w.0.lock().unwrap().prs.values_mut() {
        pr["stack"] = json!({"number":9});
    }
    w.run(&["--no-stack"]).unwrap();
    assert!(
        w.0.lock()
            .unwrap()
            .prs
            .values()
            .all(|pr| pr["stack"].is_object())
    );
}

#[test]
fn invalid_journals_and_changed_remote_state_cannot_authorise_recovery() {
    for case in [
        "invalid",
        "missing-reference",
        "selection",
        "repository",
        "commit",
        "ancestry",
        "ref",
        "base",
        "identity",
        "head",
        "new-pr",
        "new-pr-read",
        "missing-head",
    ] {
        let w = transition_world(&[2, 4, 1, 3], &[1, 2, 3]);
        with_github(&w, |github| {
            let state = state(&w);
            let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
            github
                .observe(
                    &state
                        .stack
                        .iter()
                        .map(|c| c.remote_ref().to_owned())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let plan = Recovery::observe(github, &state.stack, 0, &bases).unwrap();
            plan.apply(github, &state, &bases).unwrap();
        });
        {
            let mut m = w.0.lock().unwrap();
            match case {
                "missing-reference" => {
                    let mut saved: Value = serde_json::from_str(&m.config[JOURNAL]).unwrap();
                    saved["refs"].as_object_mut().unwrap().remove("topic/1");
                    m.config.insert(JOURNAL.into(), saved.to_string());
                }
                "invalid" => {
                    m.config.insert(JOURNAL.into(), "invalid JSON".into());
                }
                "selection" => {
                    m.commits.remove(0);
                }
                "repository" => {
                    let mut saved: Value = serde_json::from_str(&m.config[JOURNAL]).unwrap();
                    saved["repository"] = json!("other/repo");
                    m.config.insert(JOURNAL.into(), saved.to_string());
                }
                "commit" => m.commits[0].hash = oid(99),
                "ref" => {
                    m.remote_refs.insert("refs/heads/topic/1".into(), oid(99));
                }
                "base" => m.prs.get_mut(&101).unwrap()["base"]["ref"] = json!("human-base"),
                "identity" => m.prs.get_mut(&101).unwrap()["node_id"] = json!("OTHER"),
                "head" => m.prs.get_mut(&101).unwrap()["head"]["sha"] = json!(oid(99)),
                "new-pr" | "new-pr-read" => {
                    m.prs.insert(104, pr(104, "topic/4", 99, "main"));
                }
                "missing-head" => {
                    let mut saved: Value = serde_json::from_str(&m.config[JOURNAL]).unwrap();
                    saved["refs"]["topic/1"] = Value::Null;
                    m.config.insert(JOURNAL.into(), saved.to_string());
                    m.remote_refs.remove("refs/heads/topic/1");
                }
                "ancestry" => {}
                _ => unreachable!(),
            }
            m.trace.clear();
        }
        if case == "new-pr-read" {
            w.http(
                "GET pulls/104",
                vec![Response::Failure("unavailable new PR".into())],
            );
        }
        if case == "ancestry" {
            w.reply(
                &format!("git merge-base --is-ancestor {} {}", oid(22), oid(24)),
                1,
                "",
            );
        }
        assert!(w.run(&["--no-stack"]).is_err(), "{case}");
        assert!(!w.trace().iter().any(effect), "{case}");
    }
}

#[test]
fn transition_preparation_rejects_collisions_and_changing_evidence() {
    for case in [
        "collision",
        "staging-read",
        "observation",
        "ancestry",
        "final-ancestry",
        "staging-ancestry",
        "staging-during-prepare",
        "native-change",
    ] {
        let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
        if case == "native-change" {
            for pr in w.0.lock().unwrap().prs.values_mut() {
                pr["stack"] = json!({"number":9});
            }
        }
        if case == "collision" {
            w.0.lock()
                .unwrap()
                .remote_refs
                .insert("refs/heads/git-pr/transition/101".into(), oid(0));
        }
        if case == "staging-read" {
            w.fail("git ls-remote --refs -- https://github.com/org/repo.git refs/heads/git-pr/transition/101");
        }
        if case == "observation" {
            let mut changed = w.0.lock().unwrap().prs[&102].clone();
            changed["node_id"] = json!("OTHER");
            w.http(
                "GET pulls/102",
                vec![Response::Output(200, changed.to_string())],
            );
        }
        with_github(&w, |github| {
            let state = state(&w);
            let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
            github
                .observe(
                    &state
                        .stack
                        .iter()
                        .map(|c| c.remote_ref().to_owned())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let result = Recovery::observe(github, &state.stack, 0, &bases);
            if ["collision", "staging-read", "observation"].contains(&case) {
                assert!(result.is_err(), "{case}");
                return;
            }
            let mut plan = result.unwrap();
            w.0.lock().unwrap().trace.clear();
            match case {
                "ancestry" => {
                    plan.staging = None;
                    w.fail(&format!(
                        "git merge-base --is-ancestor {} {}",
                        oid(12),
                        oid(0)
                    ));
                }
                "final-ancestry" => {
                    w.reply(
                        &format!("git merge-base --is-ancestor {} {}", oid(22), oid(21)),
                        1,
                        "",
                    );
                }
                "staging-ancestry" => {
                    w.fail(&format!(
                        "git merge-base --is-ancestor {} {}",
                        oid(12),
                        oid(0)
                    ));
                }
                "staging-during-prepare" => {
                    w.reply(
                        &format!("git merge-base --is-ancestor {} {}", oid(12), oid(0)),
                        0,
                        "",
                    );
                }
                "native-change" => {
                    for pr in w.0.lock().unwrap().prs.values_mut() {
                        pr["stack"] = json!({"number":9});
                    }
                    w.http(
                        "GET stacks/9",
                        vec![Response::Failure("unavailable topology".into())],
                    );
                }
                _ => unreachable!(),
            }
            assert!(plan.apply(github, &state, &bases).is_err(), "{case}");
            assert!(!w.trace().iter().any(effect), "{case}");
        });
    }
}

#[test]
fn unavailable_observations_stop_transitions_at_every_external_boundary() {
    for native in [false, true] {
        let baseline = transition_world(&[2, 1, 3], &[1, 2, 3]);
        if native {
            for pr in baseline.0.lock().unwrap().prs.values_mut() {
                pr["stack"] = json!({"number":9});
            }
        }
        let flags = [if native {
            "--github-stack"
        } else {
            "--no-stack"
        }];
        baseline.run(&flags).unwrap();
        let operations = baseline
            .trace()
            .iter()
            .filter(|event| matches!(event["kind"].as_str(), Some("command" | "http")))
            .count();
        for boundary in 1..=operations {
            let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
            if native {
                for pr in w.0.lock().unwrap().prs.values_mut() {
                    pr["stack"] = json!({"number":9});
                }
            }
            let interruption = Arc::new(Interruption {
                world: w.clone(),
                remaining: boundary.into(),
                halted: false.into(),
                all_operations: true,
            });
            let mut app = w.app();
            app.commands.runner = interruption.clone();
            app.transport = interruption;
            let parsed =
                crate::cli::Flags::parse(flags.iter().map(|flag| (*flag).into()).collect())
                    .unwrap();
            let result = app.execute(
                &parsed,
                &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
            );
            assert!(result.is_err(), "native={native}, boundary={boundary}");
            assert!(
                w.0.lock()
                    .unwrap()
                    .prs
                    .values()
                    .all(|pr| pr["state"] == "open"),
                "native={native}, boundary={boundary}"
            );
        }
    }
}

#[test]
fn transitions_without_existing_identity_or_stable_preparation_are_refused() {
    let w = transition_world(&[1, 2], &[]);
    with_github(&w, |github| {
        let state = state(&w);
        github
            .observe(&["topic/1".into(), "topic/2".into()])
            .unwrap();
        assert!(
            Recovery::observe(github, &state.stack, 0, &["main".into(), "topic/1".into()]).is_err()
        );
    });
    let w = world(2);
    with_github(&w, |github| {
        let state = state(&w);
        let (plan, bases) = observe(&w, github, &state);
        w.reply(
            &format!("git merge-base --is-ancestor {} {}", oid(12), oid(1)),
            0,
            "",
        );
        assert!(plan.apply(github, &state, &bases).is_err());
        assert!(!w.trace().iter().any(effect));
    });
}

#[test]
fn transition_barriers_preserve_the_journal_for_unavailable_or_changed_evidence() {
    for case in ["parent-read", "ambiguous", "unpublished", "final-identity"] {
        let w = world(1);
        with_github(&w, |github| {
            let state = state(&w);
            let (plan, bases) = observe(&w, github, &state);
            if case == "parent-read" {
                w.http(
                    "GET pulls/90",
                    vec![Response::Failure("unavailable".into())],
                );
            }
            if case == "ambiguous" {
                w.0.lock()
                    .unwrap()
                    .prs
                    .insert(102, pr(102, "topic/1", 11, "obsolete"));
            }
            if ["parent-read", "ambiguous"].contains(&case) {
                assert!(plan.apply(github, &state, &bases).is_err());
                assert!(!w.trace().iter().any(effect));
                return;
            }
            plan.apply(github, &state, &bases).unwrap();
            if case == "unpublished" {
                assert!(plan.complete(github, &state).is_err());
            } else {
                Git {
                    commands: github.commands,
                    repo: &github.config.repo,
                }
                .push_with_leases(&[&state.stack[0]], Some(&plan.refs))
                .unwrap();
                plan.complete(github, &state).unwrap();
                w.0.lock().unwrap().prs.get_mut(&101).unwrap()["node_id"] = json!("CHANGED");
                assert!(plan.finish(github, &state).is_err());
            }
            assert!(w.0.lock().unwrap().config.contains_key(JOURNAL));
        });
    }
}

#[test]
fn native_restoration_failure_retains_a_resumable_transition() {
    let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
    for pr in w.0.lock().unwrap().prs.values_mut() {
        pr["stack"] = json!({"number":9});
    }
    w.http(
        "POST stacks",
        vec![Response::Failure("native restoration unavailable".into())],
    );
    assert!(w.run(&["--no-stack"]).is_err());
    assert!(w.0.lock().unwrap().config.contains_key(JOURNAL));
    w.0.lock().unwrap().http_responses.clear();
    w.run(&["--no-stack"]).unwrap();
    assert!(!w.0.lock().unwrap().config.contains_key(JOURNAL));
}

#[test]
fn repeated_recovery_can_recreate_an_anchor_deleted_before_journal_cleanup() {
    let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
    with_github(&w, |github| {
        let state = state(&w);
        let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
        github
            .observe(&["topic/1".into(), "topic/2".into(), "topic/3".into()])
            .unwrap();
        let plan = Recovery::observe(github, &state.stack, 0, &bases).unwrap();
        plan.apply(github, &state, &bases).unwrap();
    });
    with_github(&w, |github| {
        let state = state(&w);
        let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
        let plan = Recovery::observe(github, &state.stack, 0, &bases).unwrap();
        plan.apply(github, &state, &bases).unwrap();
    });
    // An interrupted cleanup has removed its ref but retained the newer journal.
    w.0.lock()
        .unwrap()
        .remote_refs
        .remove("refs/heads/git-pr/transition/101");
    w.run(&["--no-stack"]).unwrap();
    assert!(!w.0.lock().unwrap().config.contains_key(JOURNAL));
}

fn resolve_undispatched_creation(w: &Arc<World>, flags: &[&str]) {
    let pending_without_review = {
        let memory = w.0.lock().unwrap();
        memory
            .config
            .get("git-pr.pendingCreate")
            .is_some_and(|value| {
                let head = value
                    .split(' ')
                    .nth(3)
                    .unwrap()
                    .trim_start_matches("refs/heads/");
                !memory.prs.values().any(|pr| pr["head"]["ref"] == head)
            })
    };
    if pending_without_review {
        assert!(format!("{:#}", w.run(flags).unwrap_err()).contains("unknown outcome"));
        // The prefix oracle knows that no creation request reached the service.
        w.0.lock().unwrap().config.remove("git-pr.pendingCreate");
    }
}
