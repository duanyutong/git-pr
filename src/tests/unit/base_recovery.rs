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
            error.contains("no local rewrites, pushes or PR changes"),
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
        assert!(error.contains("repaired bases are retained"), "{error}");
        assert_eq!(w.0.lock().unwrap().prs[&101]["base"]["ref"], "main");
        assert!(!w.trace().iter().any(|e| e["args"][0] == "push"));
    });
}

#[test]
fn recovery_rejects_an_unsafe_partial_push_state_for_an_upper_pr() {
    let w = world(2);
    w.reply(
        &format!("git merge-base --is-ancestor {} {}", oid(12), oid(1)),
        0,
        "",
    );
    let error = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
    assert!(error.contains("indirect merge"), "{error}");
    assert!(!w.trace().iter().any(effect));
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
        let (recovery, _) = observe(&w, github, &state);
        assert!(
            recovery
                .check_graph(
                    Git {
                        commands: github.commands,
                        repo: &github.config.repo
                    },
                    &state.stack,
                    &["absent".into()]
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
