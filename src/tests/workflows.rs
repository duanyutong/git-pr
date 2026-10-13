use serde_json::{Value, json};

use super::support::{Fixture, Service};

fn commands(trace: &[Value], program: &str, operation: &str) -> Vec<Value> {
    trace
        .iter()
        .filter(|v| v["kind"] == "command" && v["program"] == program && v["args"][0] == operation)
        .cloned()
        .collect()
}

#[test]
fn first_repeat_and_unchanged_submission_preserve_bases_bodies_and_draft_choice() {
    let f = Fixture::new(3, true);
    f.run(&[
        "--add-label",
        "all",
        "--add-tip-label",
        "tip",
        "--draft=false",
    ])
    .unwrap();
    let first = f.service.prs.lock().unwrap().clone();
    assert_eq!(first.len(), 3);
    for (n, pr) in &first {
        assert_eq!(
            pr["base"]["ref"],
            if *n == 101 {
                "main".into()
            } else {
                format!("topic/{}", n - 101)
            }
        );
        assert_eq!(pr["draft"], false);
        assert!(pr["body"].as_str().unwrap().contains("This is PR **"));
        assert!(pr["labels"].as_array().unwrap().contains(&json!("all")));
    }
    assert_eq!(first[&103]["labels"], json!(["all", "tip"]));
    let trace = f.trace();
    let pushes = commands(&trace, "git", "push");
    assert_eq!(pushes.len(), 1);
    let creates = trace
        .iter()
        .filter(|v| v["method"] == "POST" && v["path"] == "pulls")
        .count();
    assert_eq!(creates, 3);
    let last_push = trace
        .iter()
        .rposition(|v| v["program"] == "git" && v["args"][0] == "push")
        .unwrap();
    let first_create = trace
        .iter()
        .position(|v| v["method"] == "POST" && v["path"] == "pulls")
        .unwrap();
    assert!(last_push < first_create);
    assert!(
        f.clock
            .waits
            .lock()
            .unwrap()
            .iter()
            .all(|d| d.as_secs_f64() <= 1.0)
    );
    assert_eq!(f.git(&["branch", "--show-current"]), "topic/3");
    {
        let mut prs = f.service.prs.lock().unwrap();
        prs.get_mut(&102).unwrap()["draft"] = json!(true);
        let body = prs[&102]["body"].as_str().unwrap().to_owned();
        prs.get_mut(&102).unwrap()["body"] = json!(format!("Human edit\n{body}\n\nBot content"));
    }
    f.service.trace.lock().unwrap().clear();
    f.run(&["--draft=false"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 3);
    assert_eq!(prs[&102]["draft"], true);
    assert!(prs[&102]["body"].as_str().unwrap().contains("Human edit"));
    assert!(prs[&102]["body"].as_str().unwrap().ends_with("Bot content"));
    drop(prs);
    assert_eq!(
        f.trace().iter().filter(|v| v["method"] == "PATCH").count(),
        0
    );
    assert_eq!(commands(&f.trace(), "git", "push").len(), 1);
    assert_eq!(
        f.output()
            .matches("https://github.com/org/repo/pull/101")
            .count(),
        2
    );
}

#[test]
fn selections_retain_existing_bases_or_fail_before_publication() {
    for selection in [
        vec!["topic/2"],
        vec!["topic/1..topic/3"],
        vec!["topic/3", "topic/1"],
    ] {
        let f = Fixture::new(3, true);
        f.run(&[]).unwrap();
        let before = f.service.prs.lock().unwrap().clone();
        let remote = f.git(&["ls-remote", "--heads", "origin"]);
        f.service.trace.lock().unwrap().clear();
        if selection == ["topic/1..topic/3"] {
            f.run(&selection).unwrap();
            assert_eq!(commands(&f.trace(), "git", "push").len(), 1);
        } else {
            let error = format!("{:#}", f.run(&selection).unwrap_err());
            assert!(error.contains("unsafe publication"), "{error}");
            assert_eq!(commands(&f.trace(), "git", "push"), [] as [Value; 0]);
            assert_eq!(*f.service.prs.lock().unwrap(), before);
            assert_eq!(f.git(&["ls-remote", "--heads", "origin"]), remote);
        }
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs[&102]["base"]["ref"], "topic/1");
        assert_eq!(prs[&103]["base"]["ref"], "topic/2");
        drop(prs);
        assert_eq!(f.git(&["branch", "--show-current"]), "topic/3");
    }
}

#[test]
fn middle_branch_publication_refuses_unselected_dependants() {
    let f = Fixture::new(3, true);
    f.run(&[]).unwrap();
    f.git(&["checkout", "topic/2"]);
    let before = f.service.prs.lock().unwrap().clone();
    f.service.trace.lock().unwrap().clear();
    let error = format!("{:#}", f.run(&[]).unwrap_err());
    assert!(error.contains("open PR #103") && error.contains("outside the verified selection"));
    assert_eq!(commands(&f.trace(), "git", "push"), [] as [Value; 0]);
    assert_eq!(*f.service.prs.lock().unwrap(), before);
    assert_eq!(f.git(&["branch", "--show-current"]), "topic/2");
}

#[test]
fn checkpoints_and_dry_run_have_distinct_effects() {
    for phase in ["validate", "get-commits", "rewrite", "push", "pr-create"] {
        let f = Fixture::new(2, true);
        f.run(&["--stop-after", phase]).unwrap();
        let trace = f.trace();
        let pushes = commands(&trace, "git", "push");
        assert_eq!(
            pushes.len(),
            usize::from(["push", "pr-create"].contains(&phase))
        );
        assert!(!trace.iter().any(|v| v["method"] == "PATCH"));
        assert!(f.output().contains(&format!("stopped after: {phase}")));
    }
    let f = Fixture::new(2, true);
    f.run(&["--dry-run"]).unwrap();
    assert_eq!(
        commands(&f.trace(), "git", "push"),
        [] as [serde_json::Value; 0]
    );
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(f.clock.waits.lock().unwrap().is_empty());
    assert!(
        f.output()
            .contains("[DRY-RUN] Would update PR descriptions")
    );
}

#[test]
fn branchless_rewriting_dry_run_and_selection_recovery() {
    let f = Fixture::with_service(
        3,
        false,
        Service {
            branchless: true,
            ..Service::default()
        },
    );
    let original = f.hashes.clone();
    f.run(&["--dry-run", "topic/3", "topic/1"]).unwrap();
    assert_eq!(
        commands(&f.trace(), "git", "push"),
        [] as [serde_json::Value; 0]
    );
    assert_eq!(commands(&f.trace(), "git", "reword").len(), 0);
    for (index, hash) in original.iter().enumerate() {
        assert_eq!(
            &f.git(&["rev-parse", &format!("topic/{}", index + 1)]),
            hash
        );
    }
    assert!(f.output().contains("Would insert Remote-Ref"));
    f.run(&["topic/3", "topic/1"]).unwrap();
    assert_eq!(commands(&f.trace(), "git", "reword").len(), 1);
    for i in [1, 3] {
        let body = f.git(&["show", "-s", "--format=%B", &format!("topic/{i}")]);
        assert!(body.contains(&format!("Remote-Ref: topic/{i}")));
    }
    assert!(
        !f.git(&["show", "-s", "--format=%B", "topic/2"])
            .contains("Remote-Ref")
    );
    assert_ne!(f.git(&["rev-parse", "topic/3"]), original[2]);
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[&102]["base"]["ref"], "topic/1");
}

#[test]
fn labels_move_with_tip_and_author_filtering() {
    let mut f = Fixture::new(2, true);
    f.run(&[
        "--add-label",
        "all",
        "--add-tip-label",
        "tip",
        "-t",
        "tag,tag",
    ])
    .unwrap();
    f.add_commit(3, true, "other@example.com");
    f.service.trace.lock().unwrap().clear();
    f.run(&["--add-label", "all", "--add-tip-label", "tip"])
        .unwrap();
    assert_eq!(commands(&f.trace(), "git", "push").len(), 1);
    f.run(&[
        "--include-other-authors",
        "--add-label",
        "all",
        "--add-tip-label",
        "tip",
    ])
    .unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs[&102]["labels"], json!(["tag", "all"]));
    assert_eq!(prs[&103]["labels"], json!(["all", "tip"]));
}

#[test]
fn native_stack_configuration_authorises_linking_and_rebuild_remains_explicit() {
    let f = Fixture::new(2, true);
    f.run(&[]).unwrap();
    assert!(!f.trace().iter().any(|v| v["path"] == "stacks"));
    f.run(&["--github-stack"]).unwrap();
    assert!(!f.output().contains("skipped GitHub Stack"));
    assert!(
        f.trace()
            .iter()
            .any(|v| v["path"] == "stacks" && v["body"]["pull_requests"] == json!([101, 102]))
    );
    *f.service.stack_base.lock().unwrap() = Some("stale".into());
    f.run(&["--github-stack"]).unwrap();
    assert!(f.output().contains("skipped GitHub Stack rebuild"));
    assert!(!f.trace().iter().any(|v| v["path"] == "stacks/9/unstack"));
    f.run(&["--github-stack", "--yes"]).unwrap();
    assert!(f.trace().iter().any(|v| v["path"] == "stacks/9/unstack"));
    assert!(!f.trace().iter().any(|v| v["program"] == "gh"));
}

#[test]
fn failures_preserve_completed_pushes_and_report_exact_postcondition() {
    let f = Fixture::new(2, true);
    f.run(&["--stop-after=push"]).unwrap();
    f.service.trace.lock().unwrap().clear();
    let key = format!(
        "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1 {}:refs/heads/topic/2",
        f.hashes[0], f.hashes[1]
    );
    f.service
        .command_errors
        .lock()
        .unwrap()
        .insert(key.clone(), (1, "reference already exists".into()));
    f.run(&["--stop-after=push"]).unwrap();
    assert_eq!(commands(&f.trace(), "git", "ls-remote").len(), 1);
    f.git(&["push", "-f", "origin", "main:refs/heads/topic/1"]);
    let error = f.run(&["--stop-after=push"]).unwrap_err();
    assert!(format!("{error:#}").contains("failed to push 1 branch"));
    let verification = format!(
        "git ls-remote --refs -- {} refs/heads/topic/1 refs/heads/topic/2",
        f.git(&["remote", "get-url", "--push", "origin"])
    );
    f.service
        .command_errors
        .lock()
        .unwrap()
        .insert(verification.clone(), (1, "verification unavailable".into()));
    let error = f.run(&["--stop-after=push"]).unwrap_err();
    assert!(format!("{error:#}").contains("verification unavailable"));
    f.service
        .command_errors
        .lock()
        .unwrap()
        .remove(&verification);
    f.service.command_errors.lock().unwrap().remove(&key);
    f.service
        .http_errors
        .lock()
        .unwrap()
        .insert("POST issues/101/labels".into(), "API unavailable".into());
    assert!(format!("{:#}", f.run(&["--add-label=late"]).unwrap_err()).contains("API unavailable"));
    assert!(
        f.git(&["ls-remote", "origin", "refs/heads/topic/2"])
            .starts_with(&f.hashes[1])
    );
}

#[test]
fn empty_invalid_duplicate_unbranched_and_detached_submissions() {
    let empty = Fixture::new(0, true);
    assert_eq!(
        empty.run(&[]).unwrap_err().to_string(),
        "no commits to submit"
    );
    let f = Fixture::new(2, true);
    assert!(
        f.run(&["topic/2..topic/1"])
            .unwrap_err()
            .to_string()
            .contains("not found on trunk..tip path")
    );
    f.git(&[
        "commit",
        "--amend",
        "-m",
        "Feature 2\n\nRemote-Ref: topic/1",
    ]);
    assert!(
        f.run(&[])
            .unwrap_err()
            .to_string()
            .contains("duplicated remote ref")
    );
    assert!(f.output().contains("DUPLICATED REMOTE REF"));
    f.git(&[
        "commit",
        "--amend",
        "-m",
        "Feature 2\n\nRemote-Ref: bad..branch",
    ]);
    assert!(format!("{:#}", f.run(&[]).unwrap_err()).contains("refusing to push"));
    assert_eq!(
        commands(&f.trace(), "git", "push"),
        [] as [serde_json::Value; 0]
    );
    let unbranched = Fixture::new(1, false);
    unbranched.git(&["checkout", "--detach"]);
    unbranched.git(&["branch", "-D", "topic/1"]);
    assert!(format!("{:#}", unbranched.run(&[]).unwrap_err()).contains("not on any local branch"));
    let detached = Fixture::new(1, true);
    detached.git(&["checkout", "--detach"]);
    detached.run(&[]).unwrap();
    assert_eq!(detached.git(&["rev-parse", "HEAD"]), detached.hashes[0]);
    assert_eq!(detached.git(&["branch", "--show-current"]), "");
    assert_eq!(detached.git(&["rev-parse", "main"]), detached.trunk);
}

#[test]
fn draft_patterns_merged_history_and_native_blocked_bases() {
    let f = Fixture::new(2, true);
    f.git(&[
        "commit",
        "--amend",
        "-m",
        "WIP: pending\n\nRemote-Ref: topic/2",
    ]);
    f.run(&["--draft=false", "--skip-draft", "--stop-after=get-commits"])
        .unwrap();
    assert!(f.output().contains("skip draft"));
    assert!(f.service.prs.lock().unwrap().is_empty());
    f.run(&["--draft=false", "--skip-draft", "--stack-up=false"])
        .unwrap();
    {
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[&101]["draft"], false);
    }
    // Including drafts creates the excluded PR while preserving the first PR's state.
    f.run(&["--draft=false", "--skip-draft", "--include-draft"])
        .unwrap();
    {
        let mut prs = f.service.prs.lock().unwrap();
        assert_eq!(prs[&102]["draft"], true);
        for pr in prs.values_mut() {
            pr["merged"] = json!(true);
            pr["stack"] = json!({"number":9});
            pr["base"]["ref"] = json!("stale");
        }
    }
    f.service.trace.lock().unwrap().clear();
    let before = f.service.prs.lock().unwrap().clone();
    let error = format!("{:#}", f.run(&["--no-stack"]).unwrap_err());
    assert!(error.contains("PR #101") && error.contains("is merged"));
    assert_eq!(commands(&f.trace(), "git", "push"), [] as [Value; 0]);
    assert_eq!(*f.service.prs.lock().unwrap(), before);
}

#[test]
fn existing_drafts_and_manually_ready_prs_preserve_their_state() {
    let f = Fixture::new(2, true);
    f.run(&[]).unwrap();
    {
        let mut prs = f.service.prs.lock().unwrap();
        assert_eq!(prs[&101]["draft"], true);
        assert_eq!(prs[&102]["draft"], true);
        prs.get_mut(&102).unwrap()["draft"] = json!(false);
    }
    f.run(&[]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs[&101]["draft"], true);
    assert_eq!(prs[&102]["draft"], false);
}

#[test]
fn template_is_loaded_once_per_submission_even_when_file_changes() {
    let mut f = Fixture::new(0, true);
    for index in 1..=2 {
        f.add_commit(index, true, "fixture@example.com");
        f.git(&[
            "commit",
            "--amend",
            "-m",
            &format!("Feature {index}\n\nRemote-Ref: topic/{index}"),
        ]);
    }
    f.write(".github/PULL_REQUEST_TEMPLATE.md", "Initial template");
    *f.service.template_change.lock().unwrap() = Some((
        f.cwd.join(".github/PULL_REQUEST_TEMPLATE.md"),
        "Changed template".into(),
    ));
    f.run(&[]).unwrap();
    let creates: Vec<_> = f
        .trace()
        .into_iter()
        .filter(|entry| entry["method"] == "POST" && entry["path"] == "pulls")
        .collect();
    assert_eq!(creates.len(), 2);
    for entry in creates {
        assert_eq!(entry["body"]["body"], "Initial template");
    }
    for pr in f.service.prs.lock().unwrap().values() {
        assert!(pr["body"].as_str().unwrap().starts_with("Initial template"));
        assert!(!pr["body"].as_str().unwrap().contains("Changed template"));
    }
}

#[test]
fn native_rebuild_decline_missing_number_unstack_and_relink_errors() {
    for path in ["POST stacks/9/unstack", "POST stacks"] {
        let f = Fixture::new(2, true);
        f.run(&["--github-stack", "--yes"]).unwrap();
        *f.service.stack_base.lock().unwrap() = Some("stale".into());
        f.terminal.answers.lock().unwrap().extend([true, false]);
        f.run(&["--github-stack"]).unwrap();
        assert!(!f.trace().iter().any(|v| v["path"] == "stacks/9/unstack"));
        f.service
            .http_errors
            .lock()
            .unwrap()
            .insert(path.into(), "service unavailable".into());
        assert!(
            format!("{:#}", f.run(&["--github-stack", "--yes"]).unwrap_err())
                .contains("service unavailable")
        );
    }
    let single = Fixture::new(1, true);
    single.run(&["--github-stack", "--yes"]).unwrap();
    assert!(!single.trace().iter().any(|v| v["path"] == "stacks"));
}

#[test]
fn invalid_repository_selection_messages_templates_and_configuration() {
    let f = Fixture::new(2, true);
    for (args, message) in [
        (vec!["--stop-after=bad"], "invalid --stop-after"),
        (vec!["main...HEAD"], "symmetric-difference"),
        (vec!["..HEAD"], "both endpoints"),
        (vec!["main.."], "both endpoints"),
        (vec!["main..HEAD", "HEAD"], "cannot combine"),
        (vec!["missing"], "failed to resolve"),
        (vec!["--output=bad"], "invalid --output"),
    ] {
        assert!(format!("{:#}", f.run(&args).unwrap_err()).contains(message));
    }
    f.write(".github/PULL_REQUEST_TEMPLATE.md", "# Custom template");
    f.git(&["add", "."]);
    f.git(&["commit", "--amend", "--no-edit"]);
    f.git(&["config", "git-pr.addLabel", "project"]);
    f.git(&["config", "git-pr.addTipLabel", "tip"]);
    f.git(&["config", "git-pr.output", "{shorthash} #{number}"]);
    f.run(&["--add-label", "cli"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs[&101]["labels"], json!(["cli"]));
    assert_eq!(prs[&102]["labels"], json!(["cli", "tip"]));
    drop(prs);
    f.run(&["--default-tags", " a, b, ,a "]).unwrap();
    assert_eq!(f.git(&["config", "--get", "git-pr.tags"]), "a,b,a");
    f.write("base.txt", "dirty");
    assert!(
        f.run(&[])
            .unwrap_err()
            .to_string()
            .contains("uncommitted changes")
    );
}

#[test]
fn branchless_submission_shares_body_analysis_and_bases_with_native_linking() {
    use super::unit::support::{BASE_PLANS, BODY_ANALYSES};
    for native in [false, true] {
        let f = Fixture::with_service(
            3,
            false,
            Service {
                branchless: true,
                ..Service::default()
            },
        );
        let flags: &[&str] = if native {
            &["--github-stack", "--yes"]
        } else {
            &[]
        };
        for first in [true, false] {
            f.service.trace.lock().unwrap().clear();
            BODY_ANALYSES.set(0);
            BASE_PLANS.set(0);
            f.run(flags).unwrap();
            assert_eq!(BODY_ANALYSES.get(), 3);
            assert_eq!(BASE_PLANS.get(), 1);
            let trace = f.trace();
            assert_eq!(commands(&trace, "git", "push").len(), 1);
            assert_eq!(commands(&trace, "git", "reword").len(), usize::from(first));
            assert!(!trace.iter().any(|v| v["program"] == "gh"));
        }
    }
}
