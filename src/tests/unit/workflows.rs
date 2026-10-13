use super::support::{Response, World, commit, log};
use crate::{diagnostics::errors, model::REMOTE_REF, stack::description};
use serde_json::{Value, json};

fn operation(trace: &[Value], program: &str, verb: &str) -> usize {
    trace
        .iter()
        .filter(|v| v["program"] == program && v["args"][0] == verb)
        .count()
}

#[test]
fn unused_creation_templates_cannot_block_local_checkpoints_or_previews() {
    for args in [
        &["--stop-after=get-commits"][..],
        &["--stop-after=rewrite"],
        &["--dry-run"],
    ] {
        let world = World::new(1);
        world.0.lock().unwrap().file_errors.insert(
            "/unit/repo/.github/PULL_REQUEST_TEMPLATE.md".into(),
            std::io::ErrorKind::PermissionDenied,
        );
        world.run(args).unwrap();
        assert!(!world.trace().iter().any(|event| {
            event["kind"] == "file"
                && event["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("/unit/repo/"))
        }));
        assert_eq!(operation(&world.trace(), "git", "push"), 0);
        assert!(world.0.lock().unwrap().prs.is_empty());
    }
    for args in [&[][..], &["--stop-after=push"], &["--stop-after=pr-create"]] {
        let world = World::new(1);
        world.0.lock().unwrap().file_errors.insert(
            "/unit/repo/.github/PULL_REQUEST_TEMPLATE.md".into(),
            std::io::ErrorKind::PermissionDenied,
        );
        let error = world.run(args).unwrap_err();
        assert!(format!("{error:#}").contains("PULL_REQUEST_TEMPLATE.md"));
        assert_eq!(operation(&world.trace(), "git", "reword"), 0);
        assert_eq!(operation(&world.trace(), "git", "push"), 0);
        assert!(world.0.lock().unwrap().prs.is_empty());
    }
}

#[test]
fn submission_and_repeat_keep_push_creation_history_and_patch_barriers() {
    let world = World::new(2);
    world
        .run(&["--no-stack", "--add-label=shared", "--add-tip-label=tip"])
        .unwrap();
    let trace = world.trace();
    assert_eq!(operation(&trace, "git", "push"), 1);
    let last_push = trace
        .iter()
        .rposition(|v| v["program"] == "git" && v["args"][0] == "push")
        .unwrap();
    let first_create = trace
        .iter()
        .position(|v| v["method"] == "POST" && v["path"] == "pulls")
        .unwrap();
    assert!(last_push < first_create);
    assert_eq!(world.0.lock().unwrap().prs.len(), 2);
    {
        let mut m = world.0.lock().unwrap();
        let body = m.prs[&101]["body"]
            .as_str()
            .unwrap()
            .replace(description::END, &format!("* #99\n{}", description::END));
        m.prs.get_mut(&101).unwrap()["body"] = json!(body);
        m.trace.clear();
    }
    world.run(&["--no-stack"]).unwrap();
    let m = world.0.lock().unwrap();
    assert_eq!(m.prs.len(), 2);
    assert!(m.prs.values().all(|pr| {
        description::extract_history(pr["body"].as_str().unwrap())
            .iter()
            .any(|h| h.number == 99)
    }));
    drop(m);
}

#[test]
fn checkpoints_stop_at_each_public_effect_boundary_and_dry_run_has_no_mutations() {
    for (phase, pushes, creates, patches) in [
        ("validate", 0, 0, 0),
        ("get-commits", 0, 0, 0),
        ("rewrite", 0, 0, 0),
        ("push", 1, 2, 0),
        ("pr-create", 1, 2, 0),
    ] {
        let w = World::new(2);
        w.run(&[&format!("--stop-after={phase}")]).unwrap();
        let trace = w.trace();
        assert_eq!(operation(&trace, "git", "push"), pushes);
        assert_eq!(
            trace
                .iter()
                .filter(|v| v["method"] == "POST" && v["path"] == "pulls")
                .count(),
            creates
        );
        assert_eq!(
            trace.iter().filter(|v| v["method"] == "PATCH").count(),
            patches
        );
        assert!(
            w.0.lock()
                .unwrap()
                .stdout
                .contains(&format!("stopped after: {phase}"))
        );
    }
    let w = World::new(2);
    w.run(&["--dry-run"]).unwrap();
    assert_eq!(operation(&w.trace(), "git", "push"), 0);
    assert_eq!(operation(&w.trace(), "gh", "pr"), 0);
    assert!(
        w.0.lock()
            .unwrap()
            .stdout
            .contains("Would update PR descriptions")
    );
}

#[test]
fn failed_lookup_stops_all_creations_and_description_history_failure_stops_patches() {
    let w = World::new(2);
    w.http(
        "POST graphql",
        vec![Response::Failure("lookup failed".into())],
    );
    let error = w.run(&[]).unwrap_err();
    assert_eq!(errors::status(&error), 2);
    assert_eq!(operation(&w.trace(), "git", "push"), 0);
    assert!(w.0.lock().unwrap().prs.is_empty());
    let w = World::new(2);
    w.run(&[]).unwrap();
    let before = w.0.lock().unwrap().prs.clone();
    w.http(
        "POST graphql",
        vec![Response::Failure("history failed".into())],
    );
    assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains("history failed"));
    assert_eq!(w.0.lock().unwrap().prs, before);
}

#[test]
fn draft_and_explicit_selection_exclusions_survive_changed_hashes_after_rewrite() {
    let w = World::new(2);
    {
        let mut m = w.0.lock().unwrap();
        m.branchless = true;
        m.commits[0].attrs.clear();
        m.commits[1].title = "WIP: pending".into();
    }
    w.run(&["--skip-draft", "--no-stack"]).unwrap();
    assert_eq!(w.0.lock().unwrap().prs.len(), 1);
    assert_eq!(operation(&w.trace(), "git", "reword"), 1);
    assert!(!w.trace().iter().any(|v| v["program"] == "git"
        && v["args"][0] == "push"
        && v["args"][3].as_str().unwrap().ends_with("topic/2")));
    w.run(&["--skip-draft", "--include-draft", "--no-stack"])
        .unwrap();
    assert_eq!(w.0.lock().unwrap().prs.len(), 2);
    for selection in [
        vec!["topic/1..topic/3"],
        vec!["topic/3", "topic/1"],
        vec!["topic/2"],
    ] {
        let w = World::new(3);
        w.run(&selection).unwrap();
        assert_eq!(operation(&w.trace(), "git", "push"), 1);
    }
}

#[test]
fn discovery_and_rewrite_failures_do_not_push_or_create_prs() {
    let w = World::new(0);
    assert!(w.run(&[]).unwrap_err().to_string().contains("no commits"));
    for key in [
        "git show --no-show-signature -s --format=%H %T refs/remotes/origin/main",
        "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..HEAD",
        "git status --porcelain=v1 --untracked-files=no",
    ] {
        let w = World::new(1);
        w.fail(key);
        assert!(w.run(&[]).is_err());
        assert_eq!(operation(&w.trace(), "git", "push"), 0);
    }
    for response in [
        Response::Failure("branches unavailable".into()),
        Response::Output(0, String::new()),
    ] {
        let w = World::new(1);
        w.0.lock().unwrap().commits[0].attrs.clear();
        w.0.lock().unwrap().responses.insert(
            "git branch --format=%(refname:strip=2)|%(objectname)".into(),
            vec![response].into(),
        );
        assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains("local branch"));
        assert_eq!(operation(&w.trace(), "git", "push"), 0);
    }
    let w = World::new(2);
    w.0.lock().unwrap().commits[1].set_attr(REMOTE_REF, "topic/1");
    assert!(w.run(&[]).unwrap_err().to_string().contains("duplicated"));
    let w = World::new(1);
    w.0.lock().unwrap().commits[0].attrs.clear();
    assert!(w.run(&[]).is_err());
}

#[test]
fn push_and_creation_failures_preserve_completed_effects_and_stop_later_phases() {
    let w = World::new(2);
    w.fail(&format!(
        "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1 {}:refs/heads/topic/2",
        commit(1).hash,
        commit(2).hash
    ));
    w.reply(
        "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/topic/1 refs/heads/topic/2",
        0,
        &format!("{} refs/heads/topic/1", commit(1).hash),
    );
    let error = format!("{:#}", w.run(&[]).unwrap_err());
    assert!(error.contains("1 branch(es): topic/2"));
    assert_eq!(operation(&w.trace(), "git", "push"), 1);
    assert!(w.0.lock().unwrap().prs.is_empty());
    let w = World::new(1);
    w.0.lock().unwrap().commits[0].attrs[0].1 = "invalid..ref".into();
    assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains("invalid Remote-Ref"));
    assert_eq!(operation(&w.trace(), "git", "push"), 0);
    let w = World::new(1);
    w.http(
        "POST pulls",
        vec![Response::Failure("create failed".into())],
    );
    assert!(w.run(&[]).is_err());
    assert_eq!(operation(&w.trace(), "git", "push"), 1);
    assert!(w.0.lock().unwrap().prs.is_empty());
}

#[test]
fn author_filters_detached_restoration_descendant_history_and_native_linking() {
    let w = World::new(3);
    {
        let mut m = w.0.lock().unwrap();
        m.commits[0].author_email = "foreign@example.com".into();
        m.commits[1].author_email = "other@example.com".into();
        m.branch.clear();
    }
    w.run(&["--stack-up=false"]).unwrap();
    assert_eq!(w.0.lock().unwrap().prs.len(), 1);
    assert!(w.0.lock().unwrap().stdout.contains("foreign@example.com"));
    let w = World::new(2);
    w.run(&["--github-stack", "--yes"]).unwrap();
    assert_eq!(
        w.trace()
            .iter()
            .filter(|v| v["method"] == "POST" && v["path"] == "stacks")
            .count(),
        1
    );
    let w = World::new(1);
    w.run(&["--default-tags=a,b"]).unwrap();
    assert_eq!(operation(&w.trace(), "git", "push"), 0);
    let w = World::new(1);
    w.reply("git rev-parse HEAD", 0, "head");
    w.reply(
        &format!(
            "git for-each-ref --contains {} --format=%(objectname) refs/heads/",
            commit(1).hash
        ),
        0,
        "descendant",
    );
    let mut descendant = commit(2);
    descendant.hash = "dddddddddddddddddddddddddddddddddddddddd".into();
    w.reply(
        &format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature {}..descendant",commit(1).hash),
        0,
        &log(&[descendant]),
    );
    w.run(&[]).unwrap();
    assert!(w.trace().iter().any(|v| {
        v["path"] == "graphql"
            && v["body"]["variables"]
                .as_object()
                .unwrap()
                .values()
                .any(|v| v == "topic/2")
    }));
}

#[test]
fn preview_reports_trailers_without_rewriting_or_changing_configuration() {
    for jj in [false, true] {
        let w = World::new(2);
        {
            let mut m = w.0.lock().unwrap();
            m.jj = jj;
            m.branchless = !jj;
            for c in &mut m.commits {
                c.attrs.clear();
            }
        }
        let before: Vec<_> =
            w.0.lock()
                .unwrap()
                .commits
                .iter()
                .map(crate::model::Commit::full_message)
                .collect();
        w.run(&["--dry-run"]).unwrap();
        let m = w.0.lock().unwrap();
        assert_eq!(
            before,
            m.commits
                .iter()
                .map(crate::model::Commit::full_message)
                .collect::<Vec<_>>()
        );
        assert!(m.prs.is_empty());
        assert!(m.stdout.contains("Would insert Remote-Ref"));
        assert!(!m.trace.iter().any(|v| v["method"].is_string()
            || v["args"][0] == "reword"
            || v["args"][0] == "describe"
            || v["args"][0] == "util"));
        drop(m);
    }
    let w = World::new(0);
    w.run(&["--dry-run", "--default-tags=preview"]).unwrap();
    assert!(!w.0.lock().unwrap().config.contains_key("git-pr.tags"));
}

#[test]
fn compact_navigation_is_optional_stable_and_preserves_surrounding_content() {
    let w = World::new(3);
    w.0.lock()
        .unwrap()
        .config
        .insert("git-pr.compactStack".into(), "true".into());
    w.run(&[]).unwrap();
    let before = w.0.lock().unwrap().prs.clone();
    assert!(
        before[&102]["body"]
            .as_str()
            .unwrap()
            .contains("* #101 — Previous PR")
    );
    assert!(
        before[&102]["body"]
            .as_str()
            .unwrap()
            .contains("* #103 — Next PR")
    );
    w.run(&[]).unwrap();
    assert_eq!(before, w.0.lock().unwrap().prs);
    w.run(&["--compact-stack=false"]).unwrap();
    assert!(
        !w.0.lock().unwrap().prs[&102]["body"]
            .as_str()
            .unwrap()
            .contains("Stack navigation")
    );
    w.run(&["--compact-stack"]).unwrap();
    for (number, pr) in &w.0.lock().unwrap().prs {
        assert_eq!(
            before[number]["body"].as_str().unwrap(),
            pr["body"]
                .as_str()
                .unwrap()
                .replace("---\n\n<!--", "---\n<!--")
        );
    }
}

#[test]
fn command_deadlines_validate_git_values_and_cli_overrides() {
    let w = World::new(0);
    for (flag, key) in [
        ("read-timeout", "git-pr.readTimeout"),
        ("command-timeout", "git-pr.commandTimeout"),
    ] {
        w.0.lock().unwrap().config.insert(key.into(), "13".into());
        w.run(&["--stop-after=validate"]).unwrap();
        for value in ["", "0", "-1", "bad", "18446744073709551616"] {
            assert!(
                w.run(&["--stop-after=validate", &format!("--{flag}={value}")])
                    .unwrap_err()
                    .to_string()
                    .contains("positive")
            );
        }
        w.run(&["--stop-after=validate", &format!("--{flag}=7")])
            .unwrap();
        w.0.lock().unwrap().config.insert(key.into(), "bad".into());
        assert!(w.run(&["--stop-after=validate"]).is_err());
        w.run(&["--stop-after=validate", &format!("--{flag}=7")])
            .unwrap();
        w.0.lock().unwrap().config.remove(key);
    }
}

#[test]
fn added_configuration_read_errors_are_propagated() {
    for args in [
        "--type=int --get git-pr.readTimeout",
        "--type=int --get git-pr.commandTimeout",
        "--type=bool --get git-pr.compactStack",
    ] {
        let w = World::new(0);
        w.fail(&format!("git config {args}"));
        assert!(w.run(&["--stop-after=validate"]).is_err());
    }
}

#[test]
fn nested_invocation_uses_repository_template() {
    for nested in [false, true] {
        let w = World::new(1);
        {
            let mut m = w.0.lock().unwrap();
            m.commits[0].message.clear();
            m.files.insert(
                "/unit/repo/.github/PULL_REQUEST_TEMPLATE.md".into(),
                "Repository template content".into(),
            );
        }
        let mut app = w.app();
        if nested {
            app.commands.cwd = "/unit/repo/subdir".into();
        }
        let flags = crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap();
        app.execute(
            &flags,
            &std::collections::BTreeMap::from([("HOME".into(), "/unit/home".into())]),
        )
        .unwrap();
        let m = w.0.lock().unwrap();
        let body = m.prs[&101]["body"].as_str().unwrap();
        assert!(body.contains("Repository template content"), "{body}");
        drop(m);
    }
}

#[test]
fn unmanaged_description_survives_adoption_and_repeat() {
    for existing in [
        "Manually written acceptance criteria and reviewer notes",
        "Manually written acceptance criteria\n---\n* ⬛ #101\n---\nBot notes",
    ] {
        let w = World::new(1);
        w.run(&["--no-stack"]).unwrap();
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] = json!(existing);
        w.run(&["--no-stack"]).unwrap();
        let m = w.0.lock().unwrap();
        let body = m.prs[&101]["body"].as_str().unwrap();
        assert!(
            body.contains("Manually written acceptance criteria"),
            "{body}"
        );
        assert!(!body.contains("Body 1"), "{body}");
        assert_eq!(body.contains("Bot notes"), existing.contains("Bot notes"));
        assert_eq!(body.matches(description::START).count(), 1);
        drop(m);
        w.0.lock().unwrap().trace.clear();
        w.run(&["--no-stack"]).unwrap();
        assert!(!w.trace().iter().any(|e| {
            e["body"]["query"]
                .as_str()
                .is_some_and(|q| q.starts_with("mutation"))
        }));
    }
}

#[test]
fn native_enablement_authorises_routine_linking_without_confirmation() {
    for (configured, args, enabled) in [
        (true, vec![], true),
        (false, vec!["--github-stack"], true),
        (true, vec!["--no-stack"], false),
        (true, vec!["--github-stack=false"], false),
        (true, vec!["--github-stack", "--no-stack"], false),
    ] {
        let world = World::new(2);
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert("git-pr.githubStack".into(), configured.to_string());
        world.run(&args).unwrap();
        assert!(!world.trace().iter().any(|e| e["kind"] == "confirm"));
        assert_eq!(
            world
                .trace()
                .iter()
                .filter(|e| e["path"] == "stacks" && e["method"] == "POST")
                .count(),
            usize::from(enabled)
        );
        world.0.lock().unwrap().trace.clear();
        world.run(&args).unwrap();
        assert!(!world.trace().iter().any(|e| e["kind"] == "confirm" || (e["path"] == "stacks" && e["method"] == "POST")));
    }
}
