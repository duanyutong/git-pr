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
    mock_ancestry(&w, &parents);
    w
}

fn mock_ancestry(w: &World, parents: &BTreeMap<u64, Vec<u64>>) {
    w.0.lock().unwrap().parents = parents
        .iter()
        .map(|(node, parents)| {
            (
                oid(*node),
                parents.iter().map(|parent| oid(*parent)).collect(),
            )
        })
        .collect();
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
}

fn is_retarget(event: &Value) -> bool {
    event["body"]["variables"]["input"]
        .get("baseRefName")
        .is_some()
}

fn effect(event: &Value) -> bool {
    event["args"][0] == "push"
        || event["method"] == "PATCH"
        || is_retarget(event)
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
fn native_dissolution_checks_auto_merge_even_with_linking_disabled() {
    let world = transition_world(&[2, 1], &[1, 2]);
    {
        let mut memory = world.0.lock().unwrap();
        for pr in memory.prs.values_mut() {
            pr["stack"] = json!({"number":9});
        }
        memory.prs.get_mut(&101).unwrap()["auto_merge"] = json!({"merge_method":"squash"});
    }
    let error = world.run(&["--no-stack"]).unwrap_err();
    assert!(format!("{error:#}").contains("auto-merge enabled on PR(s) #101"));
    assert!(!world.trace().iter().any(effect));
}

#[test]
fn native_dissolution_rechecks_eligibility_before_mutation() {
    let world = transition_world(&[2, 1], &[1, 2]);
    for pr in world.0.lock().unwrap().prs.values_mut() {
        pr["stack"] = json!({"number":9});
    }
    let state = state(&world);
    with_github(&world, |github| {
        let (recovery, bases) = observe(&world, github, &state);
        world.0.lock().unwrap().prs.get_mut(&101).unwrap()["auto_merge"] =
            json!({"merge_method":"squash"});
        world.0.lock().unwrap().trace.clear();
        let error = recovery.apply(github, &state, &bases).unwrap_err();
        assert!(format!("{error:#}").contains("auto-merge enabled"));
        assert!(!world.trace().iter().any(effect));
    });
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
                    .position(|e| {
                        is_retarget(e) && e["body"]["variables"]["input"]["baseRefName"] == "main"
                    })
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
                assert!(!w.trace().iter().any(is_retarget));
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
        "missing-snapshot",
        "missing-head",
        "changed-head",
        "native",
        "malformed-membership",
        "closed",
        "already-landed",
        "prepared-landed",
        "malformed-snapshot",
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
                "missing-head" => {
                    m.remote_refs.remove("refs/heads/topic/1");
                }
                "changed-head" => {
                    m.remote_refs.insert("refs/heads/topic/1".into(), oid(12));
                }
                "native" => m.prs.get_mut(&101).unwrap()["stack"] = json!({"number":9}),
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
            "malformed-membership" => {
                let mut rest = w.0.lock().unwrap().prs[&101].clone();
                rest["stack"] = json!("unknown");
                w.http("POST recovery", vec![recovery_reply(&[rest])]);
            }
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
            "missing-snapshot" => {
                w.fail("git rev-parse --verify refs/remotes/origin/main^{commit}");
            }
            "malformed-snapshot" => {
                w.reply(
                    "git rev-parse --verify refs/remotes/origin/main^{commit}",
                    0,
                    "bad",
                );
            }
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

#[derive(Debug)]
struct ChangeIdentity {
    world: Arc<World>,
    remaining: std::sync::atomic::AtomicUsize,
}

impl crate::github::http::Transport for ChangeIdentity {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let payload: Value = request
            .body
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        let recovery = (payload["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("query RecoveryHeads"))
            && payload["variables"]["h0"] == "topic/1")
            || payload["variables"]["r0"] == 101
            || payload["variables"]["input"]["pullRequestId"] == "PR_101"
            || (request.method == "GET" && request.url.ends_with("/pulls/101"));
        if recovery
            && self
                .remaining
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
                == 1
        {
            self.world.0.lock().unwrap().prs.get_mut(&101).unwrap()["node_id"] = json!("CHANGED");
        }
        crate::github::http::Transport::request(self.world.as_ref(), request)
    }
}

#[test]
fn recovery_revalidates_metadata_before_mutation_and_after_publication() {
    for call in 2..=7 {
        let w = world(1);
        let mut app = w.app();
        app.transport = Arc::new(ChangeIdentity {
            world: w.clone(),
            remaining: call.into(),
        });
        let flags = crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap();
        let error = format!(
            "{:#}",
            app.execute(
                &flags,
                &BTreeMap::from([("HOME".into(), "/unit/home".into())])
            )
            .unwrap_err()
        );
        assert!(error.contains("changed"), "{call}: {error}");
        assert_eq!(
            w.trace().iter().any(|e| e["args"][0] == "push"),
            call >= 6,
            "{call}"
        );
        assert_eq!(w.trace().iter().any(is_retarget), call >= 4, "{call}");
    }
}

#[test]
fn unconfirmed_retarget_stops_and_lost_acknowledgement_is_observed_without_replay() {
    for applied in [false, true] {
        let w = world(1);
        w.http(
            "POST retarget",
            vec![Response::Failure("lost acknowledgement".into())],
        );
        let original = w.0.lock().unwrap().prs[&101].clone();
        let mut after = original.clone();
        if applied {
            after["base"]["ref"] = json!("main");
        }
        w.http(
            "POST recovery",
            vec![
                recovery_reply(std::slice::from_ref(&original)),
                recovery_reply(&[after]),
            ],
        );
        with_github(&w, |github| {
            let expected = serde_json::from_value(original).unwrap();
            assert_eq!(github.retarget_recovery(&expected, "main").is_ok(), applied);
        });
        assert_eq!(w.trace().iter().filter(|e| is_retarget(e)).count(), 1);
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
                Some(&BTreeMap::new()),
            )
            .is_err()
        );
    });
    assert!(!w.trace().iter().any(effect));
}

fn state(w: &Arc<World>) -> State {
    State::discover(&mut w.config(), &w.commands()).unwrap()
}

fn planned_transition(
    github: &GitHub<'_>,
    stack: &[Commit],
    start: usize,
    bases: &[String],
) -> Result<Transition> {
    let mut transition = Transition::observe(github, stack, start, bases)?;
    let git = Git {
        commands: github.commands,
        repo: &github.config.repo,
    };
    let ancestry = transition.ancestry(git, stack)?;
    transition.plan = Arc::new(PublicationPlan::derive(
        &transition.desired(stack, github.config.github_stack),
        &transition.observed(&git.repo.trunk),
        &ancestry,
    )?);
    transition.prepare_staging(git)?;
    Ok(transition)
}

fn observe(w: &Arc<World>, github: &GitHub<'_>, state: &State) -> (Transition, Vec<String>) {
    let refs: Vec<_> = state
        .stack
        .iter()
        .map(|commit| commit.remote_ref().to_owned())
        .collect();
    github.observe(&refs).unwrap();
    let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
    let recovery = planned_transition(github, &state.stack, 0, &bases).unwrap();
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
            "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/git-pr/transition/101 refs/heads/obsolete refs/heads/topic/1".into(),
            [Response::Output(0, refs.clone()), Response::Output(0, refs.replace(&oid(11), &oid(12)))].into(),
        );
        let error = format!("{:#}", recovery.apply(github, &state, &bases).unwrap_err());
        assert!(
            error.contains("retry from fresh remote observations"),
            "{error}"
        );
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
            .any(|event| event["body"]["variables"]["input"]["baseRefName"]
                == "git-pr/transition/101")
    );
    assert_eq!(w.0.lock().unwrap().prs[&102]["state"], "open");
}

#[test]
fn failed_recovery_reads_stop_before_rewriting_or_mutation() {
    for command in [
        "git remote get-url --push --all -- origin".into(),
        "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/git-pr/transition/101 refs/heads/obsolete refs/heads/topic/1".into(),
        format!("git merge-base --is-ancestor {} {}", oid(10), oid(11)),
        "git rev-parse --verify refs/remotes/origin/main^{commit}".into(),
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
    w.http("POST recovery", vec![recovery_reply(&[closed])]);
    assert!(w.run(&["--no-stack"]).is_err());
    assert!(!w.trace().iter().any(effect));
}

#[test]
fn recovery_requires_complete_observations_and_a_trunk_transition() {
    let w = world(1);
    let state = state(&w);
    with_github(&w, |github| {
        assert!(planned_transition(github, &state.stack, 0, &["main".into()]).is_err());
        github.observe(&["topic/1".into()]).unwrap();
        assert!(planned_transition(github, &state.stack, 0, &["other".into()]).is_err());
        let (mut recovery, _) = observe(&w, github, &state);
        recovery.bases = vec!["absent".into()];
        assert!(
            recovery
                .ancestry(
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
                "POST recovery",
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

#[test]
fn recovery_rejects_malformed_rest_and_missing_head_before_membership_lookup() {
    for body in ["{", r#"{"head":{}}"#] {
        let w = world(1);
        w.http("GET pulls/101", vec![Response::Output(200, body.into())]);
        with_github(&w, |github| {
            w.0.lock().unwrap().trace.clear();
            assert!(github.recovery_pr(101).is_err());
        });
        assert_eq!(w.trace().len(), 1);
        assert!(!w.trace().iter().any(effect));
    }
}

#[test]
fn recovery_resolves_omitted_rest_membership_from_fresh_graphql_evidence() {
    for native in [false, true] {
        let w = world(1);
        let mut rest = w.0.lock().unwrap().prs[&101].clone();
        rest.as_object_mut().unwrap().remove("stack");
        w.http(
            "GET pulls/101",
            vec![Response::Output(200, rest.to_string())],
        );
        with_github(&w, |github| {
            github.observe(&["topic/1".into()]).unwrap();
            if native {
                w.0.lock().unwrap().prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
            }
            let observed = github.recovery_pr(101).unwrap();
            assert_eq!(
                observed.stack.as_ref().map(|stack| stack.number),
                native.then_some(9)
            );
        });
        assert!(!w.trace().iter().any(effect));
    }
}

#[test]
fn recovery_refuses_unknown_or_inconsistent_fallback_membership_despite_cached_evidence() {
    for case in [
        "missing",
        "unsupported",
        "stack",
        "node",
        "base",
        "state",
        "merged",
        "unavailable",
    ] {
        let w = world(1);
        let mut rest = w.0.lock().unwrap().prs[&101].clone();
        rest.as_object_mut().unwrap().remove("stack");
        w.http(
            "GET pulls/101",
            vec![Response::Output(200, rest.to_string())],
        );
        with_github(&w, |github| {
            github.observe(&["topic/1".into()]).unwrap();
            let mut response = json!({"data":{"repository":{"p0":{
                "nodes":[{"id":"PR_101","number":101,"title":"Feature","body":"Human discussion",
                    "state":"OPEN","merged":false,"headRefName":"topic/1","baseRefName":"obsolete",
                    "headRepository":{"nameWithOwner":"org/repo"},"stack":null,
                    "labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}}],
                "pageInfo":{"hasNextPage":false}
            }}}});
            let node = &mut response["data"]["repository"]["p0"]["nodes"][0];
            match case {
                "missing" => response["data"]["repository"]["p0"]["nodes"] = json!([]),
                "stack" => {
                    node.as_object_mut().unwrap().remove("stack");
                }
                "node" => node["id"] = json!("OTHER"),
                "base" => node["baseRefName"] = json!("other"),
                "state" => node["state"] = json!("CLOSED"),
                "merged" => node["merged"] = json!(true),
                _ => {}
            }
            let reply = if case == "unavailable" {
                Response::Failure("membership unavailable".into())
            } else {
                Response::Output(200, response.to_string())
            };
            let mut replies = Vec::new();
            if case == "unsupported" {
                replies.push(Response::Output(
                    200,
                    json!({"errors":[{"message":"Cannot query field stack"}]}).to_string(),
                ));
            }
            replies.push(reply);
            w.http("POST graphql", replies);
            assert!(github.recovery_pr(101).is_err(), "{case}");
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
        let payload: Value = request
            .body
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        let patch = payload["variables"]["input"].get("baseRefName").is_some();
        let result = crate::github::http::Transport::request(self.world.as_ref(), request)?;
        if patch {
            match self.failure {
                "destination" => self.world.fail("git remote get-url --push --all -- origin"),
                "refs" => self.world.fail("git ls-remote --refs -- https://github.com/org/repo.git refs/heads/git-pr/transition/101 refs/heads/obsolete refs/heads/topic/1"),
                "dependants" => { self.world.0.lock().unwrap().prs.insert(200, pr(200, "outside", 12, "topic/1")); },
                "head" => {
                    self.world.http("POST recovery", vec![Response::Failure("read unavailable".into())]);
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
    mock_ancestry(&w, &parents);
    w
}

fn rewrite_unpublished_transition(w: &World, advance_trunk: bool) {
    let mut parents = BTreeMap::from([
        (0, vec![]),
        (11, vec![0]),
        (12, vec![11]),
        (13, vec![12]),
        (100, vec![0]),
    ]);
    {
        let mut memory = w.0.lock().unwrap();
        let mut previous = 0;
        for commit in &memory.commits {
            let hash = u64::from_str_radix(&commit.hash, 16).unwrap();
            parents.insert(hash, vec![previous]);
            previous = hash;
        }
        previous = if advance_trunk { 100 } else { 0 };
        memory
            .remote_refs
            .insert("refs/heads/main".into(), oid(previous));
        for commit in &mut memory.commits {
            let hash = u64::from_str_radix(&commit.hash, 16).unwrap() + 30;
            parents.insert(hash, vec![previous]);
            commit.hash = oid(hash);
            previous = hash;
        }
        memory.trace.clear();
    }
    mock_ancestry(w, &parents);
}

#[test]
fn rewritten_local_stacks_use_current_remote_membership() {
    for order in [&[1, 2, 3][..], &[1, 4, 2, 3][..]] {
        for native in [false, true] {
            for advanced in [false, true] {
                let w = transition_world(order, &[1, 2, 3]);
                if native {
                    for pr in w.0.lock().unwrap().prs.values_mut() {
                        pr["stack"] = json!({"number":9});
                    }
                }
                if advanced {
                    for pr in w.0.lock().unwrap().prs.values_mut() {
                        pr["stack"] = Value::Null;
                    }
                }
                rewrite_unpublished_transition(&w, advanced);
                w.run(&["--no-stack"]).unwrap();
                let memory = w.0.lock().unwrap();
                for commit in &memory.commits {
                    let pr = memory
                        .prs
                        .values()
                        .find(|pr| pr["head"]["ref"] == commit.remote_ref())
                        .unwrap();
                    assert_eq!(pr["state"], "open");
                    assert_eq!(
                        pr["stack"].is_object(),
                        native && !advanced && order == [1, 2, 3]
                    );
                    assert_eq!(
                        memory.remote_refs[&format!("refs/heads/{}", commit.remote_ref())],
                        commit.hash
                    );
                }
            }
        }
    }
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
fn retargeting_without_object_changes_needs_no_branch_publication() {
    for initial_native in [false, true] {
        for link_native in [false, true] {
            let world = transition_world(&[1, 2], &[1, 2]);
            {
                let mut memory = world.0.lock().unwrap();
                memory.commits[0].hash = oid(11);
                memory.commits[1].hash = oid(12);
                memory.prs.get_mut(&102).unwrap()["base"]["ref"] = json!("main");
                if initial_native {
                    for pr in memory.prs.values_mut() {
                        pr["stack"] = json!({"number":9});
                    }
                }
                memory.trace.clear();
            }
            let flags = [if link_native {
                "--github-stack"
            } else {
                "--no-stack"
            }];
            world.run(&flags).unwrap();
            assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
            {
                let memory = world.0.lock().unwrap();
                assert_eq!(memory.prs[&102]["base"]["ref"], "topic/1");
                assert_eq!(memory.prs[&101]["node_id"], "PR_101");
                assert_eq!(memory.prs[&102]["node_id"], "PR_102");
                assert!(
                    memory
                        .prs
                        .values()
                        .all(|pr| pr["stack"].is_object() == link_native)
                );
                drop(memory);
            }
            world.0.lock().unwrap().trace.clear();
            world.run(&flags).unwrap();
            assert!(!world.trace().iter().any(effect));
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
        e["body"]["variables"]["input"]["baseRefName"]
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

#[derive(Debug)]
struct MovingTrunk {
    world: Arc<World>,
    next: std::sync::atomic::AtomicU64,
}

impl crate::github::http::Transport for MovingTrunk {
    fn request(&self, request: crate::github::http::Request<'_>) -> anyhow::Result<Vec<u8>> {
        let response = self.world.request(request)?;
        // Deliberately absent from the local ancestry oracle: submission must
        // not fetch or resolve independently published trunk objects.
        let next = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.world
            .0
            .lock()
            .unwrap()
            .remote_refs
            .insert("refs/heads/main".into(), oid(next));
        Ok(response)
    }
}

#[test]
fn trunk_can_advance_before_and_throughout_every_publication_strategy() {
    for order in [&[1, 2, 3][..], &[2, 1, 3][..], &[1, 4, 2, 3][..]] {
        for native in [false, true] {
            let world = transition_world(order, &[1, 2, 3]);
            world
                .0
                .lock()
                .unwrap()
                .remote_refs
                .insert("refs/heads/main".into(), oid(100));
            let mut app = world.app();
            app.transport = Arc::new(MovingTrunk {
                world: world.clone(),
                next: std::sync::atomic::AtomicU64::new(101),
            });
            let flags = crate::cli::Flags::parse(vec![if native {
                "--github-stack".into()
            } else {
                "--no-stack".into()
            }])
            .unwrap();
            app.execute(
                &flags,
                &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
            )
            .unwrap();
            let memory = world.0.lock().unwrap();
            assert_ne!(memory.remote_refs["refs/heads/main"], oid(100));
            assert!(
                !memory
                    .remote_refs
                    .keys()
                    .any(|r| r.starts_with("refs/heads/git-pr/transition/"))
            );
            for (index, number) in order.iter().enumerate() {
                let head = format!("topic/{number}");
                let pr = memory
                    .prs
                    .values()
                    .find(|pr| pr["head"]["ref"] == head)
                    .unwrap();
                assert_eq!(pr["state"], "open");
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
            drop(memory);
            assert!(!world.trace().iter().any(|event| {
                event["args"][0] == "ls-remote"
                    && event["args"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("refs/heads/main"))
            }));
        }
    }
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
    fn edit_messages(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &std::path::Path,
        _env: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        _messages: &[(String, String)],
    ) -> anyhow::Result<crate::commands::CommandOutput> {
        anyhow::bail!("command adapter does not support bulk message editing")
    }

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
        let payload: Value = request
            .body
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        let mutation = (matches!(request.method, "POST" | "PATCH" | "DELETE")
            && !request.url.ends_with("/graphql"))
            || payload["variables"]["input"].get("baseRefName").is_some();
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
                        || is_retarget(event)
                })
                .count();
            for (boundary, rewritten) in (1..=count)
                .flat_map(|boundary| [false, true].map(|rewritten| (boundary, rewritten)))
            {
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
                let result = app.execute(
                    &parsed,
                    &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
                );
                if result.is_ok() {
                    assert!(
                        w.0.lock()
                            .unwrap()
                            .stderr
                            .contains("temporary-base cleanup can be retried independently"),
                        "boundary={boundary}"
                    );
                }
                hydrate_created_prs(&w);
                resolve_undispatched_creation(&w, &flags);
                if rewritten {
                    rewrite_unpublished_transition(&w, true);
                }
                w.run(&flags).unwrap_or_else(|error| {
                    panic!("native={native}, order={order:?}, boundary={boundary}, rewritten={rewritten}: {error:#}")
                });
                let m = w.0.lock().unwrap();
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
                .any(|e| e["args"][0] == "push" || is_retarget(e)),
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
            .all(|pr| pr["stack"].is_null())
    );
}

fn arrange_transition_failure(w: &World, case: &str) {
    if case == "native-change" {
        for pr in w.0.lock().unwrap().prs.values_mut() {
            pr["stack"] = json!({"number":9});
        }
    }
    if case == "collision" {
        w.0.lock()
            .unwrap()
            .remote_refs
            .insert("refs/heads/git-pr/transition/101".into(), oid(11));
    }
    if case == "staging-read" {
        w.fail("git ls-remote --refs -- https://github.com/org/repo.git refs/heads/git-pr/transition/101");
    }
    if case == "observation" {
        let mut values = {
            let memory = w.0.lock().unwrap();
            [102, 101, 103].map(|number| memory.prs[&number].clone())
        };
        values[0]["node_id"] = json!("OTHER");
        w.http("POST recovery", vec![recovery_reply(&values)]);
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
        arrange_transition_failure(&w, case);
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
            let result = planned_transition(github, &state.stack, 0, &bases);
            if ["collision", "staging-read", "observation"].contains(&case) {
                assert!(result.is_err(), "{case}");
                return;
            }
            let mut plan = result.unwrap();
            w.0.lock().unwrap().trace.clear();
            match case {
                "ancestry" => {
                    Arc::make_mut(&mut plan.plan).strategy = Strategy::Retarget;
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
    for invalid in ["membership", "identity"] {
        let w = world(1);
        let mut value = pr(101, "topic/1", 11, "main");
        if invalid == "membership" {
            value["stack"] = json!("unknown");
        } else {
            value["head"]["sha"] = json!("invalid");
        }
        w.http(
            "GET pulls/101",
            vec![Response::Output(200, value.to_string())],
        );
        with_github(&w, |github| {
            assert!(github.recovery_pr(101).is_err());
        });
        assert!(!w.trace().iter().any(effect));
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
            if result.is_ok() {
                assert!(
                    w.0.lock()
                        .unwrap()
                        .stderr
                        .contains("temporary-base cleanup can be retried independently"),
                    "native={native}, boundary={boundary}"
                );
            }
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
fn unpublished_stacks_need_no_transition_and_unstable_preparation_is_refused() {
    let w = transition_world(&[1, 2], &[]);
    with_github(&w, |github| {
        let state = state(&w);
        github
            .observe(&["topic/1".into(), "topic/2".into()])
            .unwrap();
        let observed =
            planned_transition(github, &state.stack, 0, &["main".into(), "topic/1".into()])
                .unwrap();
        assert_eq!(observed.plan.strategy, Strategy::Publish);
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
fn transition_barriers_reject_unavailable_or_changed_evidence() {
    for case in [
        "parent-read",
        "ambiguous",
        "changed-head",
        "unpublished",
        "final-identity",
    ] {
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
            if case == "changed-head" {
                w.0.lock().unwrap().prs.get_mut(&101).unwrap()["head"]["sha"] = json!(oid(99));
            }
            if ["parent-read", "ambiguous", "changed-head"].contains(&case) {
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
        });
    }
}

#[test]
fn native_restoration_failure_obeys_current_options_on_retry() {
    let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
    for pr in w.0.lock().unwrap().prs.values_mut() {
        pr["stack"] = json!({"number":9});
    }
    w.http(
        "POST stacks",
        vec![Response::Failure("native restoration unavailable".into())],
    );
    assert!(w.run(&["--github-stack"]).is_err());
    w.0.lock().unwrap().http_responses.clear();
    w.0.lock().unwrap().trace.clear();
    w.run(&["--no-stack"]).unwrap();
    assert!(
        w.0.lock()
            .unwrap()
            .prs
            .values()
            .all(|pr| pr["stack"].is_null())
    );
    assert!(
        !w.trace()
            .iter()
            .any(|event| event["method"] == "POST" && event["path"] == "stacks")
    );
}

#[test]
fn repeated_recovery_can_recreate_a_missing_temporary_base() {
    let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
    with_github(&w, |github| {
        let state = state(&w);
        let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
        github
            .observe(&["topic/1".into(), "topic/2".into(), "topic/3".into()])
            .unwrap();
        let plan = planned_transition(github, &state.stack, 0, &bases).unwrap();
        plan.apply(github, &state, &bases).unwrap();
    });
    with_github(&w, |github| {
        let state = state(&w);
        let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
        github
            .observe(&["topic/1".into(), "topic/2".into(), "topic/3".into()])
            .unwrap();
        let plan = planned_transition(github, &state.stack, 0, &bases).unwrap();
        plan.apply(github, &state, &bases).unwrap();
    });
    // The next invocation reconstructs the transition even if its temporary ref disappeared.
    w.0.lock()
        .unwrap()
        .remote_refs
        .remove("refs/heads/git-pr/transition/101");
    w.run(&["--no-stack"]).unwrap();
}

#[test]
fn temporary_base_cleanup_is_idempotent_after_publication() {
    let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
    w.0.lock()
        .unwrap()
        .remote_refs
        .insert("refs/heads/git-pr/transition/101".into(), oid(0));
    with_github(&w, |github| {
        let state = state(&w);
        let (plan, bases) = observe(&w, github, &state);
        plan.apply(github, &state, &bases).unwrap();
        Git {
            commands: github.commands,
            repo: &github.config.repo,
        }
        .push_with_leases(&state.stack.iter().collect::<Vec<_>>(), Some(&plan.refs))
        .unwrap();
        plan.complete(github, &state).unwrap();
        w.0.lock()
            .unwrap()
            .remote_refs
            .remove("refs/heads/git-pr/transition/101");
        plan.finish(github, &state).unwrap();
        plan.finish(github, &state).unwrap();
    });
}

#[test]
fn temporary_bases_require_known_ownership_and_available_ancestry() {
    for case in [
        "invalid-owner",
        "unavailable",
        "collision-during-observation",
    ] {
        let w = transition_world(&[2, 1, 3], &[1, 2, 3]);
        if case == "invalid-owner" {
            w.0.lock().unwrap().prs.get_mut(&101).unwrap()["base"]["ref"] =
                json!("git-pr/transition/invalid");
        } else if case == "unavailable" {
            w.0.lock()
                .unwrap()
                .remote_refs
                .insert("refs/heads/git-pr/transition/101".into(), oid(0));
            w.fail(&format!(
                "git merge-base --is-ancestor {} {}",
                oid(0),
                oid(0)
            ));
        } else {
            w.reply("git ls-remote --refs -- https://github.com/org/repo.git refs/heads/git-pr/transition/101", 0,
                &format!("{}\trefs/heads/git-pr/transition/101", oid(11)));
        }
        assert!(w.run(&["--no-stack"]).is_err(), "{case}");
        assert!(!w.trace().iter().any(effect), "{case}");
    }
}

fn hydrate_created_prs(w: &World) {
    let mut m = w.0.lock().unwrap();
    let refs = m.remote_refs.clone();
    for pr in m.prs.values_mut() {
        if pr["head"]["sha"].is_null() {
            pr["head"]["sha"] =
                json!(refs[&format!("refs/heads/{}", pr["head"]["ref"].as_str().unwrap())]);
            pr["base"]["sha"] = json!(oid(0));
            pr["merged_at"] = Value::Null;
            pr["merge_commit_sha"] = Value::Null;
            if pr["stack"].is_null() {
                pr["stack"] = Value::Null;
            }
        }
    }
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

fn recovery_reply(prs: &[Value]) -> Response {
    let nodes: serde_json::Map<String, Value> = prs
        .iter()
        .enumerate()
        .map(|(i, pr)| {
            (
                format!("p{i}"),
                crate::tests::http_service::recovery_node(pr),
            )
        })
        .collect();
    Response::Output(200, json!({"data":{"repository":nodes}}).to_string())
}

#[derive(Debug)]
struct RestWithoutMembership(Arc<World>);

impl crate::github::http::Transport for RestWithoutMembership {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let rest_pr = request.method == "GET" && request.url.contains("/pulls/");
        let bytes = crate::github::http::Transport::request(self.0.as_ref(), request)?;
        if rest_pr {
            let mut value: Value = serde_json::from_slice(&bytes)?;
            value.as_object_mut().unwrap().remove("stack");
            return Ok(serde_json::to_vec(&value)?);
        }
        Ok(bytes)
    }
}

#[test]
fn ten_pr_transition_request_budget() {
    let w = transition_world(
        &(1..=10).rev().collect::<Vec<_>>(),
        &(1..=10).collect::<Vec<_>>(),
    );
    for pr in w.0.lock().unwrap().prs.values_mut() {
        pr["stack"] = json!({"number":9});
    }
    let mut app = w.app();
    app.transport = Arc::new(RestWithoutMembership(w.clone()));
    let flags = crate::cli::Flags::parse(vec!["--github-stack".into(), "--yes".into()]).unwrap();
    app.execute(
        &flags,
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
    .unwrap();
    let trace = w.trace();
    let requests = trace.iter().filter(|e| e["kind"] == "http").count();
    let reads = trace
        .iter()
        .filter(|e| {
            e["method"] == "GET" && e["path"].as_str().is_some_and(|p| p.starts_with("pulls/"))
        })
        .count();
    let batches = trace
        .iter()
        .filter(|e| e["body"]["variables"].get("r0").is_some())
        .count();
    eprintln!(
        "10-PR transition: {requests} HTTP requests, {reads} REST PR reads, {batches} recovery batches"
    );
    assert_eq!(reads, 0);
    assert_eq!(batches, 5);
    assert_eq!(requests, 41);
    let memory = w.0.lock().unwrap();
    for (index, number) in (1..=10).rev().enumerate() {
        let pr = &memory.prs[&(100 + number)];
        assert_eq!(pr["state"], "open");
        assert_eq!(pr["head"]["sha"], oid(20 + number));
        assert_eq!(
            pr["base"]["ref"],
            if index == 0 {
                "main".into()
            } else {
                format!("topic/{}", number + 1)
            }
        );
        assert_eq!(pr["stack"]["number"], 9);
    }
    drop(memory);
}

#[test]
fn combined_recovery_observations_refresh_cached_content_and_selected_metadata() {
    let w = world(1);
    with_github(&w, |github| {
        let references = ["topic/1".into()];
        github.observe(&references).unwrap();
        w.0.lock().unwrap().trace.clear();
        {
            let mut memory = w.0.lock().unwrap();
            let pr = memory.prs.get_mut(&101).unwrap();
            pr["node_id"] = json!("FRESH");
            pr["body"] = json!("Updated discussion");
            pr["labels"] = json!(["reviewed"]);
            let mut historical = pr.clone();
            historical["number"] = json!(99);
            historical["state"] = json!("closed");
            historical["head"]["sha"] = Value::Null;
            memory.prs.insert(99, historical);
        }
        let recovery = github.observe_recovery(&references, &[101]).unwrap();
        assert_eq!(recovery[0].node_id, "FRESH");
        let content = github.get(101).unwrap();
        assert_eq!(content.body.as_ref(), "Updated discussion");
        assert_eq!(content.labels[0].name(), "reviewed");
        assert_eq!(w.trace().len(), 1);
        assert!(
            w.trace()[0]["body"]["query"]
                .as_str()
                .unwrap()
                .contains("baseRefOid")
        );
        assert!(github.observe_recovery(&references, &[999]).is_err());
        assert_eq!(github.observe_recovery(&[], &[]).unwrap(), Vec::new());
    });
}

fn recovery_head_node(pr: &Value) -> Value {
    let mut node = crate::tests::http_service::recovery_node(pr);
    node["title"] = pr["title"].clone();
    node["body"] = pr["body"].clone();
    node["labels"] = json!({"nodes":[],"pageInfo":{"hasNextPage":false}});
    node
}

#[test]
fn combined_recovery_observations_preserve_pr_and_label_pagination() {
    let w = world(1);
    let mut node = recovery_head_node(&w.0.lock().unwrap().prs[&101]);
    node["labels"] =
        json!({"nodes":[{"name":"one"}],"pageInfo":{"hasNextPage":true,"endCursor":"labels"}});
    let mut duplicate = node.clone();
    duplicate["body"] = json!("Later duplicate");
    let page = |nodes: Value, more: bool| {
        Response::Output(200, json!({"data":{"repository":{"p0":{"nodes":nodes,"pageInfo":{"hasNextPage":more,"endCursor":"prs"}}}}}).to_string())
    };
    w.http("POST graphql", vec![
        page(json!([node]), true),
        page(json!([duplicate]), false),
        Response::Output(200, json!({"data":{"repository":{"p0":{"number":101,"labels":{"nodes":[{"name":"two"}],"pageInfo":{"hasNextPage":false}}}}}}).to_string()),
    ]);
    with_github(&w, |github| {
        let recovery = github
            .observe_recovery(&["topic/1".into()], &[101])
            .unwrap();
        assert_eq!(recovery[0].number, 101);
        let content = github.get(101).unwrap();
        assert_eq!(content.body.as_ref(), "Human discussion");
        assert_eq!(
            content
                .labels
                .iter()
                .map(crate::model::pull_request::Label::name)
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            w.trace()
                .iter()
                .filter(|event| event["kind"] == "http")
                .count(),
            3
        );
    });
}

#[test]
fn combined_recovery_observations_require_complete_selected_evidence() {
    let valid = recovery_head_node(&pr(101, "topic/1", 11, "main"));
    for field in ["stack", "headRefOid", "mergedAt", "baseRepository"] {
        let w = world(1);
        let mut node = valid.clone();
        node.as_object_mut().unwrap().remove(field);
        w.http("POST graphql", vec![Response::Output(200, json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
        with_github(&w, |github| {
            assert!(
                github
                    .observe_recovery(&["topic/1".into()], &[101])
                    .is_err(),
                "{field}"
            );
        });
    }
    let w = world(1);
    let mut node = valid;
    node["headRefOid"] = json!("invalid");
    w.http(
        "POST graphql",
        vec![Response::Output(
            200,
            json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}})
                .to_string(),
        )],
    );
    with_github(&w, |github| {
        assert!(
            github
                .observe_recovery(&["topic/1".into()], &[101])
                .is_err()
        );
    });
}

#[test]
fn recovery_observations_are_bounded_deduplicated_and_fresh() {
    let w = transition_world(&(1..=10).collect::<Vec<_>>(), &(1..=10).collect::<Vec<_>>());
    with_github(&w, |github| {
        let numbers: Vec<_> = (101..=110).chain(101..=110).collect();
        w.0.lock().unwrap().trace.clear();
        let first = github.recovery_prs(&numbers).unwrap();
        assert_eq!(first.len(), 10);
        assert_eq!(w.trace().iter().filter(|e| e["kind"] == "http").count(), 1);
        w.0.lock().unwrap().prs.get_mut(&110).unwrap()["node_id"] = json!("NEW");
        let second = github.recovery_prs(&numbers).unwrap();
        assert_ne!(first[9].node_id, second[9].node_id);
        assert_eq!(w.trace().iter().filter(|e| e["kind"] == "http").count(), 2);
        assert_eq!(github.recovery_prs(&[]).unwrap(), Vec::new());
    });
    let w = World::new(0);
    for number in 1..=26 {
        w.0.lock().unwrap().prs.insert(
            number,
            pr(number, &format!("topic/{number}"), number, "main"),
        );
    }
    with_github(&w, |github| {
        w.0.lock().unwrap().trace.clear();
        assert_eq!(
            github
                .recovery_prs(&(1..=26).collect::<Vec<_>>())
                .unwrap()
                .len(),
            26
        );
        let trace = w.trace();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0]["body"]["variables"].as_object().unwrap().len(), 27);
        assert_eq!(trace[1]["body"]["variables"].as_object().unwrap().len(), 3);
    });
}

#[test]
fn batched_recovery_rejects_incomplete_or_inconsistent_evidence() {
    let valid = crate::tests::http_service::recovery_node(&pr(101, "topic/1", 11, "main"));
    let mut cases = vec![Value::Null];
    for key in valid.as_object().unwrap().keys() {
        let mut node = valid.clone();
        node.as_object_mut().unwrap().remove(key);
        cases.push(node);
    }
    for (key, value) in [
        ("state", json!("UNKNOWN")),
        ("number", json!(102)),
        ("headRefOid", json!("bad")),
        ("headRepository", json!({"nameWithOwner":"foreign/repo"})),
        ("stack", json!("unknown")),
    ] {
        let mut node = valid.clone();
        node[key] = value;
        cases.push(node);
    }
    for node in cases {
        let w = world(1);
        w.http(
            "POST recovery",
            vec![Response::Output(
                200,
                json!({"data":{"repository":{"p0":node}}}).to_string(),
            )],
        );
        with_github(&w, |github| assert!(github.recovery_prs(&[101]).is_err()));
        assert!(!w.trace().iter().any(effect));
    }
    for response in [
        "{",
        "[]",
        "null",
        r#"{"data":0}"#,
        r#"{"data":{"repository":{}}}"#,
        r#"{"errors":[{"message":"unavailable"}]}"#,
    ] {
        let w = world(1);
        w.http(
            "POST recovery",
            vec![Response::Output(200, response.into())],
        );
        with_github(&w, |github| assert!(github.recovery_prs(&[101]).is_err()));
    }
    for state in ["closed", "merged"] {
        let w = world(1);
        let mut value = pr(101, "topic/1", 11, "main");
        value["state"] = json!(state);
        value["merged"] = json!(true);
        value["merged_at"] = json!("2026-10-30T00:00:00Z");
        value["merge_commit_sha"] = json!(oid(20));
        w.http("POST recovery", vec![recovery_reply(&[value])]);
        with_github(&w, |github| {
            let observed = github.recovery_prs(&[101]).unwrap();
            assert_eq!(observed[0].state, "closed");
            assert_eq!(observed[0].merge_commit_sha, Some(oid(20)));
        });
    }
    let w = world(1);
    let mut config = w.config();
    config.repo.name = "invalid".into();
    with_github(&w, |original| {
        let github = GitHub {
            config: &config,
            session: crate::github::Session::default(),
            ..*original
        };
        assert!(github.recovery_prs(&[101]).is_err());
    });
    let w = world(1);
    with_github(&w, |github| {
        assert!(github.recovery_prs(&[u64::MAX]).is_err());
    });
}

#[derive(Debug)]
struct RetargetFault {
    world: Arc<World>,
    fault: &'static str,
    mutations: std::sync::atomic::AtomicUsize,
}

impl RetargetFault {
    fn apply_concurrent_edit(&self) {
        match self.fault {
            "human-base" => {
                self.world.0.lock().unwrap().prs.get_mut(&102).unwrap()["base"]["ref"] =
                    json!("human-base");
            }
            "human-node" => {
                self.world.0.lock().unwrap().prs.get_mut(&102).unwrap()["node_id"] =
                    json!("CHANGED");
            }
            "human-head" => {
                self.world.0.lock().unwrap().prs.get_mut(&102).unwrap()["head"]["sha"] =
                    json!(oid(99));
            }
            "human-state" => {
                self.world.0.lock().unwrap().prs.get_mut(&102).unwrap()["state"] = json!("closed");
            }
            "human-membership" => {
                self.world.0.lock().unwrap().prs.get_mut(&102).unwrap()["stack"] =
                    json!({"number":9});
            }
            "changed-current" => {
                self.world.0.lock().unwrap().prs.get_mut(&101).unwrap()["head"]["sha"] =
                    json!(oid(99));
            }
            _ => {}
        }
    }
}

impl crate::github::http::Transport for RetargetFault {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let body: Value = request
            .body
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        let retarget = body["variables"]["input"].get("baseRefName").is_some();
        if !retarget
            || self
                .mutations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                != 0
        {
            return crate::github::http::Transport::request(self.world.as_ref(), request);
        }
        if self.fault.starts_with("rejected") {
            if self.fault == "rejected-unavailable" {
                self.world.http(
                    "POST recovery",
                    vec![Response::Failure("renewed evidence unavailable".into())],
                );
            }
            return Err(crate::github::http::HttpStatusError {
                url: request.url.into(),
                status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                body: "rate limited".into(),
                quota: crate::github::limits::Quota {
                    retry_after: Some(std::time::Duration::from_secs(2)),
                    ..Default::default()
                },
            }
            .into());
        }
        self.apply_concurrent_edit();
        let bytes = crate::github::http::Transport::request(self.world.as_ref(), request)?;
        let mut value: Value = serde_json::from_slice(&bytes)?;
        match self.fault {
            "lost" => bail!("lost acknowledgement"),
            "malformed" => return Ok(b"{".to_vec()),
            "missing-current" => value = json!({"data":{}}),
            "malformed-current" => {
                value["data"]["retarget"]["pullRequest"]
                    .as_object_mut()
                    .unwrap()
                    .remove("id");
            }
            "wrong-current" => value["data"]["retarget"]["pullRequest"]["number"] = json!(999),
            "partial"
            | "partial-unavailable"
            | "partial-wrong-current"
            | "partial-throttled"
            | "partial-throttled-invalid" => {
                value["errors"] = json!([{"message":if self.fault.starts_with("partial-throttled") {"rate limit exceeded"} else {"lookahead unavailable"}}]);
                value["data"]["retarget"]["pullRequest"]["repository"] = Value::Null;
                if ["partial-wrong-current", "partial-throttled-invalid"].contains(&self.fault) {
                    value["data"]["retarget"]["pullRequest"]["number"] = json!(999);
                }
            }
            "throttled" => {
                return Err(crate::github::http::HttpStatusError {
                    url: "https://api.github.com/graphql".into(),
                    status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                    body: "rate limited".into(),
                    quota: crate::github::limits::Quota {
                        retry_after: Some(std::time::Duration::from_secs(2)),
                        ..Default::default()
                    },
                }
                .into());
            }
            "missing-next" | "next-unavailable" => {
                value["data"]["retarget"]["pullRequest"]["repository"] = Value::Null;
            }
            "wrong-next" => {
                value["data"]["retarget"]["pullRequest"]["repository"]["next"]["number"] =
                    json!(999);
            }
            "stale" | "stale-human-base" => {
                let mut memory = self.world.0.lock().unwrap();
                memory.clock_elapsed += std::time::Duration::from_secs(31);
                if self.fault == "stale-human-base" {
                    memory.prs.get_mut(&102).unwrap()["base"]["ref"] = json!("human-base");
                }
            }
            _ => {}
        }
        if self.fault == "next-unavailable" || self.fault == "partial-unavailable" {
            self.world.http(
                "POST recovery",
                vec![Response::Failure("next unavailable".into())],
            );
        }
        Ok(serde_json::to_vec(&value)?)
    }
}

fn retarget_changes(w: &World) -> Vec<(RecoveryPullRequest, String)> {
    let memory = w.0.lock().unwrap();
    [101, 102]
        .map(|number| {
            (
                serde_json::from_value(memory.prs[&number].clone()).unwrap(),
                "main".into(),
            )
        })
        .to_vec()
}

#[test]
fn retarget_pipeline_recovers_uncertain_acknowledgements_without_replaying_writes() {
    for fault in [
        "lost",
        "rejected",
        "malformed",
        "missing-current",
        "malformed-current",
        "wrong-current",
        "partial",
        "partial-wrong-current",
        "partial-throttled",
        "partial-throttled-invalid",
        "throttled",
        "missing-next",
        "wrong-next",
        "stale",
    ] {
        let w = world(2);
        let transport = RetargetFault {
            world: w.clone(),
            fault,
            mutations: 0.into(),
        };
        with_github(&w, |original| {
            let github = GitHub {
                session: crate::github::Session::default(),
                transport: &transport,
                ..*original
            };
            github.retarget_recoveries(&retarget_changes(&w)).unwrap();
            assert!(github.was_updated(101));
            assert!(github.was_updated(102));
        });
        assert_eq!(
            transport
                .mutations
                .load(std::sync::atomic::Ordering::SeqCst),
            if fault == "rejected" { 3 } else { 2 },
            "{fault}"
        );
        let memory = w.0.lock().unwrap();
        assert_eq!(memory.prs[&101]["base"]["ref"], "main");
        assert_eq!(memory.prs[&102]["base"]["ref"], "main");
        drop(memory);
        if fault == "throttled" {
            assert!(w.trace().iter().any(|event| event["kind"] == "wait"));
        }
        if matches!(fault, "stale" | "partial" | "partial-throttled") {
            assert_eq!(
                w.trace()
                    .iter()
                    .filter(|event| event["body"]["variables"].get("r0").is_some())
                    .count(),
                2
            );
        }
    }
}

#[test]
fn retarget_pipeline_checks_the_next_identity_and_stops_after_a_failed_precondition() {
    for fault in [
        "human-base",
        "human-node",
        "human-head",
        "human-state",
        "human-membership",
        "changed-current",
        "stale-human-base",
        "next-unavailable",
        "partial-unavailable",
    ] {
        let w = world(2);
        let transport = RetargetFault {
            world: w.clone(),
            fault,
            mutations: 0.into(),
        };
        with_github(&w, |original| {
            let github = GitHub {
                session: crate::github::Session::default(),
                transport: &transport,
                ..*original
            };
            assert!(
                github.retarget_recoveries(&retarget_changes(&w)).is_err(),
                "{fault}"
            );
            assert_eq!(
                github.was_updated(101),
                fault != "changed-current",
                "{fault}"
            );
            assert!(!github.was_updated(102));
        });
        assert_eq!(
            transport
                .mutations
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "{fault}"
        );
        assert_ne!(w.0.lock().unwrap().prs[&102]["base"]["ref"], "main");
    }
}

#[test]
fn retarget_pipeline_rejects_an_unrepresentable_next_identity_before_dispatch() {
    let w = world(2);
    let mut changes = retarget_changes(&w);
    changes[1].0.number = u64::MAX;
    with_github(&w, |github| {
        assert!(github.retarget_recoveries(&changes).is_err());
    });
    assert!(!w.trace().iter().any(effect));
}

#[test]
fn ancestry_evidence_is_reused_only_for_matching_inputs_and_a_complete_graph() {
    for mode in ["reuse", "shallow", "different", "invalid", "unavailable"] {
        let world = world(1);
        with_github(&world, |github| {
            let state = state(&world);
            let (plan, _) = observe(&world, github, &state);
            let git = Git {
                commands: github.commands,
                repo: &github.config.repo,
            };
            let mut evidence = plan.ancestry(git, &state.stack).unwrap();
            if mode == "different" {
                evidence.0.clear();
            }
            if mode == "unavailable" {
                world.fail("git rev-parse --is-shallow-repository");
            }
            if ["shallow", "invalid"].contains(&mode) {
                world.reply(
                    "git rev-parse --is-shallow-repository",
                    0,
                    if mode == "shallow" { "true" } else { "invalid" },
                );
            }
            world.0.lock().unwrap().trace.clear();
            let result = plan.check_strategy(git, &state.stack, Some(&evidence));
            assert_eq!(
                result.is_ok(),
                !["invalid", "unavailable"].contains(&mode),
                "{mode}: {result:?}"
            );
            let ancestry_reads = world
                .trace()
                .iter()
                .filter(|event| {
                    event["args"][0] == "merge-base" && event["args"][1] == "--is-ancestor"
                })
                .count();
            assert_eq!(ancestry_reads > 0, mode == "different", "{mode}");
            assert!(!world.trace().iter().any(effect));
        });
    }
}

#[test]
fn published_transition_verification_rejects_missing_and_additional_observations() {
    let world = world(1);
    with_github(&world, |github| {
        let state = state(&world);
        let (plan, _) = observe(&world, github, &state);
        assert!(plan.verify_published(&state, &[]).is_err());
        let actual = github.recovery_prs(&plan.numbers()).unwrap();
        let extra = vec![actual[0].clone(), actual[0].clone()];
        assert!(plan.verify_published(&state, &extra).is_err());
        assert!(plan.verify_published(&state, &actual).is_err());
    });
}

#[derive(Debug)]
struct FinalReaders {
    world: Arc<World>,
    refs: std::sync::atomic::AtomicUsize,
    prs: std::sync::atomic::AtomicUsize,
    git_ready: std::sync::mpsc::SyncSender<()>,
    api_ready: std::sync::mpsc::SyncSender<()>,
    git_wait: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    api_wait: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    failure: &'static str,
}
impl crate::commands::CommandRunner for FinalReaders {
    fn edit_messages(
        &self,
        _: &str,
        _: &[String],
        _: &std::path::Path,
        _: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        _: &[(String, String)],
    ) -> Result<crate::commands::CommandOutput> {
        unreachable!()
    }
    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    ) -> Result<crate::commands::CommandOutput> {
        if args.first().is_some_and(|arg| arg == "ls-remote")
            && self.refs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1
        {
            self.git_ready.send(()).unwrap();
            self.git_wait
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("final GitHub reader did not overlap");
            if self.failure == "git" {
                bail!("final ref read failed");
            }
        }
        crate::commands::CommandRunner::execute(self.world.as_ref(), program, args, cwd, env)
    }
}
impl crate::github::http::Transport for FinalReaders {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let body: Value = request
            .body
            .map_or(Value::Null, |bytes| serde_json::from_slice(bytes).unwrap());
        if body["variables"].get("r0").is_some()
            && self.prs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1
        {
            self.api_ready.send(()).unwrap();
            self.api_wait
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("final Git reader did not overlap");
            if self.failure == "github" {
                bail!("final PR read failed");
            }
        }
        crate::github::http::Transport::request(self.world.as_ref(), request)
    }
}
#[test]
fn final_preparation_readers_overlap_and_both_finish_before_failure_returns() {
    for failure in ["none", "git", "github"] {
        let world = world(1);
        with_github(&world, |original| {
            let state = state(&world);
            let (plan, bases) = observe(&world, original, &state);
            let (git_ready, api_wait) = std::sync::mpsc::sync_channel(1);
            let (api_ready, git_wait) = std::sync::mpsc::sync_channel(1);
            let readers = Arc::new(FinalReaders {
                world: world.clone(),
                refs: 0.into(),
                prs: 0.into(),
                git_ready,
                api_ready,
                git_wait: git_wait.into(),
                api_wait: api_wait.into(),
                failure,
            });
            let mut commands = original.commands.clone();
            commands.runner = readers.clone();
            let github = GitHub {
                commands: &commands,
                transport: readers.as_ref(),
                session: crate::github::Session::default(),
                ..*original
            };
            let result = plan.apply(&github, &state, &bases);
            assert_eq!(result.is_ok(), failure == "none", "{failure}: {result:?}");
            assert_eq!(readers.refs.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert_eq!(readers.prs.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
        });
    }
}

#[test]
fn recovery_identity_cardinality_and_ancestry_requirements_fail_closed() {
    let world = world(1);
    with_github(&world, |github| {
        let state = state(&world);
        github.observe(&["topic/1".into()]).unwrap();
        let duplicate = [state.stack[0].clone(), state.stack[0].clone()];
        assert!(Transition::observe_prs(github, &duplicate, 0).is_err());
        let (mut plan, _) = observe(&world, github, &state);
        plan.anchor.clear();
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        assert!(plan.check_strategy(git, &state.stack, None).is_err());
    });
}
#[test]
fn combined_postpublication_verification_checks_the_head_object() {
    let world = world(1);
    with_github(&world, |github| {
        let mut state = state(&world);
        let plan =
            super::super::pull_requests::Publication::observe(github, &state.stack, 0).unwrap();
        let mut memory = world.0.lock().unwrap();
        let pr = memory.prs.get_mut(&101).unwrap();
        pr["base"]["ref"] = json!("main");
        pr["head"]["sha"] = json!(oid(99));
        drop(memory);
        let error = plan.verify(github, &mut state, true).unwrap_err();
        assert!(format!("{error:#}").contains("changed"), "{error:#}");
        assert!(!world.trace().iter().any(effect));
    });
}

#[test]
fn combined_head_recovery_observations_cross_the_25_head_boundary() {
    let world = world(26);
    with_github(&world, |github| {
        let references: Vec<_> = (1..=26).map(|n| format!("topic/{n}")).collect();
        let numbers: Vec<_> = (101..=126).collect();
        let observed = github.observe_recovery(&references, &numbers).unwrap();
        assert_eq!(
            observed.iter().map(|pr| pr.number).collect::<Vec<_>>(),
            numbers
        );
        assert_eq!(
            world
                .trace()
                .iter()
                .filter(|event| event["kind"] == "http")
                .count(),
            2
        );
        for (reference, number) in references.iter().zip(numbers) {
            assert_eq!(
                github.publication_pr(reference).unwrap().unwrap().number,
                number
            );
        }
    });
}
#[test]
fn publication_planning_shares_initial_destination_refs_in_direct_and_staged_paths() {
    for staged in [false, true] {
        let world = if staged {
            transition_world(&[2, 1, 3], &[1, 2, 3])
        } else {
            world(1)
        };
        with_github(&world, |github| {
            let state = state(&world);
            world.0.lock().unwrap().trace.clear();
            super::super::pull_requests::Publication::observe(github, &state.stack, 0).unwrap();
            let reads = world
                .trace()
                .iter()
                .filter(|event| event["program"] == "git" && event["args"][0] == "ls-remote")
                .count();
            assert_eq!(reads, if staged { 2 } else { 1 });
        });
    }
}

#[test]
fn small_transitions_check_graph_safety_without_extra_reuse_probes() {
    let world = world(1);
    world.0.lock().unwrap().commits[0].hash = oid(11);
    with_github(&world, |github| {
        let state = state(&world);
        world.0.lock().unwrap().trace.clear();
        super::super::pull_requests::Publication::observe(github, &state.stack, 0).unwrap();
        assert_eq!(
            world
                .trace()
                .iter()
                .filter(|event| event["args"] == json!(["rev-parse", "--is-shallow-repository"]))
                .count(),
            1
        );
    });
}

#[derive(Debug)]
struct NativeRate {
    world: Arc<World>,
    mode: &'static str,
    mutations: std::sync::atomic::AtomicUsize,
    topology: std::sync::atomic::AtomicUsize,
}
impl crate::github::http::Transport for NativeRate {
    fn response(
        &self,
        request: crate::github::http::Request<'_>,
    ) -> Result<crate::github::http::Response> {
        let topology = request.url.ends_with("stacks/9");
        let body = self.request(request)?;
        let limited = self.mode == "quota"
            && topology
            && self
                .topology
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0;
        Ok(crate::github::http::Response {
            acquired: Instant::now(),
            status: if body.is_empty() { 204 } else { 200 },
            headers: reqwest::header::HeaderMap::default(),
            quota: crate::github::limits::Quota {
                remaining: limited.then_some(0),
                reset: limited.then_some(std::time::Duration::from_secs(59)),
                ..Default::default()
            },
            body,
        })
    }
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        if request.url.ends_with("/unstack")
            || (self.mode == "create"
                && request.method == "POST"
                && request.url.ends_with("/stacks"))
            || (self.mode == "append" && request.url.ends_with("/add"))
        {
            let attempt = self
                .mutations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.mode != "quota" && (attempt == 0 || self.mode == "repeat") {
                return Err(crate::github::http::HttpStatusError {
                    url: request.url.into(),
                    status: if self.mode == "permission" {
                        reqwest::StatusCode::FORBIDDEN
                    } else {
                        reqwest::StatusCode::TOO_MANY_REQUESTS
                    },
                    body: "rejected".into(),
                    quota: crate::github::limits::Quota {
                        retry_after: (!["permission", "backoff"].contains(&self.mode)).then_some(
                            std::time::Duration::from_secs(if self.mode == "long" {
                                3600
                            } else {
                                2
                            }),
                        ),
                        ..Default::default()
                    },
                }
                .into());
            }
        }
        crate::github::http::Transport::request(self.world.as_ref(), request)
    }
}
#[derive(Debug)]
struct ChangeDuringWait<'a> {
    world: &'a World,
    mode: &'static str,
}
impl crate::commands::Clock for ChangeDuringWait<'_> {
    fn now(&self) -> Instant {
        crate::commands::Clock::now(self.world)
    }
    fn sleep(&self, duration: std::time::Duration) {
        crate::commands::Clock::sleep(self.world, duration);
        if duration.as_secs() >= 2 {
            let mut memory = self.world.0.lock().unwrap();
            if self.mode == "member" {
                let mut outside = pr(999, "outside", 99, "main");
                outside["stack"] = json!({"number":9});
                memory.prs.insert(999, outside);
            } else if self.mode == "head" {
                memory.prs.get_mut(&101).unwrap()["head"]["sha"] = json!(oid(99));
            }
        }
    }
}
#[test]
fn native_transition_renews_scope_after_quota_and_rate_waits() {
    for mode in ["quota", "rate", "backoff", "repeat", "permission", "long"] {
        for changed in [false, true] {
            let world = transition_world(&[2, 1], &[1, 2]);
            for pr in world.0.lock().unwrap().prs.values_mut() {
                pr["stack"] = json!({"number":9});
            }
            with_github(&world, |original| {
                let state = state(&world);
                let (plan, bases) = observe(&world, original, &state);
                let transport = NativeRate {
                    world: world.clone(),
                    mode,
                    mutations: 0.into(),
                    topology: 0.into(),
                };
                let clock = ChangeDuringWait {
                    world: &world,
                    mode: if changed { "member" } else { "none" },
                };
                let github = GitHub {
                    transport: &transport,
                    clock: Some(&clock),
                    session: crate::github::Session::default(),
                    ..*original
                };
                let result = plan.apply(&github, &state, &bases);
                let expected = !changed && ["quota", "rate", "backoff"].contains(&mode);
                assert_eq!(result.is_ok(), expected, "{mode}/{changed}: {result:?}");
                let mutations = transport
                    .mutations
                    .load(std::sync::atomic::Ordering::SeqCst);
                assert_eq!(
                    mutations,
                    if mode == "quota" {
                        usize::from(!changed)
                    } else if changed || ["permission", "long"].contains(&mode) {
                        1
                    } else if mode == "repeat" {
                        4
                    } else {
                        2
                    }
                );
                let outcome = github.commands.outcomes.snapshot()
                    [&crate::model::outcome::Operation::NativeStack];
                assert_eq!(outcome.confirmed, expected);
                assert!(!outcome.unconfirmed);
                if !expected {
                    assert!(!world.trace().iter().any(effect));
                }
            });
        }
    }
}
#[derive(Debug)]
struct SlowCombined {
    world: Arc<World>,
    mode: &'static str,
    reads: std::sync::atomic::AtomicUsize,
}
impl crate::github::http::Transport for SlowCombined {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let body: Value = request
            .body
            .map_or(Value::Null, |body| serde_json::from_slice(body).unwrap());
        let combined = body["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("query RecoveryHeads"));
        let slow = combined
            && (self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
                || self.mode == "always");
        let bytes = crate::github::http::Transport::request(self.world.as_ref(), request)?;
        if slow {
            let mut memory = self.world.0.lock().unwrap();
            memory.clock_elapsed += std::time::Duration::from_secs(31);
            if self.mode == "head" {
                memory.prs.get_mut(&101).unwrap()["head"]["sha"] = json!(oid(99));
            }
            if self.mode == "membership" {
                memory.prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
            }
        }
        Ok(bytes)
    }
}
#[test]
fn postpublication_renewal_keeps_recovery_and_ordinary_evidence_together() {
    for mode in ["same", "head", "membership", "always"] {
        let world = world(1);
        with_github(&world, |original| {
            let mut state = state(&world);
            let plan = super::super::pull_requests::Publication::observe(original, &state.stack, 0)
                .unwrap();
            let mut memory = world.0.lock().unwrap();
            memory.prs.get_mut(&101).unwrap()["base"]["ref"] = json!("main");
            memory.prs.get_mut(&101).unwrap()["head"]["sha"] = json!(state.stack[0].hash);
            memory.trace.clear();
            drop(memory);
            let transport = SlowCombined {
                world: world.clone(),
                mode,
                reads: 0.into(),
            };
            let github = GitHub {
                transport: &transport,
                session: crate::github::Session::default(),
                ..*original
            };
            let result = plan.verify(&github, &mut state, true);
            assert_eq!(result.is_ok(), mode == "same", "{mode}: {result:?}");
            assert_eq!(
                transport.reads.load(std::sync::atomic::Ordering::SeqCst),
                if mode == "always" { 4 } else { 2 }
            );
            assert!(!world.trace().iter().any(effect));
            assert!(
                world
                    .trace()
                    .iter()
                    .filter(|event| event["kind"] == "http")
                    .all(|event| event["body"]["query"]
                        .as_str()
                        .is_some_and(|query| query.starts_with("query RecoveryHeads")))
            );
        });
    }
}

#[test]
fn rate_rejected_base_updates_recheck_concurrent_head_changes_before_retrying() {
    for unavailable in [false, true] {
        let world = world(2);
        with_github(&world, |original| {
            let transport = RetargetFault {
                world: world.clone(),
                fault: if unavailable {
                    "rejected-unavailable"
                } else {
                    "rejected"
                },
                mutations: 0.into(),
            };
            let clock = ChangeDuringWait {
                world: &world,
                mode: if unavailable { "none" } else { "head" },
            };
            let github = GitHub {
                transport: &transport,
                clock: Some(&clock),
                session: crate::github::Session::default(),
                ..*original
            };
            assert!(
                github
                    .retarget_recoveries(&retarget_changes(&world))
                    .is_err()
            );
            assert_eq!(
                transport
                    .mutations
                    .load(std::sync::atomic::Ordering::SeqCst),
                1
            );
            let outcome =
                github.commands.outcomes.snapshot()[&crate::model::outcome::Operation::Base(101)];
            assert!(!outcome.unconfirmed && !outcome.confirmed);
            assert!(!world.trace().iter().any(is_retarget));
        });
    }
}

#[test]
fn native_creation_and_append_recover_definite_rate_rejections() {
    for mode in ["create", "append"] {
        let world = World::new(2);
        world.run(&["--no-stack"]).unwrap();
        if mode == "append" {
            world.0.lock().unwrap().prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
        }
        with_github(&world, |original| {
            let mut state = state(&world);
            for (index, commit) in state.stack.iter_mut().enumerate() {
                commit.pr_number = 101 + index as u64;
            }
            let transport = NativeRate {
                world: world.clone(),
                mode,
                mutations: 0.into(),
                topology: 0.into(),
            };
            let github = GitHub {
                transport: &transport,
                session: crate::github::Session::default(),
                ..*original
            };
            github.manage_stack(&state.stack, "main").unwrap();
            assert_eq!(
                transport
                    .mutations
                    .load(std::sync::atomic::Ordering::SeqCst),
                2
            );
            let outcome =
                github.commands.outcomes.snapshot()[&crate::model::outcome::Operation::NativeStack];
            assert!(outcome.confirmed && !outcome.unconfirmed);
        });
    }
}

#[test]
fn shallow_transitions_use_local_parent_evidence_without_full_history() {
    for order in [&[2, 1][..], &[1, 3, 2][..]] {
        let world = transition_world(order, &[1, 2]);
        world.reply("git rev-parse --is-shallow-repository", 0, "true");
        world.reply(
            &format!("git merge-base --is-ancestor {} {}", oid(11), oid(12)),
            1,
            "",
        );
        // Older history is genuinely absent; all relevant paths meet at this boundary.
        world
            .0
            .lock()
            .unwrap()
            .parents
            .insert(oid(0), vec![oid(999)]);
        with_github(&world, |github| {
            let state = state(&world);
            let (plan, bases) = observe(&world, github, &state);
            assert!(matches!(plan.plan.strategy, Strategy::Stage(_)));
            plan.apply(github, &state, &bases).unwrap();
        });
    }
    let world = transition_world(&[2, 1], &[1, 2]);
    world.reply("git rev-parse --is-shallow-repository", 0, "true");
    world.0.lock().unwrap().parents.remove(&oid(12));
    with_github(&world, |github| {
        let state = state(&world);
        let bases = crate::stack::selection::resolve_bases(&state.stack, 0, "main");
        github
            .observe(&["topic/1".into(), "topic/2".into()])
            .unwrap();
        let error = planned_transition(github, &state.stack, 0, &bases).unwrap_err();
        assert!(error.to_string().contains("unavailable commit"));
        assert!(!world.trace().iter().any(effect));
    });
}

#[derive(Debug)]
struct TransitionTopologyDelay<'a> {
    world: &'a World,
    armed: std::sync::atomic::AtomicBool,
}
impl crate::github::http::Transport for TransitionTopologyDelay<'_> {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let topology = request.url.ends_with("stacks/9");
        let body = crate::github::http::Transport::request(self.world, request)?;
        if topology && self.armed.load(std::sync::atomic::Ordering::SeqCst) {
            self.world.0.lock().unwrap().clock_elapsed += std::time::Duration::from_secs(901);
        }
        Ok(body)
    }
}
#[test]
fn transition_native_budget_includes_the_final_topology_observation() {
    let world = transition_world(&[2, 1], &[1, 2]);
    for pr in world.0.lock().unwrap().prs.values_mut() {
        pr["stack"] = json!({"number":9});
    }
    with_github(&world, |original| {
        let transport = TransitionTopologyDelay {
            world: &world,
            armed: false.into(),
        };
        let github = GitHub {
            transport: &transport,
            session: crate::github::Session::default(),
            ..*original
        };
        let state = state(&world);
        let (plan, bases) = observe(&world, &github, &state);
        transport
            .armed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            format!("{:#}", plan.apply(&github, &state, &bases).unwrap_err())
                .contains("cumulative wait budget")
        );
        assert!(!world.trace().iter().any(effect));
    });
}
#[test]
fn budgeted_recovery_membership_fallback_keeps_the_outer_request_context() {
    let world = world(1);
    world
        .0
        .lock()
        .unwrap()
        .prs
        .get_mut(&101)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("stack");
    with_github(&world, |github| {
        assert_eq!(
            github
                .recovery_pr_with_budget(101, Some(github.now()))
                .unwrap()
                .number,
            101
        );
    });
}

#[test]
fn shallow_merged_parent_and_temporary_guards_use_available_local_proofs() {
    for unavailable_native in [false, true] {
        let world = world(1);
        world.reply("git rev-parse --is-shallow-repository", 0, "true");
        let key = format!("git merge-base --is-ancestor {} {}", oid(10), oid(11));
        if unavailable_native {
            world.fail(&key);
        } else {
            world.reply(&key, 1, "");
        }
        world
            .0
            .lock()
            .unwrap()
            .remote_refs
            .insert("refs/heads/git-pr/transition/101".into(), oid(20));
        world.reply(
            &format!("git merge-base --is-ancestor {} {}", oid(20), oid(0)),
            1,
            "",
        );
        with_github(&world, |github| {
            let state = state(&world);
            let (plan, _) = observe(&world, github, &state);
            assert_eq!(plan.cleanup_candidates()["git-pr/transition/101"], oid(20));
            assert!(!world.trace().iter().any(effect));
        });
    }
}

#[test]
fn shallow_unreviewed_heads_preserve_related_local_history_without_adoption() {
    for available in [false, true] {
        let world = World::new(1);
        let old = oid(10);
        world.reply("git rev-parse --is-shallow-repository", 0, "true");
        world.reply(
            &format!("git merge-base --is-ancestor {old} {}", oid(1)),
            1,
            "",
        );
        {
            let mut memory = world.0.lock().unwrap();
            memory.remote_refs.insert("refs/heads/topic/1".into(), old);
            if available {
                memory.parents.insert(oid(1), vec![oid(10)]);
            }
        }
        with_github(&world, |github| {
            let mut stack = state(&world).stack;
            stack[0].publication = Some(crate::identity::Binding {
                anchor: None,
                anchor_oid: None,
                head: crate::identity::Head::new(&github.config.repo, "topic/1").unwrap(),
                review: None,
                adopted: false,
                previous_anchor: None,
            });
            let result = super::super::pull_requests::Publication::observe(github, &stack, 0);
            assert_eq!(result.is_ok(), available, "{result:?}");
            assert!(!world.trace().iter().any(effect));
        });
    }
}
