//! Operation budgets and end-state contracts for the optimised workflow.

use super::support::{Response, World};
use serde_json::{Value, json};

fn count(trace: &[Value], predicate: impl Fn(&Value) -> bool) -> usize {
    trace.iter().filter(|v| predicate(v)).count()
}

fn metadata_mutation(event: &Value) -> bool {
    event["kind"] == "http"
        && event["method"] != "GET"
        && !(event["path"] == "graphql"
            && event["body"]["query"]
                .as_str()
                .is_some_and(|query| !query.starts_with("mutation")))
}

#[test]
fn unchanged_submission_observes_each_safety_barrier_without_mutations_or_waits() {
    for size in [1, 5, 24, 25, 26, 100, 101] {
        assert_unchanged_submission(size);
    }
}

#[test]
#[ignore = "optional large-stack operation check; run explicitly with --ignored"]
fn unchanged_submission_at_large_scale() {
    assert_unchanged_submission(1000);
}

fn assert_unchanged_submission(size: u64) {
    let world = World::new(size);
    world.run(&["--no-stack"]).unwrap();
    let before = world.0.lock().unwrap().prs.clone();
    world.0.lock().unwrap().trace.clear();
    world.run(&["--no-stack"]).unwrap();
    let trace = world.trace();
    assert_eq!(world.0.lock().unwrap().prs, before);
    assert_eq!(
        count(&trace, |v| v["program"] == "git" && v["args"][0] == "push"),
        0
    );
    assert_eq!(
        count(&trace, |v| v["path"] == "graphql"),
        usize::try_from(size).unwrap().div_ceil(25)
    );
    assert_eq!(count(&trace, |v| v["kind"] == "wait"), 0);
    assert_eq!(
        count(&trace, |v| v["method"] == "PATCH"
            || v["method"] == "DELETE"
            || v["body"]["query"]
                .as_str()
                .is_some_and(|q| q.starts_with("mutation"))),
        0
    );
    assert!(!trace.iter().any(|v| v["program"] == "gh"));
    assert_eq!(
        count(&trace, |v| v["program"] == "git"
            && v["args"][0] == "config"),
        9
    );
    assert_eq!(
        count(&trace, |v| v["program"] == "git"
            && v["args"][0] == "check-ref-format"),
        0
    );
    assert_eq!(
        count(&trace, |v| v["program"] == "git" && v["args"][0] == "log"),
        1
    );
}

#[test]
fn first_submission_uses_one_publication_and_bounded_content_mutations() {
    let world = World::new(5);
    world.run(&["--no-stack"]).unwrap();
    let trace = world.trace();
    assert_eq!(
        count(&trace, |v| v["path"] == "pulls" && v["method"] == "POST"),
        5
    );
    assert_eq!(
        count(&trace, |v| v["body"]["query"]
            .as_str()
            .is_some_and(|q| q.starts_with("mutation"))),
        1
    );
    assert_eq!(
        count(&trace, |v| v["program"] == "git" && v["args"][0] == "push"),
        1
    );
    let prs = world.0.lock().unwrap().prs.clone();
    for (i, pr) in prs.values().enumerate() {
        assert_eq!(
            pr["base"]["ref"],
            if i == 0 {
                "main".to_owned()
            } else {
                format!("topic/{i}")
            }
        );
        assert!(pr["body"].as_str().unwrap().contains("This PR"));
    }
}

#[test]
fn native_linkage_is_idempotent_and_never_invokes_an_extension() {
    let world = World::new(3);
    world.run(&["--github-stack", "--yes"]).unwrap();
    assert_eq!(
        count(&world.trace(), |v| v["path"] == "stacks"
            && v["method"] == "POST"),
        1
    );
    world.0.lock().unwrap().trace.clear();
    world.run(&["--github-stack", "--yes"]).unwrap();
    let trace = world.trace();
    assert_eq!(
        count(&trace, |v| v["path"] == "stacks/9" && v["method"] == "GET"),
        1
    );
    assert!(
        !trace
            .iter()
            .any(|v| v["program"] == "gh" || v["kind"] == "confirm" || v["kind"] == "wait")
    );
}

#[test]
fn unchanged_metadata_preserves_human_and_bot_text_across_footer_and_native_modes() {
    for native in [false, true] {
        for compact in [false, true] {
            for stack_up in [false, true] {
                for emoji in [false, true] {
                    let world = World::new(12);
                    {
                        let mut memory = world.0.lock().unwrap();
                        memory
                            .config
                            .insert("git-pr.stackUp".into(), stack_up.to_string());
                        memory
                            .config
                            .insert("git-pr.stackEmoji".into(), emoji.to_string());
                    }
                    let mut flags =
                        vec!["--yes", "--add-label=reviewed", "--add-tip-label=stack-tip"];
                    flags.push(if native {
                        "--github-stack"
                    } else {
                        "--no-stack"
                    });
                    if compact {
                        flags.push("--compact-stack");
                    }
                    world.run(&flags).unwrap();
                    let before = {
                        let mut memory = world.0.lock().unwrap();
                        for pr in memory.prs.values_mut() {
                            let body = pr["body"].as_str().unwrap();
                            pr["body"] = json!(format!(
                                "Human summary\n\n{body}\n\n<!-- CURSOR_SUMMARY -->\n---\n\n> [!NOTE]\n> **Overview**\n> \n> Retain 32\u{202f}MiB and trailing spaces.  \n\n<!-- /CURSOR_SUMMARY -->"
                            ));
                        }
                        memory.prs.clone()
                    };
                    for _ in 0..2 {
                        {
                            let mut memory = world.0.lock().unwrap();
                            memory.trace.clear();
                            memory.stdout.clear();
                        }
                        world.run(&flags).unwrap();
                        let memory = world.0.lock().unwrap();
                        assert_eq!(memory.prs, before, "{flags:?}");
                        assert!(
                            !memory.trace.iter().any(metadata_mutation),
                            "{flags:?}: {:?}",
                            memory.trace
                        );
                        assert!(
                            !memory.stdout.contains("(updated)")
                                && !memory.stdout.contains("(created)"),
                            "{flags:?}: {}",
                            memory.stdout
                        );
                        assert_eq!(
                            memory
                                .stdout
                                .lines()
                                .filter(|line| line.starts_with("https://github.com/org/repo/pull/"))
                                .count(),
                            12,
                            "{flags:?}: {}",
                            memory.stdout
                        );
                        drop(memory);
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_metadata_changes_update_only_affected_prs_and_do_not_repeat() {
    for native in [false, true] {
        let world = World::new(5);
        let flags = [
            "--yes",
            if native {
                "--github-stack"
            } else {
                "--no-stack"
            },
            "--add-label=reviewed",
            "--add-tip-label=stack-tip",
        ];
        world.run(&flags).unwrap();
        let before = {
            let mut memory = world.0.lock().unwrap();
            let before = memory.prs.clone();
            memory.commits[0].title = "Changed title".into();
            let body = memory.prs[&102]["body"]
                .as_str()
                .unwrap()
                .replace("This PR", "Outdated annotation");
            memory.prs.get_mut(&102).unwrap()["body"] = json!(body);
            memory.prs.get_mut(&103).unwrap()["labels"] = json!([]);
            memory.trace.clear();
            memory.stdout.clear();
            before
        };
        world.run(&flags).unwrap();
        {
            let memory = world.0.lock().unwrap();
            let mutations: Vec<_> = memory
                .trace
                .iter()
                .filter(|e| metadata_mutation(e))
                .collect();
            assert_eq!(mutations.len(), 2, "{mutations:?}");
            let content = mutations.iter().find(|e| e["path"] == "graphql").unwrap();
            let inputs = content["body"]["variables"].as_object().unwrap();
            let changed: std::collections::BTreeSet<_> = inputs
                .values()
                .map(|input| input["pullRequestId"].as_str().unwrap())
                .collect();
            assert_eq!(
                changed,
                std::collections::BTreeSet::from(["PR_101", "PR_102"])
            );
            assert!(mutations.iter().any(|e| e["path"] == "issues/103/labels"));
            for number in 101..=105 {
                let row = memory
                    .stdout
                    .lines()
                    .find(|row| row.contains(&format!("/pull/{number} ")))
                    .unwrap();
                assert_eq!(row.ends_with("(updated)"), number <= 103, "{row}");
                if number == 102 {
                    let body = memory.prs[&number]["body"].as_str().unwrap();
                    assert!(body.starts_with("Body 2\n"));
                    assert!(body.contains("#102 👈 This PR"));
                    assert!(!body.contains("Outdated annotation"));
                } else {
                    assert_eq!(memory.prs[&number]["body"], before[&number]["body"]);
                }
            }
            drop(memory);
        }
        {
            let mut memory = world.0.lock().unwrap();
            memory.trace.clear();
            memory.stdout.clear();
        }
        world.run(&flags).unwrap();
        let memory = world.0.lock().unwrap();
        assert!(!memory.trace.iter().any(metadata_mutation));
        assert!(!memory.stdout.contains("(updated)"));
        drop(memory);
    }
}

#[test]
fn footer_hash_migration_happens_once_and_rebased_heads_preserve_descriptions() {
    for native in [false, true] {
        let world = World::new(3);
        let flags = [
            "--yes",
            if native {
                "--github-stack"
            } else {
                "--no-stack"
            },
        ];
        world.run(&flags).unwrap();
        {
            let mut memory = world.0.lock().unwrap();
            for pr in memory.prs.values_mut() {
                let body = pr["body"]
                    .as_str()
                    .unwrap()
                    .replace(" 👈 This PR\n", " 👈 This PR (deadbeef)\n");
                pr["body"] = json!(format!(
                    "Human summary (deadbeef)\n\n{body}\n\n<!-- CURSOR_SUMMARY -->\n> Reviewed deadbeef.  \n<!-- /CURSOR_SUMMARY -->"
                ));
            }
            memory.trace.clear();
        }
        world.run(&flags).unwrap();
        assert_eq!(count(&world.trace(), metadata_mutation), 1);
        let bodies: std::collections::BTreeMap<_, _> = world
            .0
            .lock()
            .unwrap()
            .prs
            .iter()
            .map(|(&number, pr)| {
                let body = pr["body"].as_str().unwrap();
                assert!(!body.contains("This PR ("));
                assert!(body.starts_with("Human summary (deadbeef)"));
                assert!(body.ends_with("<!-- /CURSOR_SUMMARY -->"));
                (number, body.to_owned())
            })
            .collect();
        for rebase in [true, false] {
            {
                let mut memory = world.0.lock().unwrap();
                if rebase {
                    for (index, commit) in memory.commits.iter_mut().enumerate() {
                        commit.hash = format!("abcdef12{index:032x}");
                    }
                }
                memory.trace.clear();
                memory.stdout.clear();
            }
            world.run(&flags).unwrap();
            let memory = world.0.lock().unwrap();
            assert!(!memory.trace.iter().any(metadata_mutation));
            for (&number, body) in &bodies {
                assert_eq!(memory.prs[&number]["body"], *body);
            }
            if !rebase {
                assert!(!memory.stdout.contains("(updated)"));
            }
            drop(memory);
        }
    }
}

#[test]
fn incomplete_observations_do_not_authorise_publication_or_creation() {
    for body in [
        json!({"data":{"repository":null}}),
        json!({"errors":[{"message":"rate limited"}]}),
        json!({"data":{"repository":{"p0":{"nodes":[]}}}}),
    ] {
        let world = World::new(1);
        world.http(
            "POST graphql",
            vec![Response::Output(200, body.to_string())],
        );
        assert!(world.run(&[]).is_err());
        assert!(
            !world
                .trace()
                .iter()
                .any(|v| v["args"][0] == "push" || v["path"] == "pulls")
        );
    }
}

#[test]
fn excluded_authors_and_local_checkpoints_need_no_service_or_rewrite() {
    let world = World::new(3);
    {
        let mut m = world.0.lock().unwrap();
        for c in &mut m.commits {
            c.author_email = "other@example.com".into();
            c.attrs.clear();
        }
    }
    world.run(&[]).unwrap();
    assert!(
        !world
            .trace()
            .iter()
            .any(|v| v["kind"] == "http" || v["args"][0] == "push" || v["args"][0] == "reword")
    );
    for phase in ["validate", "get-commits"] {
        let world = World::new(1);
        world.0.lock().unwrap().files.clear();
        world.run(&[&format!("--stop-after={phase}")]).unwrap();
        assert!(
            !world
                .trace()
                .iter()
                .any(|v| v["kind"] == "credential" || v["kind"] == "http")
        );
    }
}

#[test]
fn moving_tip_removes_only_observed_labels_and_reports_removal_failures() {
    for fail in [false, true] {
        let world = World::new(2);
        world.run(&["--add-tip-label=tip /label"]).unwrap();
        world
            .0
            .lock()
            .unwrap()
            .commits
            .push(super::support::commit(3));
        if fail {
            world.http(
                "DELETE issues/102/labels/tip%20%2Flabel",
                vec![Response::Failure("removal rejected".into())],
            );
        }
        let result = world.run(&["--add-tip-label=tip /label"]);
        assert_eq!(result.is_err(), fail);
        if !fail {
            assert_eq!(world.0.lock().unwrap().prs[&102]["labels"], json!([]));
        }
    }
}

#[test]
fn failed_native_api_reports_failure_and_keeps_completed_pr_updates() {
    let world = World::new(2);
    world.http(
        "POST stacks",
        vec![Response::Output(404, "unavailable".into())],
    );
    let error = world.run(&["--github-stack", "--yes"]).unwrap_err();
    assert_eq!(crate::diagnostics::errors::status(&error), 1);
    assert_eq!(
        error
            .downcast_ref::<crate::github::http::HttpStatusError>()
            .unwrap()
            .status
            .as_u16(),
        404
    );
    assert_eq!(world.0.lock().unwrap().prs.len(), 2);
    assert!(
        !world
            .0
            .lock()
            .unwrap()
            .stderr
            .contains("Native stacks are unavailable")
    );
}

#[test]
fn candidate_tip_reduction_batches_calls_and_checks_every_discarded_root() {
    for size in [1, 10, 100, 129, 1000] {
        let mut tips: Vec<_> = (0..size).map(|n| format!("{n:04}")).collect();
        // Cover different positions of the global maximum, including batch boundaries.
        for pivot in [0, size / 2, size - 1] {
            tips.swap(pivot, size - 1);
            let calls = std::cell::Cell::new(0);
            let leaf = crate::stack::selection::pick_stack_leaf(&tips, &|batch| {
                calls.set(calls.get() + 1);
                assert!(batch.len() <= 128);
                Ok(vec![batch.iter().max().unwrap().clone()])
            })
            .unwrap();
            assert_eq!(leaf.as_ref(), tips.iter().max());
            assert!(calls.get() <= 2 * size.div_ceil(127));
            if size > 1 {
                let leaf = crate::stack::selection::pick_stack_leaf(&tips, &|batch| {
                    let mut roots = Vec::new();
                    for parity in [0, 1] {
                        if let Some(root) = batch
                            .iter()
                            .filter(|tip| tip.parse::<usize>().unwrap() % 2 == parity)
                            .max()
                        {
                            roots.push(root.clone());
                        }
                    }
                    Ok(roots)
                })
                .unwrap();
                assert_eq!(leaf, None);
            }
        }
    }
    let tips = vec!["x".to_owned(); 129];
    assert!(crate::stack::selection::pick_stack_leaf(&tips, &|_| Ok(vec![])).is_err());
    let calls = std::cell::Cell::new(0);
    assert!(
        crate::stack::selection::pick_stack_leaf(&tips, &|_| {
            calls.set(calls.get() + 1);
            if calls.get() > 2 {
                anyhow::bail!("verification unavailable");
            }
            Ok(vec!["x".into()])
        })
        .is_err()
    );
}

#[test]
fn submission_reuses_body_analysis_and_base_plans_in_both_native_modes() {
    use super::support::{BASE_PLANS, BODY_ANALYSES};
    for native in [false, true] {
        for size in [1_u64, 5, 26, 129] {
            let world = World::new(size);
            {
                let mut memory = world.0.lock().unwrap();
                memory.branchless = true;
                memory.config.insert("git-pr.stackUp".into(), "true".into());
            }
            let flags: &[&str] = if native {
                &["--github-stack", "--yes"]
            } else {
                &[]
            };
            for phase in ["first", "unchanged", "changed", "unchanged-after-change"] {
                if phase == "changed" {
                    for pr in world.0.lock().unwrap().prs.values_mut() {
                        pr["title"] = json!("Stale title");
                        let body = pr["body"]
                            .as_str()
                            .unwrap()
                            .replace("This PR", "Outdated annotation");
                        pr["body"] = json!(format!("Human content\n{body}\n\nBot content"));
                    }
                }
                world.0.lock().unwrap().trace.clear();
                BODY_ANALYSES.set(0);
                BASE_PLANS.set(0);
                world.run(flags).unwrap();
                assert_eq!(BODY_ANALYSES.get(), usize::try_from(size).unwrap());
                assert_eq!(BASE_PLANS.get(), 1);
                let trace = world.trace();
                assert_eq!(
                    count(&trace, |v| v["program"] == "git" && v["args"][0] == "push"),
                    if phase == "first" {
                        usize::try_from(size).unwrap().div_ceil(128)
                    } else {
                        0
                    }
                );
                let changed = matches!(phase, "first" | "changed");
                let mut content_ids = std::collections::HashSet::new();
                for request in trace.iter().filter(|v| {
                    v["body"]["query"]
                        .as_str()
                        .is_some_and(|q| q.starts_with("mutation"))
                }) {
                    for input in request["body"]["variables"].as_object().unwrap().values() {
                        assert!(content_ids.insert(input["pullRequestId"].as_str().unwrap()));
                    }
                }
                assert_eq!(
                    content_ids.len(),
                    if changed {
                        usize::try_from(size).unwrap()
                    } else {
                        0
                    }
                );
                assert_eq!(
                    count(&trace, |v| v["path"] == "stacks" && v["method"] == "POST"),
                    usize::from(native && size > 1 && phase == "first")
                );
                assert!(!trace.iter().any(|v| v["program"] == "gh"));
                if phase.contains("change") && phase != "unchanged" {
                    for pr in world.0.lock().unwrap().prs.values() {
                        let body = pr["body"].as_str().unwrap();
                        assert!(body.starts_with("Human content"));
                        assert!(body.ends_with("Bot content"));
                    }
                }
            }
        }
    }
}

#[test]
fn rewritten_selections_and_compact_footers_share_one_base_plan() {
    use super::support::{BASE_PLANS, BODY_ANALYSES};
    for native in [false, true] {
        for compact in [false, true] {
            let world = World::new(3);
            {
                let mut memory = world.0.lock().unwrap();
                memory.branchless = true;
                memory
                    .commits
                    .iter_mut()
                    .for_each(|commit| commit.attrs.clear());
            }
            let mut flags = vec!["--yes"];
            if native {
                flags.push("--github-stack");
            }
            if compact {
                flags.push("--compact-stack");
            }
            flags.extend(["topic/3", "topic/1"]);
            BODY_ANALYSES.set(0);
            BASE_PLANS.set(0);
            world.run(&flags).unwrap();
            assert_eq!(BASE_PLANS.get(), 1);
            assert_eq!(BODY_ANALYSES.get(), 2);
            let memory = world.0.lock().unwrap();
            assert_eq!(memory.prs.len(), 2);
            assert_eq!(memory.prs[&101]["head"]["ref"], "topic/1");
            assert_eq!(memory.prs[&102]["base"]["ref"], "topic/1");
        }
    }
}
