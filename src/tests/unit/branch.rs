use super::candidate;
use crate::{cli::Flags, tests::unit::support::World};

fn standalone(count: u64) -> std::sync::Arc<World> {
    let world = World::new(count);
    {
        let mut memory = world.0.lock().unwrap();
        for commit in &mut memory.commits {
            commit.attrs.clear();
            commit.author_email = "automation[bot]@example.com".into();
        }
        memory.files.clear();
        memory.token.clear();
    }
    world
}

#[test]
fn standalone_branch_retains_foreign_authorship_without_pr_boundaries() {
    for branchless in [false, true] {
        let world = standalone(1);
        world.0.lock().unwrap().branchless = branchless;
        for attempt in 1..=2 {
            world.run(&[]).unwrap();
            let trace = world.trace();
            let pushes: Vec<_> = trace
                .iter()
                .filter(|event| event["args"][0] == "push")
                .collect();
            assert_eq!(pushes.len(), attempt);
            for push in pushes {
                assert_eq!(
                    push["args"],
                    serde_json::json!([
                        "push",
                        "--porcelain",
                        "--no-follow-tags",
                        "--set-upstream",
                        "--",
                        "origin",
                        "refs/heads/topic/1:refs/heads/topic/1"
                    ])
                );
            }
            assert!(!trace.iter().any(|event| event["kind"] == "http"
                || event["kind"] == "credential"
                || event["kind"] == "file"));
            let memory = world.0.lock().unwrap();
            assert!(memory.prs.is_empty());
            assert_eq!(memory.commits[0].attrs, []);
            assert!(!memory.stdout.contains("skip "));
            drop(memory);
        }
    }
}

#[test]
fn managed_branch_metadata_does_not_select_ordinary_publication() {
    let world = World::new(1);
    let config = world.config();
    assert!(
        candidate(&config, &Flags::default(), &world.commands())
            .unwrap()
            .is_none()
    );
}

#[test]
fn standalone_multicommit_branch_pushes_once_and_preserves_every_object() {
    let world = standalone(3);
    world.reply(
        "git branch --format=%(refname:strip=2)|%(objectname)",
        0,
        &format!("topic/3|{:040x}", 3),
    );
    let original = world.0.lock().unwrap().commits.clone();
    world.run(&[]).unwrap();
    let memory = world.0.lock().unwrap();
    assert_eq!(memory.commits, original);
    assert_eq!(
        memory
            .trace
            .iter()
            .filter(|event| event["args"][0] == "push")
            .count(),
        1
    );
    assert!(memory.prs.is_empty());
    drop(memory);
}

#[test]
fn standalone_push_output_redacts_remote_credentials() {
    let world = standalone(1);
    world.reply(
        "git push --porcelain --no-follow-tags --set-upstream -- origin refs/heads/topic/1:refs/heads/topic/1",
        0,
        "To https://automation:synthetic-secret@example.com/team/repo.git\nDone",
    );
    world.run(&[]).unwrap();
    let output = world.0.lock().unwrap().stdout.clone();
    assert!(output.contains("https://[redacted]@example.com/team/repo.git"));
    assert!(!output.contains("synthetic-secret"));
}

#[test]
fn standalone_branch_rejects_concurrent_publication_before_pushing() {
    let world = standalone(1);
    world.0.lock().unwrap().file_errors.insert(
        "/unit/repo/.git/git-pr.lock".into(),
        std::io::ErrorKind::WouldBlock,
    );
    let error = format!("{:#}", world.run(&[]).unwrap_err());
    assert!(error.contains("another submission is active"));
    assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
    world.0.lock().unwrap().file_errors.clear();
    world.run(&[]).unwrap();
    assert_eq!(
        world
            .trace()
            .iter()
            .filter(|event| event["args"][0] == "push")
            .count(),
        1
    );
}

#[test]
fn standalone_preview_and_checkpoints_respect_publication_boundary() {
    for (argument, pushes) in [
        ("--dry-run", 0),
        ("--stop-after=validate", 0),
        ("--stop-after=get-commits", 0),
        ("--stop-after=push", 1),
    ] {
        let world = standalone(1);
        world.run(&[argument]).unwrap();
        let memory = world.0.lock().unwrap();
        assert_eq!(
            memory
                .trace
                .iter()
                .filter(|event| event["args"][0] == "push")
                .count(),
            pushes
        );
        assert!(memory.prs.is_empty());
        if argument == "--dry-run" {
            assert!(memory.stdout.contains("Would push topic/1 to origin"));
        } else {
            assert!(
                memory
                    .stdout
                    .contains(argument.strip_prefix("--stop-after=").unwrap())
            );
        }
        if argument == "--stop-after=get-commits" {
            assert!(memory.stdout.contains("Feature 1"));
        }
        drop(memory);
    }
}

#[test]
fn explicit_managed_intent_and_associations_preserve_stack_workflow() {
    let world = World::new(1);
    world.0.lock().unwrap().commits[0].attrs.clear();
    let mut config = world.config();
    let commands = world.commands();
    for arguments in [
        vec!["--draft=false"],
        vec!["--github-stack"],
        vec!["--no-stack"],
        vec!["--include-other-authors"],
        vec!["--skip-draft"],
        vec!["--include-draft"],
        vec!["--draft-pattern=ready*"],
        vec!["--add-label=review"],
        vec!["--add-tip-label=review"],
        vec!["-t=review"],
        vec!["--compact-stack"],
        vec!["--branch-from-title"],
        vec!["--output={url}"],
    ] {
        let flags = Flags::parse(arguments.into_iter().map(str::to_owned).collect()).unwrap();
        assert!(candidate(&config, &flags, &commands).unwrap().is_none());
    }
    config.selection.explicit = true;
    assert!(
        candidate(&config, &Flags::default(), &commands)
            .unwrap()
            .is_none()
    );
    config.selection.explicit = false;
    config.repo.jj = true;
    assert!(
        candidate(&config, &Flags::default(), &commands)
            .unwrap()
            .is_none()
    );
    config.repo.jj = false;
    for checkpoint in [
        crate::config::checkpoint::Checkpoint::Rewrite,
        crate::config::checkpoint::Checkpoint::PrCreate,
    ] {
        config.stop_after = Some(checkpoint);
        assert!(
            candidate(&config, &Flags::default(), &commands)
                .unwrap()
                .is_none()
        );
    }
    config.stop_after = None;
    for field in [
        "gitPrHead",
        "gitPrPullRequest",
        "gitPrChangeId",
        "github-pr-owner-number",
    ] {
        let key = format!("branch.topic/1.{field}");
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert(key.clone(), "retained association".into());
        assert!(
            candidate(&config, &Flags::default(), &commands)
                .unwrap()
                .is_none()
        );
        world.0.lock().unwrap().config.remove(&key);
    }
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert("git-pr.pendingCreate".into(), "pending".into());
    assert!(
        candidate(&config, &Flags::default(), &commands)
            .unwrap()
            .is_none()
    );
}

#[test]
fn standalone_requires_a_single_checked_out_anchor_and_allows_unrelated_branches() {
    let world = World::new(2);
    for commit in &mut world.0.lock().unwrap().commits {
        commit.attrs.clear();
    }
    let config = world.config();
    let commands = world.commands();
    assert!(
        candidate(&config, &Flags::default(), &commands)
            .unwrap()
            .is_none()
    );
    for branch in ["", "main", "missing"] {
        world.0.lock().unwrap().branch = branch.into();
        assert!(
            candidate(&config, &Flags::default(), &commands)
                .unwrap()
                .is_none()
        );
    }
    world.0.lock().unwrap().branch = "topic/2".into();
    world.reply(
        "git branch --format=%(refname:strip=2)|%(objectname)",
        0,
        &format!("topic/2|{:040x}\nmain|{:040x}", 2, 0),
    );
    assert_eq!(
        candidate(&config, &Flags::default(), &commands)
            .unwrap()
            .as_deref(),
        Some("topic/2")
    );
    world.reply(
        "git branch --format=%(refname:strip=2)|%(objectname)",
        0,
        &format!("topic/2|{:040x}\nalias|{:040x}", 2, 2),
    );
    assert!(
        candidate(&config, &Flags::default(), &commands)
            .unwrap()
            .is_none()
    );
}

#[test]
fn standalone_observation_and_push_failures_retain_diagnostics() {
    for command in [
        "git branch --show-current",
        "git branch --format=%(refname:strip=2)|%(objectname)",
        "git config --null --show-scope --show-origin --includes --get-regexp ^(branch\\..*\\.(gitprhead|gitprpullrequest|gitprchangeid|github-pr-owner-number|remote|merge)|git-pr\\.pendingcreate)$",
    ] {
        let world = standalone(1);
        world.fail(command);
        assert!(world.run(&[]).is_err());
        assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
    }
    let world = standalone(1);
    world.reply("git push --porcelain --no-follow-tags --set-upstream -- origin refs/heads/topic/1:refs/heads/topic/1", 1, "non-fast-forward");
    let error = format!("{:#}", world.run(&[]).unwrap_err());
    assert!(error.contains("branch push failed") && error.contains("non-fast-forward"));
    assert!(
        world
            .0
            .lock()
            .unwrap()
            .stderr
            .contains("Unconfirmed: branch publication")
    );
    let world = standalone(1);
    world.0.lock().unwrap().branch = "bad..branch".into();
    assert!(format!("{:#}", world.run(&[]).unwrap_err()).contains("invalid branch"));
}

#[test]
fn standalone_history_accepts_merges_and_empty_branches_and_rejects_invalid_observations() {
    const HISTORY: &str = "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..HEAD";
    let world = standalone(1);
    let message = crate::tests::unit::support::log(&world.0.lock().unwrap().commits).replace(
        &format!("{:040x}\0Feature", 0),
        &format!("{:040x} {:040x}\0Feature", 0, 2),
    );
    world.reply(HISTORY, 0, &message);
    world.run(&[]).unwrap();
    let world = standalone(1);
    world.reply(HISTORY, 0, "");
    world.run(&[]).unwrap();
    let world = standalone(1);
    world.reply(HISTORY, 0, "malformed");
    assert!(format!("{:#}", world.run(&[]).unwrap_err()).contains("invalid commit identity"));
    let world = standalone(1);
    world.fail(HISTORY);
    assert!(
        format!("{:#}", world.run(&[]).unwrap_err()).contains("failed to find common ancestor")
    );
}
