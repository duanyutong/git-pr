use super::{State, presentation::validate_duplicates};
use crate::tests::unit::support::{World, commit, log};
use crate::{git::Git, github::GitHub, model::Commit};

fn publication_effect(event: &serde_json::Value) -> bool {
    event["args"][0] == "push"
        || event["args"][0] == "reword"
        || event["method"] == "PATCH"
        || event["method"] == "DELETE"
        || (event["method"] == "POST"
            && (event["path"] != "graphql"
                || event["body"]["query"]
                    .as_str()
                    .is_some_and(|q| q.starts_with("mutation"))))
}

#[test]
fn publication_rejects_a_changed_local_selection_after_preparation() {
    for change in ["removed", "skipped", "renamed"] {
        let w = World::new(1);
        let config = w.config();
        let commands = w.commands();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1)]);
        let plan = super::pull_requests::Publication::observe(&github, &s.stack, 0).unwrap();
        match change {
            "removed" => s.stack.clear(),
            "skipped" => s.stack[0].skip = true,
            "renamed" => s.stack[0].set_attr(crate::model::REMOTE_REF, "other"),
            _ => unreachable!(),
        }
        assert!(plan.verify(&github, &mut s, false).is_err());
        assert!(!w.trace().iter().any(publication_effect));
    }
}

#[test]
fn publication_rejects_a_different_selection_with_the_same_number_of_commits() {
    let world = World::new(2);
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "",
    };
    let mut state = state(vec![commit(1), commit(2)]);
    state.stack[1].skip = true;
    let prepared = super::pull_requests::Publication::observe(&github, &state.stack, 0).unwrap();
    state.stack[0].skip = true;
    state.stack[1].skip = false;
    world.0.lock().unwrap().trace.clear();
    let error = prepared.verify(&github, &mut state, false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("selection changed after preparation")
    );
    assert_eq!(state.stack[0].hash, commit(1).hash);
    assert_eq!(state.stack[1].hash, commit(2).hash);
    assert!(!world.trace().iter().any(publication_effect));
}

#[test]
fn explicit_head_is_resolved_before_read_only_rewrite_position_preflight() {
    use crate::{cli::Flags, config};
    use std::collections::BTreeMap;

    let world = World::new(2);
    {
        let mut memory = world.0.lock().unwrap();
        memory.branchless = true;
        memory.commits[0].attrs.clear();
    }
    world.reply("git rev-parse --verify HEAD^{commit}", 0, &commit(2).hash);
    world.reply(
        "git branch --format=%(refname:strip=2)|%(objectname)",
        0,
        &format!("topic/1|{}", commit(1).hash),
    );
    let flags = Flags::parse(vec![
        "--dry-run".into(),
        "--no-stack".into(),
        "refs/remotes/origin/main..HEAD".into(),
    ])
    .unwrap();
    let mut commands = world.commands();
    let config::Loaded::Ready(mut config) = config::load(
        &flags,
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
        &mut commands,
        world.as_ref(),
        world.as_ref(),
    )
    .unwrap() else {
        panic!("expected resolved configuration");
    };
    assert_eq!(config.selection.tip, commit(2).hash);
    let mut state = State::discover(&mut config, &commands).unwrap();
    assert_eq!(state.tip, commit(2).hash);
    let mut prepared = false;
    world.0.lock().unwrap().trace.clear();
    state
        .rewrite(&config, &commands, world.as_ref(), &mut |_, _| {
            prepared = true;
            Ok(())
        })
        .unwrap();
    assert!(prepared);
    let trace = world.trace();
    assert!(trace.iter().any(|event| {
        event["args"][0] == "rev-list" && event["args"][2] == format!("{}..HEAD", commit(2).hash)
    }));
    assert!(
        trace.iter().any(|event| {
            event["args"][0] == "rev-parse" && event["args"][2] == "HEAD~0^{commit}"
        })
    );
    assert!(!trace.iter().any(|event| {
        publication_effect(event)
            || matches!(
                event["args"][0].as_str(),
                Some("reset" | "rebase" | "describe")
            )
    }));
}

#[test]
fn publication_barrier_read_failures_stop_reconciliation_without_using_stale_state() {
    for expire_head in [false, true] {
        let w = World::new(1);
        w.run(&["--no-stack"]).unwrap();
        let config = w.config();
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1)]);
        let plan = super::pull_requests::Publication::observe(&github, &s.stack, 0).unwrap();
        if expire_head {
            github.expire_head_observation_for_test("topic/1");
        } else {
            github.expire_pr_observation_for_test(101);
        }
        w.http(
            "POST graphql",
            vec![crate::tests::unit::support::Response::Failure(
                "barrier unavailable".into(),
            )],
        );
        w.0.lock().unwrap().trace.clear();
        assert!(
            plan.verify(&github, &mut s, false)
                .unwrap_err()
                .to_string()
                .contains("GitHub")
        );
        assert!(!w.trace().iter().any(publication_effect));
    }
}

#[test]
fn base_verification_rejects_changes_after_creation_without_retargeting() {
    let w = World::new(1);
    w.run(&["--no-stack"]).unwrap();
    w.0.lock().unwrap().prs.get_mut(&101).unwrap()["base"]["ref"] = serde_json::json!("other");
    let config = w.config();
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    let mut s = state(vec![commit(1)]);
    s.stack[0].pr_number = 101;
    w.0.lock().unwrap().trace.clear();
    assert!(
        app.reconcile_prs(&["main".into()], &github, &mut s, &[0])
            .is_err()
    );
    assert!(!w.trace().iter().any(publication_effect));
}

#[test]
fn reordered_publications_stop_before_rewriting_or_remote_effects_in_both_modes() {
    for native in [false, true] {
        for infer_refs in [false, true] {
            let w = World::new(4);
            let flags = [
                if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                "--yes",
            ];
            w.run(&flags).unwrap();
            let (commits, prs, branches) = {
                let mut memory = w.0.lock().unwrap();
                memory.branchless = true;
                memory.commits.swap(0, 1);
                let branches = memory
                    .commits
                    .iter()
                    .map(|c| format!("{}|{}", c.remote_ref(), c.hash))
                    .collect::<Vec<_>>()
                    .join("\n");
                if infer_refs {
                    for commit in &mut memory.commits {
                        commit.attrs.clear();
                    }
                }
                (memory.commits.clone(), memory.prs.clone(), branches)
            };
            w.reply(
                "git branch --format=%(refname:strip=2)|%(objectname)",
                0,
                &branches,
            );
            for _ in 0..2 {
                w.0.lock().unwrap().trace.clear();
                let error = format!("{:#}", w.run(&flags).unwrap_err());
                assert!(error.contains("unsafe publication: PR #102"), "{error}");
                assert!(error.contains("no local rewrites, pushes or PR changes"));
                let memory = w.0.lock().unwrap();
                assert_eq!(memory.commits, commits);
                assert_eq!(memory.prs, prs);
                assert!(!memory.trace.iter().any(publication_effect));
                drop(memory);
            }
        }
    }
}

#[test]
fn closed_and_merged_heads_cannot_be_recreated_by_retrying_submission() {
    for native in [false, true] {
        for merged in [false, true] {
            let w = World::new(2);
            let flags = [
                if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                "--yes",
            ];
            w.run(&flags).unwrap();
            {
                let mut memory = w.0.lock().unwrap();
                let pr = memory.prs.get_mut(&101).unwrap();
                pr["state"] = serde_json::json!("closed");
                pr["merged"] = serde_json::json!(merged);
                pr["base"]["ref"] = serde_json::json!("topic/2");
                drop(memory);
            }
            for _ in 0..2 {
                w.0.lock().unwrap().trace.clear();
                let error = format!("{:#}", w.run(&flags).unwrap_err());
                assert!(
                    error.contains(if merged { "is merged" } else { "is closed" }),
                    "{error}"
                );
                assert!(
                    error.contains("recorded base \"topic/2\"")
                        && error.contains("refusing to publish or create a replacement")
                );
                assert!(!error.contains("git rebase"));
                assert!(!w.trace().iter().any(publication_effect));
                assert_eq!(w.0.lock().unwrap().prs.len(), 2);
            }
        }
    }
}

#[test]
fn publication_observes_lifecycle_changes_inside_the_cache_lifetime_and_pins_identity() {
    for change in [
        "closed",
        "merged",
        "missing",
        "replacement",
        "node",
        "base",
        "appeared",
    ] {
        let w = World::new(2);
        w.run(&["--no-stack"]).unwrap();
        {
            let mut memory = w.0.lock().unwrap();
            let mut pr = memory.prs[&101].clone();
            match change {
                "closed" => pr["state"] = serde_json::json!("closed"),
                "merged" => pr["merged"] = serde_json::json!(true),
                "replacement" => {
                    pr["number"] = serde_json::json!(201);
                    memory.changes_on_push.insert(101, None);
                }
                "node" => pr["node_id"] = serde_json::json!("DIFFERENT"),
                "base" => pr["base"]["ref"] = serde_json::json!("different"),
                "appeared" => {
                    memory.prs.remove(&101);
                }
                "missing" => {}
                _ => unreachable!(),
            }
            let number = pr["number"].as_u64().unwrap();
            memory
                .changes_on_push
                .insert(number, if change == "missing" { None } else { Some(pr) });
            memory.trace.clear();
            memory.stdout.clear();
        }
        let error = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
        assert!(
            error.contains("after publication; pushed refs are retained"),
            "{change}: {error}"
        );
        let memory = w.0.lock().unwrap();
        assert_eq!(
            memory
                .trace
                .iter()
                .filter(|e| e["args"][0] == "push")
                .count(),
            1
        );
        assert!(
            !memory
                .trace
                .iter()
                .any(|e| publication_effect(e) && e["kind"] == "http")
        );
        assert!(!memory.stdout.contains("Stack of"));
        drop(memory);
    }
}

#[test]
fn unselected_dependants_and_ambiguous_open_heads_prevent_publication() {
    for ambiguous in [false, true] {
        let w = World::new(2);
        w.run(&["--no-stack"]).unwrap();
        {
            let mut memory = w.0.lock().unwrap();
            let mut external = memory.prs[&102].clone();
            external["number"] = serde_json::json!(201);
            external["node_id"] = serde_json::json!("PR_201");
            if !ambiguous {
                external["head"]["ref"] = serde_json::json!("outside");
            }
            memory.prs.insert(201, external);
            memory.trace.clear();
        }
        let error = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
        assert!(
            error.contains(if ambiguous {
                "multiple open PRs"
            } else {
                "outside the verified selection"
            }),
            "{error}"
        );
        assert!(!w.trace().iter().any(publication_effect));
    }
}

#[test]
fn expired_publication_dependants_stop_new_or_retargeted_outside_prs_before_push() {
    for change in ["created", "retargeted"] {
        let world = World::new(2);
        world.run(&["--no-stack"]).unwrap();
        let config = world.config();
        let app = world.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: Some(world.as_ref()),
            config: &config,
            commands: &app.commands,
            transport: world.as_ref(),
            template: "",
        };
        let mut external = world.0.lock().unwrap().prs[&102].clone();
        external["number"] = serde_json::json!(201);
        external["node_id"] = serde_json::json!("PR_201");
        external["head"]["ref"] = serde_json::json!("outside");
        if change == "retargeted" {
            external["base"]["ref"] = serde_json::json!("other");
            world.0.lock().unwrap().prs.insert(201, external.clone());
        }
        let mut selected = state(vec![commit(1), commit(2)]);
        let plan = super::pull_requests::Publication::observe(&github, &selected.stack, 0).unwrap();
        {
            let mut memory = world.0.lock().unwrap();
            memory.clock_elapsed += std::time::Duration::from_secs(31);
            external["base"]["ref"] = serde_json::json!("topic/1");
            memory.prs.insert(201, external);
            memory.trace.clear();
        }
        let error = app
            .push(
                &config,
                Git {
                    commands: &app.commands,
                    repo: &config.repo,
                },
                &github,
                &mut selected,
                &plan,
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("outside the verified selection"),
            "{change}: {error:#}"
        );
        assert!(!world.trace().iter().any(publication_effect));
    }
}

#[test]
fn publication_reuses_fresh_dependants_and_renews_only_expired_observations() {
    let world = World::new(2);
    world.run(&["--no-stack"]).unwrap();
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: Some(world.as_ref()),
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "",
    };
    let mut selected = state(vec![commit(1), commit(2)]);
    let plan = super::pull_requests::Publication::observe(&github, &selected.stack, 0).unwrap();
    world.0.lock().unwrap().trace.clear();
    plan.verify(&github, &mut selected, false).unwrap();
    world.0.lock().unwrap().clock_elapsed += std::time::Duration::from_secs(30);
    plan.verify(&github, &mut selected, false).unwrap();
    assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
    world.0.lock().unwrap().clock_elapsed += std::time::Duration::from_secs(1);
    plan.verify(&github, &mut selected, false).unwrap();
    let refreshed = world.trace();
    assert_eq!(refreshed.len(), 1);
    assert!(
        refreshed[0]["body"]["query"]
            .as_str()
            .unwrap()
            .contains("baseRefName:")
    );
    world.0.lock().unwrap().trace.clear();
    plan.verify(&github, &mut selected, false).unwrap();
    assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
}

#[test]
fn failed_dependant_refresh_stops_publication_and_does_not_renew_the_observation() {
    let world = World::new(2);
    world.run(&["--no-stack"]).unwrap();
    let config = world.config();
    let app = world.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: Some(world.as_ref()),
        config: &config,
        commands: &app.commands,
        transport: world.as_ref(),
        template: "",
    };
    let mut selected = state(vec![commit(1), commit(2)]);
    let plan = super::pull_requests::Publication::observe(&github, &selected.stack, 0).unwrap();
    world.0.lock().unwrap().clock_elapsed += std::time::Duration::from_secs(31);
    world.http(
        "POST graphql",
        vec![crate::tests::unit::support::Response::Failure(
            "dependant acquisition unavailable".into(),
        )],
    );
    world.0.lock().unwrap().trace.clear();
    let error = app
        .push(
            &config,
            Git {
                commands: &app.commands,
                repo: &config.repo,
            },
            &github,
            &mut selected,
            &plan,
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("dependant acquisition unavailable"));
    assert!(!world.trace().iter().any(publication_effect));
    {
        let mut memory = world.0.lock().unwrap();
        memory.http_responses.clear();
        let mut external = memory.prs[&102].clone();
        external["number"] = serde_json::json!(201);
        external["node_id"] = serde_json::json!("PR_201");
        external["head"]["ref"] = serde_json::json!("outside");
        memory.prs.insert(201, external);
        memory.trace.clear();
    }
    assert!(
        plan.verify(&github, &mut selected, false)
            .unwrap_err()
            .to_string()
            .contains("outside the verified selection")
    );
    assert!(!world.trace().iter().any(publication_effect));
}

#[test]
fn slow_dependant_acquisition_preserves_the_age_of_each_groups_first_page() {
    use crate::{
        commands::Clock,
        github::http::{Request, Transport},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Debug)]
    struct SlowLaterGroup {
        world: Arc<World>,
        delayed: AtomicBool,
    }

    impl Transport for SlowLaterGroup {
        fn request(&self, request: Request<'_>) -> anyhow::Result<Vec<u8>> {
            let payload: serde_json::Value = request
                .body
                .map(serde_json::from_slice)
                .transpose()?
                .unwrap_or_default();
            let response = self.world.request(request)?;
            if payload["query"]
                .as_str()
                .is_some_and(|query| query.contains("baseRefName:"))
                && payload["variables"]["b0"] == "topic/26"
                && !self.delayed.swap(true, Ordering::Relaxed)
            {
                self.world.sleep(std::time::Duration::from_secs(31));
            }
            Ok(response)
        }
    }

    for base in ["topic/1", "topic/26"] {
        let world = World::new(26);
        world.run(&["--no-stack"]).unwrap();
        let config = world.config();
        let commands = world.commands();
        let transport = SlowLaterGroup {
            world: world.clone(),
            delayed: AtomicBool::new(false),
        };
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: Some(world.as_ref()),
            config: &config,
            commands: &commands,
            transport: &transport,
            template: "",
        };
        let mut selected = state((1..=26).map(commit).collect());
        let plan = super::pull_requests::Publication::observe(&github, &selected.stack, 0).unwrap();
        assert!(transport.delayed.load(Ordering::Relaxed));
        {
            let mut memory = world.0.lock().unwrap();
            let mut external = memory.prs[&102].clone();
            external["number"] = serde_json::json!(201);
            external["node_id"] = serde_json::json!("PR_201");
            external["head"]["ref"] = serde_json::json!("outside");
            external["base"]["ref"] = serde_json::json!(base);
            memory.prs.insert(201, external);
            memory.trace.clear();
        }
        assert!(
            plan.verify(&github, &mut selected, false)
                .unwrap_err()
                .to_string()
                .contains("outside the verified selection")
        );
        assert!(!world.trace().iter().any(publication_effect));
    }
}

fn state(commits: Vec<Commit>) -> State {
    State {
        stack: commits,
        validated_refs: std::collections::HashSet::default(),
        start: 0,
        trunk: "refs/remotes/origin/main".into(),
        trunk_hash: format!("{:040x}", 0),
        tip: "HEAD".into(),
        observed_tip: "HEAD".into(),
        base: "refs/remotes/origin/main".into(),
        tip_change_id: String::new(),
        tip_depth: 0,
        selected_depths: Vec::new(),
        selected_changes: Vec::new(),
    }
}

#[test]
fn cached_number_assignment_retains_description_storage_and_commit_metadata() {
    use crate::tests::unit::support::WORKER_STARTS;
    for existing in [false, true] {
        let world = World::new(100);
        if existing {
            world.run(&["--no-stack"]).unwrap();
        }
        let config = world.config();
        let commands = world.commands();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: world.as_ref(),
            template: "",
        };
        let mut selected = state(
            (1..=100)
                .map(|number| {
                    let mut item = commit(number);
                    item.message = "m".repeat(60_000);
                    if !existing {
                        item.skip = number != 1;
                        item.pr_number = if number == 1 { 101 } else { 0 };
                    }
                    item
                })
                .collect(),
        );
        github
            .observe(
                &selected
                    .stack
                    .iter()
                    .map(|c| c.remote_ref().to_owned())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let messages = selected
            .stack
            .iter()
            .map(|c| c.message.as_ptr())
            .collect::<Vec<_>>();
        let original = selected.stack.clone();
        world.0.lock().unwrap().trace.clear();
        WORKER_STARTS.set(0);
        selected
            .fill_numbers(&github, &vec!["main".into(); 100])
            .unwrap();
        for (index, ((item, pointer), mut expected)) in selected
            .stack
            .iter()
            .zip(messages)
            .zip(original)
            .enumerate()
        {
            assert_eq!(item.message.as_ptr(), pointer);
            if existing {
                expected.pr_number = 101 + u64::try_from(index).unwrap();
            }
            assert_eq!(*item, expected);
        }
        assert_eq!(WORKER_STARTS.get(), 0);
        assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
    }
}

#[test]
fn ordinary_issue_references_survive_description_submission_without_history_reads() {
    for emoji in [false, true] {
        let w = World::new(1);
        w.run(&["--no-stack"]).unwrap();
        let human =
            "Human summary\n---\n## Verification\n* Fixes #999\nManual verification details";
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] = serde_json::json!(human);
        let mut config = w.config();
        config.emoji = emoji;
        config.selection.explicit = true;
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1)]);
        s.stack[0].pr_number = 101;
        let git = Git {
            commands: &app.commands,
            repo: &config.repo,
        };
        w.0.lock().unwrap().trace.clear();
        super::Application::update_descriptions(&config, git, &github, &s).unwrap();
        let m = w.0.lock().unwrap();
        assert!(m.prs[&101]["body"].as_str().unwrap().starts_with(human));
        assert!(!m.trace.iter().any(|v| {
            v["path"] == "pulls/999"
                || v["body"]["query"]
                    .as_str()
                    .is_some_and(|q| q.contains("number:999"))
        }));
        drop(m);
    }
}

#[test]
fn malformed_footer_errors_preserve_descriptions_and_stop_content_mutations() {
    let start = crate::stack::description::START;
    let end = crate::stack::description::END;
    for native in [false, true] {
        for compact in [false, true] {
            for malformed in [
                format!("Human summary\n{start}\nHuman notes\nBot status"),
                format!("Human summary\n{end}\nHuman notes\nBot status"),
                format!(
                    "Human summary\n{start}\n* #101\n{end}\nHuman notes\n{start}\n* #102\n{end}\nBot status"
                ),
            ] {
                let w = World::new(2);
                w.run(&["--no-stack"]).unwrap();
                w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] =
                    serde_json::json!(malformed);
                let args = [
                    if native {
                        "--github-stack"
                    } else {
                        "--no-stack"
                    },
                    if compact {
                        "--compact-stack"
                    } else {
                        "--compact-stack=false"
                    },
                    "--yes",
                ];
                for _ in 0..2 {
                    w.0.lock().unwrap().trace.clear();
                    let diagnostic = format!("{:#}", w.run(&args).unwrap_err());
                    assert!(diagnostic.contains("PR #101 description"));
                    assert!(diagnostic.contains("malformed managed stack footer"));
                    assert!(diagnostic.contains("preserving human text"));
                    let memory = w.0.lock().unwrap();
                    assert_eq!(memory.prs[&101]["body"], malformed);
                    assert!(!memory.trace.iter().any(content_mutation));
                    drop(memory);
                }
                w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] =
                    serde_json::json!("Human summary\nHuman notes\nBot status");
                w.run(&args).unwrap();
                {
                    let memory = w.0.lock().unwrap();
                    let body = memory.prs[&101]["body"].as_str().unwrap();
                    assert!(body.contains("Human notes") && body.contains("Bot status"));
                    assert_eq!(body.matches(start).count(), 1);
                    assert_eq!(body.matches(end).count(), 1);
                    drop(memory);
                }
                w.0.lock().unwrap().trace.clear();
                w.run(&args).unwrap();
                assert!(!w.trace().iter().any(content_mutation));
            }
        }
    }
}

fn content_mutation(event: &serde_json::Value) -> bool {
    event["kind"] == "http"
        && event["method"] != "GET"
        && (event["path"] != "graphql"
            || event["body"]["query"]
                .as_str()
                .is_some_and(|query| query.starts_with("mutation")))
}

#[test]
fn reserved_initial_description_inputs_fail_before_writes_and_allow_safe_repair() {
    for native in [false, true] {
        for compact in [false, true] {
            for template_source in [false, true] {
                for existing in [false, true] {
                    let w = World::new(2);
                    if existing {
                        w.run(&["--no-stack"]).unwrap();
                    }
                    let seed = format!(
                        "Human notes\n```html\n{}\n```",
                        crate::stack::description::START
                    );
                    let template_path =
                        std::path::PathBuf::from("/unit/repo/.github/PULL_REQUEST_TEMPLATE.md");
                    {
                        let mut memory = w.0.lock().unwrap();
                        if existing {
                            memory.prs.get_mut(&101).unwrap()["body"] = serde_json::json!("");
                        }
                        if template_source {
                            memory.commits[0].message.clear();
                            memory.files.insert(template_path.clone(), seed.clone());
                        } else {
                            memory.commits[0].message.clone_from(&seed);
                        }
                    }
                    let args = [
                        if native {
                            "--github-stack"
                        } else {
                            "--no-stack"
                        },
                        if compact {
                            "--compact-stack"
                        } else {
                            "--compact-stack=false"
                        },
                        "--yes",
                    ];
                    for _ in 0..2 {
                        w.0.lock().unwrap().trace.clear();
                        let diagnostic = format!("{:#}", w.run(&args).unwrap_err());
                        assert!(diagnostic.contains(if template_source {
                            "PR template"
                        } else {
                            "commit message"
                        }));
                        assert!(diagnostic.contains("reserved managed-footer delimiters"));
                        assert!(diagnostic.contains("&lt;"));
                        assert!(!diagnostic.contains("Human notes"));
                        if existing {
                            assert!(diagnostic.contains("PR #101 description"));
                            assert_eq!(w.0.lock().unwrap().prs[&101]["body"], "");
                        } else {
                            assert!(w.0.lock().unwrap().prs.is_empty());
                        }
                        assert!(!w.trace().iter().any(content_mutation));
                    }
                    let repaired = seed.replace('<', "&lt;");
                    {
                        let mut memory = w.0.lock().unwrap();
                        if template_source {
                            memory.files.insert(template_path, repaired.clone());
                        } else {
                            memory.commits[0].message.clone_from(&repaired);
                        }
                    }
                    w.run(&args).unwrap();
                    let body = w.0.lock().unwrap().prs[&101]["body"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                    assert!(body.contains(&repaired));
                    assert_eq!(body.matches(crate::stack::description::START).count(), 1);
                    assert_eq!(body.matches(crate::stack::description::END).count(), 1);
                    w.0.lock().unwrap().trace.clear();
                    w.run(&args).unwrap();
                    assert_eq!(w.0.lock().unwrap().prs[&101]["body"], body);
                    assert!(!w.trace().iter().any(content_mutation));
                }
            }
        }
    }
}

#[test]
fn full_stack_preserves_other_author_and_draft_context_without_extra_publication() {
    for native in [false, true] {
        for compact in [false, true] {
            let w = World::new(4);
            {
                let mut memory = w.0.lock().unwrap();
                memory.commits[1].author_email = "other@example.com".into();
                memory.commits[2].title = "WIP: pending draft".into();
            }
            let args = [
                if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                if compact {
                    "--compact-stack"
                } else {
                    "--compact-stack=false"
                },
                "--skip-draft",
                "--yes",
            ];
            w.run(&args).unwrap();
            {
                let memory = w.0.lock().unwrap();
                assert_eq!(memory.prs.len(), 2);
                for pr in memory.prs.values() {
                    let body = pr["body"].as_str().unwrap();
                    assert_eq!(body.contains("Feature 2"), !compact);
                    assert_eq!(body.contains("WIP: pending draft"), !compact);
                    assert_eq!(
                        body.contains(&format!("/commit/{}", commit(2).hash)),
                        !compact
                    );
                    assert_eq!(
                        body.contains(&format!("/commit/{}", commit(3).hash)),
                        !compact
                    );
                    assert!(!body.contains("#0"));
                }
                let push = memory
                    .trace
                    .iter()
                    .find(|event| event["args"][0] == "push")
                    .unwrap();
                let pushed = push["args"].as_array().unwrap();
                assert!(pushed.iter().any(|arg| {
                    arg.as_str()
                        .is_some_and(|arg| arg.ends_with("refs/heads/topic/1"))
                }));
                assert!(pushed.iter().any(|arg| {
                    arg.as_str()
                        .is_some_and(|arg| arg.ends_with("refs/heads/topic/4"))
                }));
                assert!(!pushed.iter().any(|arg| {
                    arg.as_str().is_some_and(|arg| {
                        arg.ends_with("refs/heads/topic/2") || arg.ends_with("refs/heads/topic/3")
                    })
                }));
                assert!(!memory.trace.iter().any(|event| event["path"] == "pulls/0"));
                drop(memory);
            }
            w.0.lock().unwrap().trace.clear();
            w.run(&args).unwrap();
            let trace = w.trace();
            assert!(!trace.iter().any(content_mutation));
            assert_eq!(
                trace
                    .iter()
                    .filter(|event| event["args"][0] == "push")
                    .count(),
                1
            );
        }
    }
}

#[test]
fn native_reconciliation_propagates_not_found_at_every_request_boundary() {
    for path in [
        "GET pulls/101",
        "GET stacks/9",
        "POST stacks/9/add",
        "POST stacks/9/unstack",
        "POST stacks",
    ] {
        let w = World::new(2);
        w.run(&["--no-stack"]).unwrap();
        let rebuild = matches!(path, "POST stacks/9/unstack" | "POST stacks");
        {
            let mut m = w.0.lock().unwrap();
            let pr = m.prs.get_mut(&101).unwrap();
            pr["stack"] = serde_json::json!({"number":9});

            m.trace.clear();
            m.stdout.clear();
            m.stderr.clear();
        }
        if rebuild {
            w.http("GET stacks/9", vec![crate::tests::unit::support::Response::Output(200, serde_json::json!({"number":9,"base":{"ref":"stale"},"pull_requests":[{"number":101},{"number":102}]}).to_string())]);
        }
        let mut config = w.config();
        config.github_stack = true;
        config.auto_accept = true;
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1), commit(2)]);
        s.stack[0].pr_number = 101;
        s.stack[1].pr_number = 102;
        w.http(
            path,
            vec![crate::tests::unit::support::Response::Output(
                404,
                "Missing resource".into(),
            )],
        );
        let error = super::Application::native_stack(
            &config,
            &github,
            &s,
            &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk),
        )
        .unwrap_err();
        assert_eq!(crate::diagnostics::errors::status(&error), 1, "{path}");
        assert_eq!(
            error
                .downcast_ref::<crate::github::http::HttpStatusError>()
                .unwrap()
                .status
                .as_u16(),
            404,
            "{path}"
        );
        let m = w.0.lock().unwrap();
        assert!(!m.stderr.contains("Native stacks are unavailable"));
        assert!(!m.stdout.contains("GitHub Stack updated"));
        assert_eq!(m.prs.len(), 2);
        if path == "POST stacks" {
            assert!(m.trace.iter().any(|v| v["path"] == "stacks/9/unstack"));
            assert!(m.prs[&101]["stack"].is_null());
        }
        drop(m);
    }
}

#[test]
fn presentation_handles_unknown_authors_and_each_pr_status_without_io() {
    let w = World::new(0);
    let mut config = w.config();
    let app = w.app();
    let mut unknown = commit(1);
    unknown.author_email.clear();
    let mut s = state(vec![unknown, commit(2)]);
    app.filter_authors(&config, &mut s);
    assert!(s.stack[0].skip);
    assert!(w.0.lock().unwrap().stdout.contains("@unknown"));
    config.stack_up = false;
    app.print_results(
        &config,
        &[
            commit(1),
            Commit {
                newly_created: true,
                ..commit(2)
            },
            Commit {
                base_updated: true,
                ..commit(3)
            },
        ],
    );
    assert!(w.0.lock().unwrap().stdout.contains("(created)"));
    assert!(w.0.lock().unwrap().stdout.contains("(updated)"));
    validate_duplicates(&[], &w.commands()).unwrap();
}

#[test]
fn missing_refs_fail_before_validation_push_or_pr_mutation() {
    let w = World::new(0);
    let config = w.config();
    let app = w.app();
    let mut c = commit(1);
    c.attrs.clear();
    let mut s = state(vec![c]);
    let git = Git {
        commands: &app.commands,
        repo: &config.repo,
    };
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    w.0.lock().unwrap().trace.clear();
    let error = app
        .push(
            &config,
            git,
            &github,
            &mut s,
            &super::pull_requests::Publication::default(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("no Remote-Ref"));
    assert_eq!(w.trace(), Vec::<serde_json::Value>::new());
}

#[test]
fn restored_selection_preserves_drafts_and_skips_even_when_identities_change() {
    let w = World::new(2);
    let mut config = w.config();
    config.skip_draft = true;
    config.draft_patterns = crate::stack::selection::DraftPatterns::new(&["wip:*".into()]).unwrap();
    let mut s = state(vec![
        commit(1),
        Commit {
            title: "WIP: pending".into(),
            ..commit(2)
        },
    ]);
    config.repo.branchless = true;
    s.stack[0].attrs.clear();
    let reread = [
        Commit {
            hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ..commit(1)
        },
        Commit {
            hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            title: "WIP: pending".into(),
            ..commit(2)
        },
    ];
    w.reply("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..HEAD", 0, &log(&reread));
    s.rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
        .unwrap();
    assert_eq!(s.stack[0].hash, reread[0].hash);
    assert!(!s.stack[0].skip && s.stack[1].skip);
}

#[test]
fn jujutsu_conflicts_observed_after_rewriting_stop_publication() {
    let world = World::new(2);
    {
        let mut memory = world.0.lock().unwrap();
        memory.jj = true;
        memory.commits[1].attrs.clear();
    }
    let mut config = world.config();
    config.repo.jj = true;
    let commands = world.commands();
    let mut selected = State::discover(&mut config, &commands).unwrap();
    assert!(selected.stack.iter().all(|commit| !commit.conflicted));
    world.0.lock().unwrap().trace.clear();
    let error = selected
        .rewrite(&config, &commands, world.as_ref(), &mut |_, _| {
            // Model a concurrent backend change after the initial observation.
            world.0.lock().unwrap().commits[1].conflicted = true;
            Ok(())
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("unresolved Jujutsu conflicts"));
    let trace = world.trace();
    assert!(
        trace
            .iter()
            .any(|event| event["program"] == "jj" && event["args"][0] == "describe")
    );
    assert!(!trace.iter().any(|event| event["args"][0] == "push"));
    assert!(world.0.lock().unwrap().prs.is_empty());
}

#[test]
fn jujutsu_selection_survives_rewriting_and_reports_failed_identity_recovery() {
    for args in [
        vec!["topic/2", "topic/3"],
        vec!["topic/1", "topic/3"],
        vec!["topic/1..topic/3"],
    ] {
        let w = World::new(3);
        {
            let mut m = w.0.lock().unwrap();
            m.jj = true;
            m.commits[2].attrs.clear();
        }
        w.run(&args).unwrap();
        assert!(
            w.trace()
                .iter()
                .any(|v| v["program"] == "jj" && v["args"][0] == "describe")
        );
    }
    for id in ["change1", "change2"] {
        let w = World::new(2);
        {
            let mut m = w.0.lock().unwrap();
            m.jj = true;
            for c in &mut m.commits {
                c.attrs.clear();
            }
        }
        let mut config = w.config();
        config.selection.explicit = true;
        config.selection.base = format!("{:040x}", 0);
        config.selection.tip = commit(2).hash;
        config.selection.selected = vec![commit(1).hash, commit(2).hash];
        let mut s = State::discover(&mut config, &w.commands()).unwrap();
        if id == "change1" {
            s.selected_changes[0] = "missing".into();
        } else {
            w.fail(&format!(
                "jj --ignore-working-copy log -r {id} --no-graph -T commit_id ++ \"\\n\""
            ));
        }
        assert!(
            s.rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
                .is_err()
        );
    }
}

#[test]
fn explicit_selection_requires_a_base_on_the_visible_path() {
    let w = World::new(2);
    let mut config = w.config();
    config.selection.explicit = true;
    config.selection.base = "short".into();
    config.selection.tip = commit(2).hash;
    let error = State::discover(&mut config, &w.commands()).unwrap_err();
    assert!(error.to_string().contains("short..00000000"));
    config.selection.selected = vec![commit(1).hash, "missing".into()];
    assert!(State::discover(&mut config, &w.commands()).is_err());
}

#[test]
fn merged_history_and_pr_completion_are_covered_without_submission_io() {
    let w = World::new(2);
    w.run(&["--no-stack"]).unwrap();
    {
        let mut m = w.0.lock().unwrap();
        for pr in m.prs.values_mut() {
            pr["merged"] = serde_json::json!(true);
        }
    }
    let mut config = w.config();
    config.emoji = true;
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    let mut s = state(vec![commit(1), commit(2)]);
    s.fill_numbers(
        &github,
        &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk),
    )
    .unwrap();
    assert!(super::pull_requests::Publication::observe(&github, &s.stack, 0).is_err());
    let (_, merged) = super::pull_requests::description_history(
        &config,
        &github,
        &s.stack.iter().collect::<Vec<_>>(),
        &mut crate::stack::description::BodyCache::default(),
    )
    .unwrap();
    assert_eq!(merged.len(), 2);
    s.stack[0].pr_number = 0;
    s.stack[0].skip = true;
    w.0.lock().unwrap().prs.get_mut(&101).unwrap()["state"] = serde_json::json!("closed");
    let mut selected_config = config;
    selected_config.selection.explicit = true;
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &selected_config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    w.http(
        "POST graphql",
        vec![crate::tests::unit::support::Response::Output(200, serde_json::json!({"data":{"repository":{"p0":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}}).to_string())],
    );
    s.fill_numbers(
        &github,
        &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk),
    )
    .unwrap();
    assert_eq!(s.stack[0].pr_number, 101);
    s.stack[0].pr_number = 0;
    let github = GitHub {
        session: crate::github::Session::default(),
        ..github
    };
    w.http(
        &format!("GET commits/{}/pulls", s.stack[0].hash),
        vec![crate::tests::unit::support::Response::Failure(
            "lookup failure".into(),
        )],
    );
    assert!(
        s.fill_numbers(
            &github,
            &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk)
        )
        .is_err()
    );
}

#[test]
fn creation_base_labels_and_checkout_failures_stop_later_work() {
    use crate::tests::unit::support::Response;
    let w = World::new(1);
    w.fail("git checkout topic/1");
    w.run(&["--no-stack"]).unwrap();
    assert!(!w.trace().iter().any(|v| v["args"][0] == "checkout"));
    for endpoint in ["POST pulls", "POST issues/101/labels"] {
        let w = World::new(1);
        w.http(endpoint, vec![Response::Failure("rejected".into())]);
        assert!(w.run(&["--no-stack", "--add-label=required"]).is_err());
    }
    let w = World::new(2);
    w.run(&[]).unwrap();
    w.0.lock().unwrap().prs.get_mut(&102).unwrap()["base"]["ref"] = serde_json::json!("stale");
    w.http(
        "PATCH pulls/102",
        vec![Response::Failure("base rejected".into())],
    );
    assert!(w.run(&[]).is_err());
}

#[test]
fn worker_panics_are_joined_and_stop_the_submission_phase() {
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let result = crate::commands::parallel(vec![0, 1, 2], |i| {
        completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert!(i != 1, "injected worker failure");
        Ok(i)
    });
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("worker terminated")
    );
    assert_eq!(completed.load(std::sync::atomic::Ordering::SeqCst), 3);
    let completed = std::sync::atomic::AtomicUsize::new(0);
    let result = crate::commands::parallel((0..20).collect(), |i| {
        completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if i == 0 {
            anyhow::bail!("rejected");
        }
        Ok(i)
    });
    assert!(result.is_err());
    assert_eq!(completed.load(std::sync::atomic::Ordering::SeqCst), 8);
}

#[test]
fn discovery_and_reread_errors_preserve_selection_context() {
    let log_key = "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..HEAD";
    for key in [
        log_key.to_owned(),
        format!("git rev-list --count {}..HEAD", commit(1).hash),
    ] {
        let w = World::new(2);
        w.0.lock().unwrap().commits[0].attrs.clear();
        let mut config = w.config();
        config.selection.selected = vec![commit(1).hash, commit(2).hash];
        if key.starts_with("git rev-list") {
            let commits = w.0.lock().unwrap().commits.clone();
            w.reply(
                log_key,
                0,
                &log(&commits).replace(&format!("\0{}\0", commit(1).hash), "\0unknown\0"),
            );
        }
        w.fail(&key);
        assert!(State::discover(&mut config, &w.commands()).is_err());
    }
    for key in ["git rev-parse --verify HEAD~0^{commit}", log_key] {
        let w = World::new(2);
        let mut config = w.config();
        config.repo.branchless = true;
        config.selection.explicit = true;
        config.selection.selected = vec![commit(1).hash];
        let mut s = state(vec![commit(1), commit(2)]);
        s.base.clone_from(&s.trunk_hash);
        s.stack[0].attrs.clear();
        s.selected_depths = vec![1];
        w.fail(key);
        if key == log_key {
            w.reply("git rev-parse --verify HEAD~0^{commit}", 0, "HEAD");
        }
        assert!(
            s.rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
                .is_err(),
            "{key}"
        );
    }
}

#[test]
fn pr_reads_and_native_link_failures_stop_after_completed_effects() {
    let w = World::new(2);
    w.http(
        &format!("GET commits/{}/pulls", commit(1).hash),
        vec![crate::tests::unit::support::Response::Failure(
            "ancestor lookup failed".into(),
        )],
    );
    assert!(w.run(&["topic/1..topic/2"]).is_err());
    assert!(!w.trace().iter().any(|v| v["method"] == "PATCH"));
    let w = World::new(2);
    w.http(
        "POST stacks",
        vec![crate::tests::unit::support::Response::Failure(
            "link failed".into(),
        )],
    );
    assert!(w.run(&["--yes", "--github-stack"]).is_err());
    assert_eq!(
        w.trace()
            .iter()
            .filter(|v| v["body"]["query"]
                .as_str()
                .is_some_and(|q| q.starts_with("mutation")))
            .count(),
        1
    );
    let w = World::new(1);
    let config = w.config();
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    let mut s = state(vec![commit(1)]);
    w.http(
        "POST graphql",
        vec![crate::tests::unit::support::Response::Failure(
            "find failed".into(),
        )],
    );
    assert!(
        s.fill_numbers(
            &github,
            &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk)
        )
        .is_err()
    );
    s.stack[0].pr_number = 101;
    w.http(
        "GET pulls/101",
        vec![
            crate::tests::unit::support::Response::Output(200, "{}".into()),
            crate::tests::unit::support::Response::Failure("target read failed".into()),
        ],
    );
    let git = Git {
        commands: &app.commands,
        repo: &config.repo,
    };
    assert!(
        crate::submission::Application::update_descriptions(&config, git, &github, &s).is_err()
    );
}

#[test]
fn descendant_discovery_accepts_missing_refs_but_propagates_failed_lookups() {
    for (missing, failed_log) in [(false, false), (true, false), (false, true)] {
        let w = World::new(2);
        w.run(&["--no-stack"]).unwrap();
        let config = w.config();
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1)]);
        s.stack[0].pr_number = 101;
        w.reply(
            "git for-each-ref --contains HEAD --format=%(objectname) refs/heads/",
            0,
            &commit(2).hash,
        );
        if missing {
            w.0.lock().unwrap().commits[1].attrs.clear();
        }
        if failed_log {
            w.fail(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..{}", commit(2).hash));
        }
        if !missing {
            w.http(
                "POST graphql",
                vec![crate::tests::unit::support::Response::Failure(
                    "descendant lookup failed".into(),
                )],
            );
        }
        w.0.lock().unwrap().trace.clear();
        let git = Git {
            commands: &app.commands,
            repo: &config.repo,
        };
        assert_eq!(
            crate::submission::Application::update_descriptions(&config, git, &github, &s).is_ok(),
            missing
        );
        if missing {
            assert!(!w.trace().iter().any(|event| {
                event["path"] == "graphql"
                    && event["body"]["query"]
                        .as_str()
                        .unwrap()
                        .starts_with("query")
            }));
        }
    }
}

#[test]
fn invalid_selection_recovery_and_failed_rewrite_observations_stop_publication() {
    let w = World::new(2);
    let mut config = w.config();
    config.repo.jj = true;
    w.fail("jj --ignore-working-copy log -r @- --no-graph -T commit_id");
    assert!(State::discover(&mut config, &w.commands()).is_err());
    let w = World::new(2);
    let mut config = w.config();
    config.repo.jj = true;
    config.selection.selected = vec![commit(1).hash];
    w.fail(r#"jj --ignore-working-copy log -r heads(@- | (@ ~ empty())) --no-graph -T commit_id ++ "\n""#);
    assert!(State::discover(&mut config, &w.commands()).is_err());
    let w = World::new(2);
    let mut config = w.config();
    config.repo.jj = true;
    config.selection.explicit = true;
    config.selection.tip = "tip".into();
    config.selection.base = "base".into();
    w.fail("jj --ignore-working-copy log -r tip --no-graph -T change_id");
    assert!(State::discover(&mut config, &w.commands()).is_err());
    let mut c = commit(1);
    c.attrs.clear();
    config.selection.selected = vec!["missing".into()];
    let commands = w.commands();
    let git = Git {
        commands: &commands,
        repo: &config.repo,
    };
    assert!(
        State::capture_selection(
            &[c],
            &std::collections::HashMap::new(),
            &config,
            git,
            "HEAD"
        )
        .is_err()
    );
    for key in [
        "git check-ref-format --branch topic/1",
        "jj --ignore-working-copy log -r @- --no-graph -T commit_id",
    ] {
        let w = World::new(1);
        w.0.lock().unwrap().commits[0].attrs.clear();
        let mut config = w.config();
        config.repo.branchless = true;
        config.repo.jj = key.starts_with("jj ");
        let mut s = state(w.0.lock().unwrap().commits.clone());
        if key == "jj --ignore-working-copy log -r @- --no-graph -T commit_id" {
            s.validated_refs.insert("topic/1".into());
        }
        if key.starts_with("git check-ref-format") {
            w.reply(
                "git branch --format=%(refname:strip=2)|%(objectname)",
                0,
                &format!("invalid..ref|{}", commit(1).hash),
            );
        } else {
            w.fail(key);
        }
        assert!(
            s.rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
                .is_err()
        );
    }
}

#[test]
fn publication_and_description_failures_cannot_cross_completion_barriers() {
    use crate::tests::unit::support::Response;
    let w = World::new(1);
    let config = w.config();
    let app = w.app();
    let commands = &app.commands;
    let git = Git {
        commands,
        repo: &config.repo,
    };
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands,
        transport: w.as_ref(),
        template: "",
    };
    let mut s = state(vec![commit(1)]);
    app.reconcile_prs(
        &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk),
        &github,
        &mut s,
        &[],
    )
    .unwrap();
    s.stack[0].attrs[0].1 = "invalid..ref".into();
    assert!(
        app.push(
            &config,
            git,
            &github,
            &mut s,
            &super::pull_requests::Publication::default()
        )
        .is_err()
    );
    s.stack[0].attrs[0].1 = "topic/1".into();
    w.http(
        "POST graphql",
        vec![Response::Failure("observation failed".into())],
    );
    assert!(
        app.push(
            &config,
            git,
            &github,
            &mut s,
            &super::pull_requests::Publication::default()
        )
        .is_err()
    );
    assert!(
        app.reconcile_prs(
            &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk),
            &github,
            &mut s,
            &[0]
        )
        .is_err()
    );
    let mut invalid_config = config.clone();
    invalid_config.repo.name.clear();
    let expired = GitHub {
        config: &invalid_config,
        session: crate::github::Session::default(),
        ..github
    };
    assert!(
        crate::submission::Application::update_descriptions(&config, git, &expired, &s).is_err()
    );
    w.0.lock().unwrap().http_responses.clear();
    w.run(&["--no-stack"]).unwrap();
    let github = GitHub {
        session: crate::github::Session::default(),
        config: &config,
        ..expired
    };
    s.stack[0].pr_number = 0;
    assert!(
        crate::submission::Application::update_descriptions(&config, git, &github, &s).is_err()
    );
    s.stack[0].pr_number = 101;
    s.stack[0].title = "Changed".into();
    w.http(
        "POST graphql",
        vec![Response::Failure("content update failed".into())],
    );
    assert!(
        crate::submission::Application::update_descriptions(&config, git, &github, &s).is_err()
    );
}

#[test]
fn failed_depth_recovery_cannot_silently_change_an_explicit_selection() {
    let w = World::new(1);
    let mut config = w.config();
    let mut c = commit(1);
    c.attrs.clear();
    config.selection.selected = vec![c.hash.clone()];
    w.fail(&format!("git rev-list --count {}..HEAD", c.hash));
    let commands = w.commands();
    let git = Git {
        commands: &commands,
        repo: &config.repo,
    };
    assert!(
        State::capture_selection(
            std::slice::from_ref(&c),
            &std::collections::HashMap::new(),
            &config,
            git,
            "HEAD"
        )
        .is_err()
    );
    w.reply(&format!("git rev-list --count {}..HEAD", c.hash), 0, "2");
    assert_eq!(
        State::capture_selection(
            std::slice::from_ref(&c),
            &std::collections::HashMap::new(),
            &config,
            git,
            "HEAD"
        )
        .unwrap()
        .0,
        vec![2]
    );
    let depths = std::collections::HashMap::from([(c.hash.clone(), 3)]);
    assert_eq!(
        State::capture_selection(&[c], &depths, &config, git, "HEAD")
            .unwrap()
            .0,
        vec![3]
    );
}

#[test]
fn descendant_description_context_is_observed_in_batches() {
    let w = World::new(27);
    w.run(&["--no-stack"]).unwrap();
    let config = w.config();
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    let mut s = state(vec![commit(1)]);
    s.stack[0].pr_number = 101;
    w.reply(
        "git for-each-ref --contains HEAD --format=%(objectname) refs/heads/",
        0,
        &commit(27).hash,
    );
    w.0.lock().unwrap().trace.clear();
    let git = Git {
        commands: &app.commands,
        repo: &config.repo,
    };
    crate::submission::Application::update_descriptions(&config, git, &github, &s).unwrap();
    let count = w
        .trace()
        .iter()
        .filter(|e| {
            e["path"] == "graphql"
                && e["body"]["query"]
                    .as_str()
                    .is_some_and(|q| q.starts_with("query"))
        })
        .count();
    assert_eq!(count, 2);
    assert!(
        w.0.lock().unwrap().prs[&101]["body"]
            .as_str()
            .unwrap()
            .contains("#127")
    );
}

#[test]
fn failed_descendant_read_preserves_open_pr_history() {
    for failure in [
        "git show --no-show-signature -s --format=%H %T HEAD".to_owned(),
        "git for-each-ref --contains HEAD --format=%(objectname) refs/heads/".to_owned(),
        format!("git merge-base --independent {}", commit(2).hash),
        format!(
            "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature HEAD..{}",
            commit(2).hash
        ),
    ] {
        let w = World::new(2);
        w.0.lock()
            .unwrap()
            .config
            .insert("git-pr.stackEmoji".into(), "true".into());
        w.run(&["--no-stack"]).unwrap();
        let mut config = w.config();
        config.emoji = true;
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1)]);
        s.stack[0].pr_number = 101;
        w.reply(
            "git for-each-ref --contains HEAD --format=%(objectname) refs/heads/",
            0,
            &commit(2).hash,
        );
        w.fail(&failure);
        w.0.lock().unwrap().trace.clear();
        let git = Git {
            commands: &app.commands,
            repo: &config.repo,
        };
        assert!(
            crate::submission::Application::update_descriptions(&config, git, &github, &s).is_err()
        );
        let m = w.0.lock().unwrap();
        assert_eq!(m.prs[&102]["state"], "open");
        let body = m.prs[&101]["body"].as_str().unwrap();
        assert!(!body.contains("* ✔️ #102"), "{body}");
        assert!(!m.trace.iter().any(|e| {
            e["method"] == "PATCH"
                || e["body"]["query"]
                    .as_str()
                    .is_some_and(|q| q.starts_with("mutation"))
        }));
        drop(m);
    }
}

#[test]
fn partial_history_uses_observed_merge_status_and_preserves_bodies_on_failure() {
    for (emoji, merged, closed, missing) in [
        (true, false, false, false),
        (true, true, true, false),
        (true, false, true, false),
        (true, false, false, true),
        (false, false, false, true),
    ] {
        let w = World::new(2);
        w.run(&["--no-stack"]).unwrap();
        let mut config = w.config();
        config.emoji = emoji;
        config.selection.explicit = true;
        {
            let mut m = w.0.lock().unwrap();
            let body = m.prs[&101]["body"]
                .as_str()
                .unwrap()
                .replace("* #102", "* ✔️ #102");
            m.prs.get_mut(&101).unwrap()["body"] = serde_json::json!(body);
            m.prs.get_mut(&102).unwrap()["merged"] = serde_json::json!(merged);
            m.prs.get_mut(&102).unwrap()["state"] =
                serde_json::json!(if closed { "closed" } else { "open" });
            if missing {
                m.prs.remove(&102);
            }
        }
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut s = state(vec![commit(1)]);
        s.stack[0].pr_number = 101;
        let git = Git {
            commands: &app.commands,
            repo: &config.repo,
        };
        let before = w.0.lock().unwrap().prs[&101]["body"].clone();
        w.0.lock().unwrap().trace.clear();
        let result = crate::submission::Application::update_descriptions(&config, git, &github, &s);
        let m = w.0.lock().unwrap();
        if missing && emoji {
            assert!(result.is_err());
            assert_eq!(m.prs[&101]["body"], before);
            assert!(!m.trace.iter().any(|e| {
                e["method"] == "PATCH"
                    || e["body"]["query"]
                        .as_str()
                        .is_some_and(|q| q.starts_with("mutation"))
            }));
        } else {
            result.unwrap();
            let body = m.prs[&101]["body"].as_str().unwrap();
            assert!(body.contains("#102"));
            assert_eq!(body.contains("* ✔️ #102"), emoji && merged);
        }
        drop(m);
    }
}

#[test]
fn explicit_context_reuses_a_known_open_pr() {
    let w = World::new(2);
    w.run(&["--no-stack"]).unwrap();
    for pr in w.0.lock().unwrap().prs.values_mut() {
        pr["body"] = serde_json::json!("Human description");
    }
    let mut config = w.config();
    config.selection.explicit = true;
    config.emoji = true;
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    github
        .observe(&["topic/1".into(), "topic/2".into()])
        .unwrap();
    assert_eq!(github.observed_head("topic/1", "open"), 101);
    let mut s = state(vec![commit(1), commit(2)]);
    s.start = 1;
    s.stack[1].pr_number = 102;
    w.http(
        &format!("GET commits/{}/pulls", s.stack[0].hash),
        vec![crate::tests::unit::support::Response::Output(
            200,
            "[]".into(),
        )],
    );
    s.fill_numbers(
        &github,
        &crate::stack::selection::resolve_bases(&s.stack, s.start, &github.config.repo.trunk),
    )
    .unwrap();
    assert_eq!(s.stack[0].pr_number, 101);
    let git = Git {
        commands: &app.commands,
        repo: &config.repo,
    };
    super::Application::update_descriptions(&config, git, &github, &s).unwrap();
    let body = w.0.lock().unwrap().prs[&102]["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(body.contains("#101"), "{body}");
    assert!(!w.trace().iter().any(|e| {
        e["path"]
            .as_str()
            .is_some_and(|p| p.starts_with("commits/"))
    }));
}

#[test]
fn selected_rewrite_reuses_snapshot_ranks_even_with_a_diverged_trunk() {
    for (tip_depth, diverged, missing) in [
        (0, false, false),
        (1, false, false),
        (0, true, false),
        (0, false, true),
        (1, false, true),
    ] {
        let w = World::new(100);
        let mut config = w.config();
        config.repo.branchless = true;
        config.selection.explicit = true;
        config.selection.selected = (1..=100 - tip_depth).map(|n| commit(n).hash).collect();
        let mut s = state((1..=100 - tip_depth).map(commit).collect());
        s.base.clone_from(&s.trunk_hash);
        for c in &mut s.stack {
            c.attrs.clear();
        }
        s.tip_depth = usize::try_from(tip_depth).unwrap();
        s.selected_depths = (s.tip_depth..100).collect();
        if missing {
            s.selected_depths.push(if tip_depth == 0 { 101 } else { 0 });
        }
        if diverged {
            s.base.clone_from(&s.trunk_hash);
            w.reply(
                "git show --no-show-signature -s --format=%H %T refs/remotes/origin/main",
                0,
                "diverged tree0",
            );
        }
        w.0.lock().unwrap().trace.clear();
        let result = s.rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()));
        if missing {
            assert!(format!("{:#}", result.unwrap_err()).contains("selected commit disappeared"));
        } else {
            result.unwrap();
            assert_eq!(
                s.selected().iter().filter(|c| !c.skip).count(),
                100 - usize::try_from(tip_depth).unwrap()
            );
            let reads = w
                .trace()
                .iter()
                .filter(|v| v["program"] == "git" && v["args"][0] == "rev-parse")
                .count();
            assert_eq!(reads, 1);
        }
    }
}

#[test]
fn repeated_change_ids_cannot_select_an_arbitrary_rewritten_commit() {
    let w = World::new(3);
    {
        let mut m = w.0.lock().unwrap();
        m.jj = true;
        for c in &mut m.commits {
            c.attrs.clear();
        }
        m.commits[1].change_id = m.commits[0].change_id.clone();
    }
    let mut config = w.config();
    config.selection.explicit = true;
    config.selection.selected = (1..=3).map(|n| commit(n).hash).collect();
    let mut s = State::discover(&mut config, &w.commands()).unwrap();
    let error = s
        .rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
        .unwrap_err();
    assert!(error.to_string().contains("ambiguous change identities"));
    assert!(!w.trace().iter().any(|v| v["args"][0] == "push"));
}

#[test]
fn failed_batched_recovery_prevents_publication() {
    let w = World::new(2);
    let mut config = w.config();
    config.repo.branchless = true;
    config.selection.explicit = true;
    config.selection.selected = vec![commit(1).hash, commit(2).hash];
    let mut s = state(vec![commit(1), commit(2)]);
    s.base.clone_from(&s.trunk_hash);
    for c in &mut s.stack {
        c.attrs.clear();
    }
    s.selected_depths = vec![1, 0];
    let rewritten: Vec<_> = (1..=2)
        .map(|n| {
            let mut c = commit(n);
            c.hash = format!("b{n:039x}");
            c
        })
        .collect();
    w.reply(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..{}", rewritten[1].hash), 0,
        &log(&rewritten).replace(&format!("\0{}\0", rewritten[0].hash), "\0unknown\0"));
    w.fail("git rev-parse --revs-only --end-of-options HEAD~1^{commit} HEAD~0^{commit}");
    let error = s
        .rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
        .unwrap_err();
    assert!(format!("{error:#}").contains("unavailable"));
    assert!(!w.trace().iter().any(|v| v["args"][0] == "push"));
}

#[test]
fn explicit_selection_batches_unique_inputs_and_reuses_diverged_path_ranks() {
    for count in [100, 1000] {
        for diverged in [false, true] {
            let w = World::new(count);
            for c in &mut w.0.lock().unwrap().commits {
                c.attrs.clear();
            }
            if diverged {
                w.reply(
                    "git show --no-show-signature -s --format=%H %T refs/remotes/origin/main",
                    0,
                    "diverged tree0",
                );
            }
            let mut args = vec!["--stop-after=get-commits".to_owned()];
            for n in 1..=count {
                args.extend([format!("topic/{n}"), format!("topic/{n}")]);
            }
            w.run(&args.iter().map(String::as_str).collect::<Vec<_>>())
                .unwrap();
            let trace = w.trace();
            assert_eq!(
                trace
                    .iter()
                    .filter(|e| e["args"][0] == "rev-parse" && e["args"][1] == "--revs-only")
                    .count(),
                usize::try_from(count).unwrap().div_ceil(128)
            );
            assert!(!trace.iter().any(|e| e["args"][0] == "rev-list"));
            assert!(!trace.iter().any(|e| {
                e["args"][0] == "rev-parse"
                    && e["args"][1] == "--verify"
                    && e["args"][2]
                        .as_str()
                        .is_some_and(|arg| arg.starts_with("topic/"))
            }));
        }
    }
}

#[test]
fn inferred_trunk_and_duplicate_destinations_stop_before_rewriting() {
    for trunk in [false, true] {
        let w = World::new(2);
        let mut config = w.config();
        config.repo.branchless = true;
        let mut s = state(vec![commit(1), commit(2)]);
        s.stack[0].attrs[0].1 = "topic/2".into();
        s.stack[1].attrs.clear();
        if trunk {
            config.repo.trunk = "develop".into();
            w.reply(
                "git branch --format=%(refname:strip=2)|%(objectname)",
                0,
                &format!("develop|{}", commit(2).hash),
            );
        }
        let error = s
            .rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(if trunk { "remote trunk" } else { "duplicate" }),
            "{error:#}"
        );
        assert!(
            !w.trace()
                .iter()
                .any(|e| matches!(e["args"][0].as_str(), Some("push" | "reword" | "describe")))
        );
    }
}

#[test]
fn explicit_trunk_targets_and_failed_jujutsu_selection_reads_stop_publication() {
    let w = World::new(1);
    w.0.lock().unwrap().commits[0].attrs[0].1 = "main".into();
    assert!(format!("{:#}", w.run(&["--no-stack"]).unwrap_err()).contains("remote trunk"));
    assert!(
        !w.trace()
            .iter()
            .any(|e| matches!(e["args"][0].as_str(), Some("push" | "reword")))
    );
    let w = World::new(2);
    w.0.lock().unwrap().jj = true;
    w.fail("jj --ignore-working-copy log -r heads(@- | (@ ~ empty())) --no-graph -T commit_id ++ \"\\n\"");
    let error = w.run(&["topic/1", "topic/2"]).unwrap_err();
    assert!(format!("{error:#}").contains("unavailable"));
    assert!(!w.trace().iter().any(|e| e["args"][0] == "push"));
}

#[test]
fn rewrite_anchors_preserve_bases_and_reject_unverifiable_recovery() {
    for failure in ["", "position", "mismatch", "count", "anchor", "range"] {
        let w = World::new(2);
        w.0.lock().unwrap().commits[0].attrs.clear();
        let mut config = w.config();
        config.repo.branchless = true;
        config.selection.explicit = true;
        config.selection.base = format!("{:040x}", 0);
        config.selection.tip = commit(2).hash;
        let mut s = State::discover(&mut config, &w.commands()).unwrap();
        if failure != "anchor" {
            w.reply(
                "git branch --format=%(refname:strip=2)|%(objectname)",
                0,
                &format!("topic/1|{}", commit(1).hash),
            );
        }
        match failure {
            "position" => w.fail("git rev-parse --verify HEAD~0^{commit}"),
            "mismatch" => w.reply("git rev-parse --verify HEAD~0^{commit}", 0, &commit(1).hash),
            "count" => w.fail(&format!("git rev-list --count {}..HEAD", commit(2).hash)),
            "anchor" => w.fail("git rev-parse --verify refs/heads/topic/2^{commit}"),
            "range" => w.reply(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..b{:039x}", 2), 0, ""),
            _ => {},
        }
        let result = s.rewrite(&config, &w.commands(), w.as_ref(), &mut |_, _| Ok(()));
        if failure.is_empty() {
            result.unwrap();
            assert_eq!(s.base, config.selection.base);
            assert_eq!(s.selected().len(), 2);
        } else {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains(match failure {
                    "range" => "range disappeared",
                    "mismatch" => "current checkout",
                    _ => "unavailable",
                }),
                "{error}"
            );
        }
        assert!(!w.trace().iter().any(|e| e["args"][0] == "push"));
        if matches!(failure, "position" | "mismatch" | "count") {
            assert!(!w.trace().iter().any(|e| e["args"][0] == "reword"));
        }
    }
}

#[test]
fn merged_pr_observation_failures_are_not_treated_as_unmerged() {
    use crate::tests::unit::support::Response;
    for fail_head in [false, true] {
        let w = World::new(1);
        w.run(&["--no-stack"]).unwrap();
        let config = w.config();
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        if fail_head {
            w.http(
                "POST graphql",
                vec![Response::Failure("head observation failed".into())],
            );
        } else {
            assert_eq!(github.find_head("topic/1", "all").unwrap(), 101);
            github.expire_pr_observation_for_test(101);
            w.http(
                "POST graphql",
                vec![Response::Failure("merge status unavailable".into())],
            );
        }
        let diagnostic = format!(
            "{:#}",
            super::pull_requests::Publication::observe(&github, &[commit(1)], 0).unwrap_err()
        );
        assert!(diagnostic.contains(if fail_head {
            "head observation failed"
        } else {
            "merge status unavailable"
        }));
        assert!(!w.0.lock().unwrap().stdout.contains("merged PR(s) found"));
    }
}

#[test]
fn merged_preflight_keeps_expired_observations_batched() {
    for expire_heads in [true, false] {
        let w = World::new(51);
        w.run(&["--no-stack"]).unwrap();
        let config = w.config();
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let commits: Vec<_> = (1..=51).map(commit).collect();
        github
            .observe(
                &commits
                    .iter()
                    .map(|c| c.remote_ref().to_owned())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        for commit in &commits {
            if expire_heads {
                github.expire_head_observation_for_test(commit.remote_ref());
            }
        }
        if !expire_heads {
            for number in 101..=151 {
                github.expire_pr_observation_for_test(number);
            }
        }
        w.0.lock().unwrap().trace.clear();
        super::pull_requests::Publication::observe(&github, &commits, 0).unwrap();
        let requests: Vec<_> = w
            .trace()
            .into_iter()
            .filter(|event| event["kind"] == "http")
            .collect();
        assert_eq!(requests.len(), 6);
        assert!(requests.iter().all(|event| event["path"] == "graphql"));
        assert!(
            requests
                .iter()
                .all(|event| event["body"]["variables"].as_object().unwrap().len() <= 52)
        );
        if expire_heads {
            w.0.lock().unwrap().trace.clear();
            super::pull_requests::Publication::observe(&github, &commits, 0).unwrap();
            assert_eq!(
                w.trace()
                    .iter()
                    .filter(|event| event["kind"] == "http")
                    .count(),
                3
            );
        }
    }
}

#[test]
fn merged_preflight_uses_the_refreshed_head_identity() {
    let w = World::new(1);
    w.run(&["--no-stack"]).unwrap();
    w.0.lock().unwrap().prs.get_mut(&101).unwrap()["merged"] = serde_json::json!(true);
    let config = w.config();
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    assert!(super::pull_requests::Publication::observe(&github, &[commit(1)], 0).is_err());
    {
        let mut memory = w.0.lock().unwrap();
        memory.prs.get_mut(&101).unwrap()["state"] = serde_json::json!("closed");
        let mut replacement = memory.prs[&101].clone();
        replacement["number"] = serde_json::json!(201);
        replacement["state"] = serde_json::json!("open");
        replacement["merged"] = serde_json::json!(false);
        memory.prs.insert(201, replacement);
        memory.stdout.clear();
        memory.trace.clear();
    }
    github.expire_head_observation_for_test("topic/1");
    super::pull_requests::Publication::observe(&github, &[commit(1)], 0).unwrap();
    assert_eq!(w.0.lock().unwrap().stdout, "");
    assert_eq!(github.observed_head("topic/1", "all"), 201);
    assert_eq!(w.trace().iter().filter(|e| e["kind"] == "http").count(), 2);
}

#[test]
fn description_history_refreshes_expired_observations_in_bounded_batches() {
    for expire_heads in [false, true] {
        let w = World::new(51);
        w.run(&["--no-stack"]).unwrap();
        let config = w.config();
        let app = w.app();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &app.commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut commits: Vec<_> = (1..=51).map(commit).collect();
        for (i, commit) in commits.iter_mut().enumerate() {
            commit.pr_number = 101 + u64::try_from(i).unwrap();
        }
        github
            .observe(
                &commits
                    .iter()
                    .map(|c| c.remote_ref().to_owned())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        for commit in &commits {
            github.expire_pr_observation_for_test(commit.pr_number);
            if expire_heads {
                github.expire_head_observation_for_test(commit.remote_ref());
            }
        }
        {
            let mut memory = w.0.lock().unwrap();
            memory.prs.get_mut(&101).unwrap()["body"] = serde_json::json!(
                "Updated human text\n<!-- git-pr-stack-start -->\n* #77\n<!-- git-pr-stack-end -->"
            );
            memory.prs.get_mut(&151).unwrap()["merged"] = serde_json::json!(true);
            memory.trace.clear();
        }
        let mut bodies = crate::stack::description::BodyCache::default();
        let (history, merged) = super::pull_requests::description_history(
            &config,
            &github,
            &commits.iter().collect::<Vec<_>>(),
            &mut bodies,
        )
        .unwrap();
        assert!(history.iter().any(|entry| entry.number == 77));
        assert!(merged.contains(&151));
        assert!(
            github
                .get(101)
                .unwrap()
                .body
                .starts_with("Updated human text")
        );
        let requests: Vec<_> = w
            .trace()
            .into_iter()
            .filter(|event| event["kind"] == "http")
            .collect();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|event| event["path"] == "graphql"
            && event["body"]["variables"].as_object().unwrap().len() <= 27));
        w.0.lock().unwrap().trace.clear();
        super::pull_requests::description_history(
            &config,
            &github,
            &commits.iter().collect::<Vec<_>>(),
            &mut bodies,
        )
        .unwrap();
        assert!(!w.trace().iter().any(|event| event["kind"] == "http"));
    }
}

#[test]
fn description_history_retains_original_identities_and_refresh_failures() {
    let w = World::new(1);
    w.run(&["--no-stack"]).unwrap();
    let config = w.config();
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    let mut commit = commit(1);
    commit.pr_number = 101;
    github.observe(&[commit.remote_ref().to_owned()]).unwrap();
    github.expire_pr_observation_for_test(101);
    w.http(
        "POST graphql",
        vec![crate::tests::unit::support::Response::Failure(
            "history network unavailable".into(),
        )],
    );
    w.0.lock().unwrap().trace.clear();
    let error = super::pull_requests::description_history(
        &config,
        &github,
        &[&commit],
        &mut crate::stack::description::BodyCache::default(),
    )
    .unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("failed to refresh PR stack history")
            && diagnostic.contains("history network unavailable")
    );
    assert!(
        !w.trace()
            .iter()
            .any(|event| event["method"] == "PATCH" || event["method"] == "GET")
    );
    {
        let mut memory = w.0.lock().unwrap();
        memory.http_responses.clear();
        memory.prs.get_mut(&101).unwrap()["head"]["ref"] = serde_json::json!("renamed");
        memory.prs.get_mut(&101).unwrap()["merged"] = serde_json::json!(true);
        memory.trace.clear();
    }
    let (_, merged) = super::pull_requests::description_history(
        &config,
        &github,
        &[&commit],
        &mut crate::stack::description::BodyCache::default(),
    )
    .unwrap();
    assert_eq!(merged, std::collections::HashSet::from([101]));
    let requests: Vec<_> = w
        .trace()
        .into_iter()
        .filter(|event| event["kind"] == "http")
        .collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["path"], "graphql");
    assert_eq!(requests[1]["path"], "pulls/101");
}

#[test]
fn explicit_jujutsu_selection_reuses_observed_tip_identities() {
    for selection_list in [false, true] {
        for rewrite in [false, true] {
            let w = World::new(3);
            {
                let mut memory = w.0.lock().unwrap();
                memory.jj = true;
                if rewrite {
                    for commit in &mut memory.commits {
                        commit.attrs.clear();
                    }
                }
            }
            let mut config = w.config();
            config.selection.explicit = true;
            config.selection.base = format!("{:040x}", 0);
            config.selection.tip = commit(3).hash;
            if selection_list {
                config.selection.selected = (1..=3).map(|n| commit(n).hash).collect();
            }
            w.fail(&format!(
                "jj --ignore-working-copy log -r {} --no-graph -T change_id",
                commit(3).hash
            ));
            w.0.lock().unwrap().trace.clear();
            let s = State::discover(&mut config, &w.commands()).unwrap();
            assert_eq!(s.tip_change_id, commit(3).change_id);
            assert_eq!(s.stack.len(), 3);
            let trace = w.trace();
            assert_eq!(
                trace
                    .iter()
                    .filter(|v| v["program"] == "jj" && v["args"][0] == "--at-operation")
                    .count(),
                1
            );
            assert!(!trace.iter().any(|v| v["program"] == "jj"
                && v["args"].as_array().unwrap().last().unwrap() == "change_id"));
        }
    }
}

#[test]
fn filtered_jujutsu_tip_keeps_its_observed_recovery_identity() {
    let w = World::new(3);
    w.0.lock().unwrap().jj = true;
    let mut config = w.config();
    config.selection.explicit = true;
    config.selection.base = format!("{:040x}", 0);
    config.selection.tip = commit(3).hash;
    let observation = log(&w.0.lock().unwrap().commits).replace("\0tree3\0", "\0tree2\0");
    w.reply(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..{}", config.selection.tip), 0, &observation);
    w.fail(&format!(
        "jj --ignore-working-copy log -r {} --no-graph -T change_id",
        config.selection.tip
    ));
    w.0.lock().unwrap().trace.clear();
    let state = State::discover(&mut config, &w.commands()).unwrap();
    assert_eq!(state.stack.len(), 2);
    assert_eq!(state.tip_change_id, commit(3).change_id);
    assert!(
        !w.trace().iter().any(|v| v["program"] == "jj"
            && v["args"].as_array().unwrap().last().unwrap() == "change_id")
    );
}

#[test]
fn unavailable_jujutsu_tip_observation_retains_a_fallible_identity_fallback() {
    for fail in [false, true] {
        let w = World::new(3);
        w.0.lock().unwrap().jj = true;
        let mut config = w.config();
        config.selection.explicit = true;
        config.selection.base = format!("{:040x}", 0);
        // Exercise the adapter fallback for an unresolved alias, rather than a
        // commit identity present in the structured history response.
        config.selection.tip = "tip".into();
        let key = "jj --ignore-working-copy log -r tip --no-graph -T change_id";
        if fail {
            w.fail(key);
        } else {
            w.reply(key, 0, &commit(3).change_id);
        }
        w.0.lock().unwrap().trace.clear();
        let result = State::discover(&mut config, &w.commands());
        assert_eq!(result.is_err(), fail);
        if fail {
            assert!(format!("{:#}", result.unwrap_err()).contains("unavailable"));
        } else {
            assert_eq!(result.unwrap().tip_change_id, commit(3).change_id);
        }
        assert_eq!(
            w.trace()
                .iter()
                .filter(|v| v["program"] == "jj"
                    && v["args"].as_array().unwrap().last().unwrap() == "change_id")
                .count(),
            1
        );
    }
}

#[test]
fn cached_absence_does_not_suppress_creation_when_filling_selected_identities() {
    let w = World::new(1);
    let config = w.config();
    let app = w.app();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &app.commands,
        transport: w.as_ref(),
        template: "",
    };
    github.observe(&["topic/1".into()]).unwrap();
    let mut selected = state(vec![commit(1)]);
    w.0.lock().unwrap().trace.clear();
    selected.fill_numbers(&github, &["main".into()]).unwrap();
    assert_eq!(selected.stack[0].pr_number, 101);
    assert!(selected.stack[0].newly_created);
    assert_eq!(
        w.trace()
            .iter()
            .filter(|v| v["method"] == "POST" && v["path"] == "pulls")
            .count(),
        1
    );
}

#[test]
fn rich_presentations_replace_only_default_lists_and_preserve_workflow_effects() {
    use crate::commands::Terminal;
    #[derive(Debug)]
    struct Plain(std::sync::Arc<World>);
    impl Terminal for Plain {
        fn out(&self, text: &str) {
            self.0.out(text);
        }
        fn err(&self, text: &str) {
            self.0.err(text);
        }
        fn confirm(&self, prompt: &str) -> bool {
            self.0.confirm(prompt)
        }
    }
    let plain = World::new(3);
    let mut app = plain.app();
    app.commands.terminal = std::sync::Arc::new(Plain(plain.clone()));
    app.execute(
        &crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap(),
        &std::collections::BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
    .unwrap();
    let rich = World::new(3);
    rich.0.lock().unwrap().rich_output = true;
    rich.run(&["--no-stack"]).unwrap();
    assert_eq!(plain.trace(), rich.trace());
    assert_eq!(plain.0.lock().unwrap().prs, rich.0.lock().unwrap().prs);
    let output = rich.0.lock().unwrap().stdout.clone();
    assert!(!output.contains("Stack of") && !output.contains(&commit(1).to_string()));
    rich.run(&["--no-stack", "--output={number}"]).unwrap();
    assert!(rich.0.lock().unwrap().stdout.contains("Stack of 3"));
}

#[test]
fn final_status_reports_content_and_label_changes_only_in_the_current_invocation() {
    for change in ["title", "body", "add label", "remove label"] {
        let w = World::new(2);
        w.run(&["--no-stack"]).unwrap();
        let mut flags = vec!["--no-stack"];
        {
            let mut memory = w.0.lock().unwrap();
            memory.stdout.clear();
            match change {
                "title" => memory.commits[0].title = "Changed title".into(),
                "body" => {
                    memory.prs.get_mut(&101).unwrap()["body"] =
                        serde_json::json!("Human text without the stack section");
                }
                "add label" => flags.push("--add-label=review"),
                "remove label" => {
                    memory.prs.get_mut(&101).unwrap()["labels"] =
                        serde_json::json!([{"name":"review"}]);
                    memory.prs.get_mut(&102).unwrap()["labels"] =
                        serde_json::json!([{"name":"review"}]);
                    flags.push("--add-tip-label=review");
                }
                _ => unreachable!(),
            }
        }
        w.run(&flags).unwrap();
        let output = w.0.lock().unwrap().stdout.clone();
        assert!(
            output
                .lines()
                .any(|row| row.contains("/pull/101 ") && row.ends_with("(updated)")),
            "{change}: {output}"
        );
        w.0.lock().unwrap().stdout.clear();
        w.run(&flags).unwrap();
        assert!(
            !w.0.lock().unwrap().stdout.contains("(updated)"),
            "{change}"
        );
    }
}

#[test]
fn stack_orientation_controls_preview_and_results_without_reordering_submission() {
    for stack_up in [false, true] {
        let w = World::new(3);
        let mut config = w.config();
        config.stack_up = stack_up;
        let original: Vec<_> = (1..=3).map(commit).collect();
        let mut selected = state(original.clone());
        w.app().present_selection(&config, &mut selected);
        let preview = w.0.lock().unwrap().stdout.clone();
        assert_eq!(
            preview.find("Feature 3") < preview.find("Feature 1"),
            stack_up
        );
        assert_eq!(
            selected
                .selected()
                .iter()
                .map(|c| &c.hash)
                .collect::<Vec<_>>(),
            original.iter().map(|c| &c.hash).collect::<Vec<_>>()
        );
        w.0.lock().unwrap().stdout.clear();
        w.app().print_results(&config, &original);
        let results = w.0.lock().unwrap().stdout.clone();
        assert_eq!(
            results.find("Feature 3") < results.find("Feature 1"),
            stack_up
        );
        assert!(!w.trace().iter().any(publication_effect));
    }
}
