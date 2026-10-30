use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};
use serde_json::json;

use super::support::{Fixture, Service};
use crate::{
    diagnostics::errors,
    git::Git,
    github::http::{Request, Transport},
    model::Repository,
    model::{Commit, REMOTE_REF},
    stack::description,
};

#[test]
fn merged_downstack_recovery_uses_real_ancestry_and_preserves_the_open_pr() {
    for method in ["merge", "squash", "rebase"] {
        for deleted in [false, true] {
            let f = Fixture::with_service(
                2,
                true,
                Service {
                    indirect_merges: Some(false),
                    ..Service::default()
                },
            );
            f.run(&["--no-stack"]).unwrap();
            f.git(&["checkout", "main"]);
            match method {
                "merge" => {
                    f.git(&["merge", "--no-ff", "topic/1", "-m", "Merge downstack"]);
                }
                "squash" => {
                    f.git(&["merge", "--squash", "topic/1"]);
                    f.git(&["commit", "-m", "Squash downstack"]);
                }
                "rebase" => {
                    f.git(&["cherry-pick", "topic/1"]);
                    f.git(&["commit", "--amend", "-m", "Rebase downstack"]);
                }
                _ => unreachable!(),
            }
            let merged = f.git(&["rev-parse", "main"]);
            f.git(&["push", "origin", "main"]);
            {
                let mut prs = f.service.prs.lock().unwrap();
                for (number, head) in [(101, &f.hashes[0]), (102, &f.hashes[1])] {
                    let pr = prs.get_mut(&number).unwrap();
                    pr["head"]["sha"] = json!(head);
                    pr["base"]["sha"] = json!(f.trunk);
                    pr["stack"] = serde_json::Value::Null;
                }
                let parent = prs.get_mut(&101).unwrap();
                parent["state"] = json!("closed");
                parent["merged"] = json!(true);
                parent["merge_commit_sha"] = json!(merged);
                parent["base"]["ref"] = json!("intermediate");
                parent["stack"] = json!({"number":9});
                drop(prs);
            }
            if deleted {
                f.git(&["push", "origin", "--delete", "topic/1"]);
            }
            f.git(&["checkout", "topic/2"]);
            f.git(&["rebase", "--onto", "main", "topic/1"]);
            let expected = f.git(&["rev-parse", "HEAD"]);
            f.service.trace.lock().unwrap().clear();
            f.run(&["--no-stack"]).unwrap();
            let trace = f.trace();
            let patch = trace
                .iter()
                .position(|e| e["method"] == "PATCH" && e["body"]["base"] == "main")
                .unwrap();
            let push = trace.iter().position(|e| e["args"][0] == "push").unwrap();
            assert!(patch < push);
            assert!(
                trace[push]["args"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(format!(
                        "--force-with-lease=refs/heads/topic/2:{}",
                        f.hashes[1]
                    )))
            );
            let prs = f.service.prs.lock().unwrap();
            assert_eq!(prs.len(), 2);
            assert_eq!(prs[&102]["state"], "open");
            assert_eq!(prs[&102]["merged"], false);
            assert_eq!(prs[&102]["base"]["ref"], "main");
            assert_eq!(prs[&102]["head"]["sha"], expected);
            drop(prs);
            assert!(
                f.git(&["ls-remote", "origin", "refs/heads/topic/2"])
                    .starts_with(&expected)
            );
        }
    }
}

#[test]
fn recovery_publication_lease_rejects_a_concurrent_remote_update() {
    let f = Fixture::new(2, true);
    f.git(&[
        "push",
        "origin",
        &format!("{}:refs/heads/topic/1", f.hashes[1]),
    ]);
    let repo = Repository {
        remote: "origin".into(),
        trunk: "main".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &f.app.commands,
        repo: &repo,
    };
    let commit = Commit {
        hash: f.hashes[0].clone(),
        attrs: vec![("remote-ref".into(), "topic/1".into())],
        ..Commit::default()
    };
    let leases = std::collections::BTreeMap::from([("topic/1".into(), Some(f.trunk.clone()))]);
    assert!(git.push_with_leases(&[&commit], Some(&leases)).is_err());
    assert!(
        f.git(&["ls-remote", "origin", "refs/heads/topic/1"])
            .starts_with(&f.hashes[1])
    );
}

#[test]
fn publication_requires_one_verified_repository_destination_before_effects() {
    let f = Fixture::new(1, true);
    let fetch = f.git(&["remote", "get-url", "origin"]);
    f.git(&["push", &fetch, "HEAD:refs/heads/topic/1"]);
    let first = f.temp.path().join("first.git");
    let second = f.temp.path().join("second.git");
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();
    for destination in [first, second] {
        f.git(&["init", "--bare", destination]);
        f.git(&["push", destination, "main:refs/heads/topic/1"]);
        f.git(&[
            "remote",
            "set-url",
            "--add",
            "--push",
            "origin",
            destination,
        ]);
    }
    // A lost response may conceal a successful write to only the first destination.
    f.git(&["push", "-f", first, "HEAD:refs/heads/topic/1"]);
    f.service.command_errors.lock().unwrap().insert(
        format!(
            "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1",
            f.hashes[0]
        ),
        (1, "connection lost during push".into()),
    );
    let error = f.run(&["--no-stack"]).unwrap_err();
    assert!(format!("{error:#}").contains("single verified push destination"));
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(
        f.git(&["ls-remote", second, "refs/heads/topic/1"])
            .starts_with(&f.trunk)
    );

    f.git(&["config", "--unset-all", "remote.origin.pushurl"]);
    f.git(&["remote", "set-url", "--push", "origin", second]);
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
    assert!(
        f.git(&["ls-remote", second, "refs/heads/topic/1"])
            .starts_with(&f.hashes[0])
    );
}

#[test]
fn existing_single_character_remote_ref_is_published_without_rewriting() {
    let f = Fixture::new(1, false);
    f.git(&["commit", "--amend", "-m", "Feature\n\nRemote-Ref: a"]);
    let original = f.git(&["rev-parse", "HEAD"]);
    f.run(&["--no-stack"]).unwrap();
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(f.git(&["rev-parse", "HEAD"]), original);
    assert!(
        f.git(&["ls-remote", "origin", "refs/heads/a"])
            .starts_with(&original)
    );
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[&101]["head"]["ref"], "a");
}

#[test]
fn single_character_tags_preserve_the_published_branch_and_label() {
    for footer in [
        "Remote-Ref: published-topic\nTags: x",
        "Tags: x\nRemote-Ref: published-topic",
    ] {
        let f = Fixture::new(1, false);
        f.git(&["commit", "--amend", "-m", &format!("Feature\n\n{footer}")]);
        let original = f.git(&["rev-parse", "HEAD"]);
        f.run(&["--no-stack"]).unwrap();
        f.run(&["--no-stack"]).unwrap();
        assert_eq!(f.git(&["rev-parse", "HEAD"]), original);
        assert!(
            f.git(&["ls-remote", "origin", "refs/heads/published-topic"])
                .starts_with(&original)
        );
        assert_eq!(f.git(&["ls-remote", "origin", "refs/heads/topic/1"]), "");
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[&101]["head"]["ref"], "published-topic");
        assert_eq!(prs[&101]["labels"], json!(["x"]));
    }
}

#[test]
fn trunk_discovery_ignores_shadowing_local_branches_and_tags() {
    for collisions in [
        vec!["refs/heads/origin/main"],
        vec!["refs/tags/origin/main"],
        vec!["refs/heads/origin/main", "refs/tags/origin/main"],
    ] {
        let f = Fixture::new(2, true);
        for reference in &collisions {
            f.git(&["update-ref", reference, &f.hashes[0]]);
        }
        f.run(&["--no-stack"]).unwrap();
        f.run(&["--no-stack"]).unwrap();
        assert_eq!(f.git(&["rev-parse", "HEAD"]), f.hashes[1]);
        assert_eq!(f.git(&["rev-parse", "refs/remotes/origin/main"]), f.trunk);
        for reference in &collisions {
            assert_eq!(f.git(&["rev-parse", reference]), f.hashes[0]);
        }
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[&101]["base"]["ref"], "main");
        assert_eq!(prs[&101]["head"]["ref"], "topic/1");
        assert_eq!(prs[&102]["base"]["ref"], "topic/1");
        assert_eq!(prs[&102]["head"]["ref"], "topic/2");
    }
}

#[test]
fn branchless_rewriting_preserves_single_character_and_pipe_branches() {
    for reference in ["a", "topic|one"] {
        let f = Fixture::with_service(
            1,
            false,
            Service {
                branchless: true,
                ..Service::default()
            },
        );
        f.git(&["branch", "-m", reference]);
        f.run(&["--no-stack", reference]).unwrap();
        let rewritten = f.git(&["rev-parse", "HEAD"]);
        assert_eq!(rewritten, f.hashes[0]);
        f.run(&["--no-stack", reference]).unwrap();
        assert_eq!(f.git(&["rev-parse", "HEAD"]), rewritten);
        assert!(
            !f.git(&["show", "-s", "--format=%B", "HEAD"])
                .contains("Remote-Ref")
        );
        assert!(
            f.git(&["ls-remote", "origin", &format!("refs/heads/{reference}")])
                .starts_with(&rewritten)
        );
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[&101]["head"]["ref"], reference);
    }
}

#[test]
fn pipe_branch_is_synchronised_without_moving_the_checked_out_branch() {
    let f = Fixture::new(1, true);
    f.git(&["branch", "topic|one", &f.trunk]);
    Git {
        commands: &f.app.commands,
        repo: &Repository::default(),
    }
    .sync_refs(&[Commit {
        hash: f.hashes[0].clone(),
        attrs: vec![(REMOTE_REF.into(), "topic|one".into())],
        ..Commit::default()
    }]);
    assert_eq!(f.git(&["rev-parse", "refs/heads/topic|one"]), f.hashes[0]);
    assert_eq!(f.git(&["rev-parse", "HEAD"]), f.hashes[0]);
    assert_eq!(f.git(&["status", "--porcelain"]), "");
}

fn hydrate_publication_heads(f: &Fixture) {
    let observed = f.service.prs.lock().unwrap().clone();
    for (number, original) in observed {
        let head = f.git(&[
            "rev-parse",
            &format!(
                "refs/remotes/origin/{}",
                original["head"]["ref"].as_str().unwrap()
            ),
        ]);
        let mut prs = f.service.prs.lock().unwrap();
        let pr = prs.get_mut(&number).unwrap();
        pr["head"]["sha"] = json!(head);
        pr["base"]["sha"] = json!(f.trunk);
        pr["merged_at"] = serde_json::Value::Null;
        pr["merge_commit_sha"] = serde_json::Value::Null;
        if pr["stack"].is_null() {
            pr["stack"] = serde_json::Value::Null;
        }
        drop(prs);
    }
}

fn advance_remote_trunk(commands: &crate::commands::Commands, remote: &str) -> Result<String> {
    let repository = format!("--git-dir={remote}");
    let old = commands.git(&[&repository, "rev-parse", "refs/heads/main"])?;
    let tree = commands.git(&[&repository, "rev-parse", &format!("{old}^{{tree}}")])?;
    // Create the object only in the bare remote. The publishing checkout must
    // complete without fetching this unrelated integration commit.
    let new = commands.git(&[
        &repository,
        "-c",
        "user.name=Independent Author",
        "-c",
        "user.email=independent@example.com",
        "commit-tree",
        &tree,
        "-p",
        &old,
        "-m",
        "Independent integration change",
    ])?;
    commands.git(&[&repository, "update-ref", "refs/heads/main", &new, &old])?;
    Ok(new)
}

#[derive(Debug)]
struct AdvanceTrunkAfterMutation {
    service: Arc<Service>,
    commands: crate::commands::Commands,
    remote: String,
    phase: &'static str,
    advanced: AtomicBool,
}

impl Transport for AdvanceTrunkAfterMutation {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let body: serde_json::Value = request
            .body
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        let trigger = match self.phase {
            "retarget" => request.method == "PATCH" && body.get("base").is_some(),
            "description" => {
                request.method == "POST"
                    && body["query"]
                        .as_str()
                        .is_some_and(|query| query.contains("updatePullRequest"))
            }
            "linked" => request.method == "POST" && request.url.ends_with("/stacks"),
            _ => false,
        };
        let response = self.service.request(request)?;
        if trigger && !self.advanced.swap(true, Ordering::SeqCst) {
            advance_remote_trunk(&self.commands, &self.remote)?;
        }
        Ok(response)
    }
}

#[test]
fn independent_trunk_changes_do_not_invalidate_stack_publication_or_cleanup() {
    for native in [false, true] {
        for phase in ["before", "cached", "retarget", "description", "linked"] {
            if phase == "linked" && !native {
                continue;
            }
            let mut f = Fixture::with_service(
                3,
                true,
                Service {
                    indirect_merges: Some(true),
                    ..Service::default()
                },
            );
            let flags = [
                if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                "--yes",
            ];
            f.run(&flags).unwrap();
            hydrate_publication_heads(&f);
            f.git(&["checkout", "--detach", &f.trunk]);
            for index in [1, 0, 2] {
                f.git(&["cherry-pick", &f.hashes[index]]);
            }
            // Moving local main is also irrelevant: selection uses the cached
            // remote-tracking base, and publication must leave local main alone.
            f.git(&["update-ref", "refs/heads/main", &f.hashes[0]]);
            let remote = f
                .temp
                .path()
                .join("remote.git")
                .to_str()
                .unwrap()
                .to_owned();
            if matches!(phase, "before" | "cached") {
                advance_remote_trunk(&f.app.commands, &remote).unwrap();
            }
            if phase == "cached" {
                // Even when the new integration commit is already available,
                // a bottom PR based on the older trunk needs no forced rebase.
                f.git(&["fetch", "origin", "main:refs/remotes/origin/main"]);
            }
            let movement = Arc::new(AdvanceTrunkAfterMutation {
                service: f.service.clone(),
                commands: f.app.commands.clone(),
                remote,
                phase,
                advanced: AtomicBool::new(false),
            });
            f.app.transport = movement.clone();
            f.service.trace.lock().unwrap().clear();
            f.run(&flags).unwrap();
            assert_eq!(
                movement.advanced.load(Ordering::SeqCst),
                !matches!(phase, "before" | "cached"),
                "native={native}, phase={phase}"
            );
            let remote_trunk = f.git(&["ls-remote", "origin", "refs/heads/main"]);
            let remote_trunk = remote_trunk.split_whitespace().next().unwrap();
            assert_ne!(remote_trunk, f.trunk);
            assert_eq!(f.git(&["rev-parse", "refs/heads/main"]), f.hashes[0]);
            if phase != "cached" {
                assert!(
                    f.app
                        .commands
                        .git(&["cat-file", "-e", &format!("{remote_trunk}^{{commit}}")])
                        .is_err()
                );
            }
            let prs = f.service.prs.lock().unwrap();
            for (number, base) in [(102, "main"), (101, "topic/2"), (103, "topic/1")] {
                assert_eq!(prs[&number]["state"], "open");
                assert_eq!(prs[&number]["merged"], false);
                assert_eq!(prs[&number]["base"]["ref"], base);
                assert_eq!(prs[&number]["stack"].is_object(), native);
            }
            drop(prs);
            assert!(
                !f.git(&["ls-remote", "--heads", "origin"])
                    .contains("git-pr/transition/")
            );
            assert!(
                !f.terminal
                    .stderr
                    .lock()
                    .unwrap()
                    .contains("temporary-base cleanup can be retried")
            );
            // A retry converges with the same current branch targets.
            f.run(&flags).unwrap();
        }
    }
}

#[test]
fn reordered_stack_preserves_open_prs_with_the_hosts_indirect_merge_lifecycle() {
    for native in [false, true] {
        for delete in [false, true] {
            let f = Fixture::with_service(
                3,
                true,
                Service {
                    indirect_merges: Some(delete),
                    ..Service::default()
                },
            );
            let flags = [
                if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                "--yes",
            ];
            f.run(&flags).unwrap();
            hydrate_publication_heads(&f);
            let original = f.service.prs.lock().unwrap().clone();
            f.git(&["checkout", "--detach", &f.trunk]);
            for index in [1, 0, 2] {
                f.git(&["cherry-pick", &f.hashes[index]]);
            }
            f.service.trace.lock().unwrap().clear();
            f.run(&flags).unwrap();
            let prs = f.service.prs.lock().unwrap();
            for (number, base) in [(102, "main"), (101, "topic/2"), (103, "topic/1")] {
                assert_eq!(prs[&number]["state"], "open");
                assert_eq!(prs[&number]["merged"], false);
                assert_eq!(prs[&number]["node_id"], original[&number]["node_id"]);
                assert_eq!(prs[&number]["base"]["ref"], base);
            }
            drop(prs);
            assert!(
                !f.git(&["ls-remote", "--heads", "origin"])
                    .contains("git-pr/transition/")
            );
            assert!(
                !f.git(&["config", "--local", "--list"])
                    .contains("transitionjournal")
            );
            f.run(&flags).unwrap();
        }
    }
}

#[test]
fn unguarded_reordering_exercises_the_indirect_merge_oracle() {
    for delete in [false, true] {
        let f = Fixture::with_service(
            3,
            true,
            Service {
                indirect_merges: Some(delete),
                ..Service::default()
            },
        );
        f.run(&["--no-stack"]).unwrap();
        f.git(&["checkout", "--detach", &f.trunk]);
        let mut specs = Vec::new();
        for index in [1, 0, 2] {
            f.git(&["cherry-pick", &f.hashes[index]]);
            specs.push(format!(
                "{}:refs/heads/topic/{}",
                f.git(&["rev-parse", "HEAD"]),
                index + 1
            ));
        }
        f.git(&["push", "-f", "origin", &specs[0], &specs[1], &specs[2]]);
        assert_eq!(f.service.prs.lock().unwrap()[&102]["merged"], true);
        assert_eq!(
            !f.git(&["ls-remote", "--heads", "origin"])
                .contains("refs/heads/topic/2"),
            delete
        );
    }
}

#[test]
fn insertion_at_every_position_supports_missing_and_published_new_branches() {
    for position in 0..=3 {
        for published in [false, true] {
            let f = Fixture::with_service(
                3,
                true,
                Service {
                    indirect_merges: Some(true),
                    ..Service::default()
                },
            );
            f.run(&["--no-stack"]).unwrap();
            hydrate_publication_heads(&f);
            f.git(&["checkout", "--detach", &f.trunk]);
            for index in 0..=3 {
                if index == position {
                    f.write("inserted.txt", "synthetic insertion\n");
                    f.git(&["add", "inserted.txt"]);
                    f.git(&[
                        "commit",
                        "-m",
                        "Inserted layer\n\nRemote-Ref: topic/inserted",
                    ]);
                    if published {
                        f.git(&["push", "origin", "HEAD:refs/heads/topic/inserted"]);
                    }
                }
                if index < 3 {
                    f.git(&["cherry-pick", &f.hashes[index]]);
                }
            }
            f.run(&["--no-stack"]).unwrap();
            let mut order = vec!["topic/1", "topic/2", "topic/3"];
            order.insert(position, "topic/inserted");
            let prs = f.service.prs.lock().unwrap();
            assert_eq!(prs.len(), 4);
            for (index, head) in order.iter().enumerate() {
                let pr = prs.values().find(|pr| pr["head"]["ref"] == *head).unwrap();
                assert_eq!(pr["state"], "open");
                assert_eq!(
                    pr["base"]["ref"],
                    if index == 0 { "main" } else { order[index - 1] }
                );
            }
            drop(prs);
        }
    }
}

#[derive(Debug)]
struct RestWithoutMembership(Arc<Service>);

impl Transport for RestWithoutMembership {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let omit = request.method == "GET" && request.url.contains("/pulls/");
        let bytes = self.0.request(request)?;
        if !omit {
            return Ok(bytes);
        }
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
        value.as_object_mut().unwrap().remove("stack");
        Ok(serde_json::to_vec(&value)?)
    }
}

fn expand_two_prs(f: &Fixture) -> Vec<(String, String)> {
    f.git(&["checkout", "--detach", &f.trunk]);
    let mut order = Vec::new();
    for index in 0..9 {
        let head = match index {
            2 | 4 => {
                let old = usize::from(index == 4);
                f.git(&["cherry-pick", &f.hashes[old]]);
                format!("topic/{}", old + 1)
            }
            _ => {
                let file = format!("inserted-{index}.txt");
                f.write(&file, "New stack layer\n");
                f.git(&["add", &file]);
                let head = format!("topic/inserted-{index}");
                f.git(&[
                    "commit",
                    "-m",
                    &format!("New layer {index}\n\nRemote-Ref: {head}"),
                ]);
                head
            }
        };
        order.push((head, f.git(&["rev-parse", "HEAD"])));
    }
    order
}

#[test]
fn expanding_two_published_prs_to_nine_preserves_identity_and_publishes_all_new_layers() {
    for (existing_native, native) in [(false, false), (false, true), (true, true), (true, false)] {
        for delete in [false, true] {
            let mut f = Fixture::with_service(
                2,
                true,
                Service {
                    indirect_merges: Some(delete),
                    ..Service::default()
                },
            );
            let flags = [
                if native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                "--draft",
                "--yes",
            ];
            f.run(&[
                if existing_native {
                    "--github-stack"
                } else {
                    "--no-stack"
                },
                "--draft",
                "--yes",
            ])
            .unwrap();
            hydrate_publication_heads(&f);
            let original = f.service.prs.lock().unwrap().clone();
            let order = expand_two_prs(&f);
            assert_eq!(
                f.git(&["ls-remote", "--heads", "origin"]).lines().count(),
                3
            );
            f.app.transport = Arc::new(RestWithoutMembership(f.service.clone()));
            f.service.trace.lock().unwrap().clear();
            f.run(&flags).unwrap();
            let prs = f.service.prs.lock().unwrap().clone();
            assert_eq!(prs.len(), 9);
            for (index, (head, hash)) in order.iter().enumerate() {
                let pr = prs.values().find(|pr| pr["head"]["ref"] == *head).unwrap();
                assert_eq!(pr["state"], "open");
                assert_eq!(pr["merged"], false);
                assert_eq!(pr["draft"], true);
                assert_eq!(
                    pr["base"]["ref"],
                    if index == 0 {
                        "main"
                    } else {
                        &order[index - 1].0
                    }
                );
                assert_eq!(pr["stack"].is_object(), native);
                assert!(
                    f.git(&["ls-remote", "origin", &format!("refs/heads/{head}")])
                        .starts_with(hash)
                );
            }
            for number in [101, 102] {
                assert_eq!(prs[&number]["node_id"], original[&number]["node_id"]);
                assert!(prs[&number]["body"].as_str().unwrap().contains("Body"));
            }
            assert!(
                !f.git(&["ls-remote", "--heads", "origin"])
                    .contains("git-pr/transition/")
            );
            assert!(
                !f.git(&["config", "--local", "--list"])
                    .contains("transitionjournal")
            );
            f.service.trace.lock().unwrap().clear();
            f.run(&flags).unwrap();
            assert_eq!(f.service.prs.lock().unwrap().len(), 9);
            assert!(
                !f.trace()
                    .iter()
                    .any(|event| event["method"] == "POST" && event["path"] == "pulls")
            );
        }
    }
}

#[test]
fn rebased_expansion_recovers_a_dissolved_native_stack_before_any_branch_was_published() {
    let mut f = Fixture::with_service(
        2,
        true,
        Service {
            indirect_merges: Some(true),
            ..Service::default()
        },
    );
    f.run(&["--github-stack", "--draft", "--yes"]).unwrap();
    hydrate_publication_heads(&f);
    let original = f.service.prs.lock().unwrap().clone();
    let order = expand_two_prs(&f);
    let old_tip = f.git(&["rev-parse", "HEAD"]);
    f.app.transport = Arc::new(RestWithoutMembership(f.service.clone()));
    f.service.command_errors.lock().unwrap().insert(
        format!("git push --porcelain --no-follow-tags --force-with-lease=refs/heads/git-pr/transition/101: -- origin {}:refs/heads/git-pr/transition/101", f.trunk),
        (1, "interrupted before staging publication".into()),
    );
    let error = f.run(&["--github-stack", "--draft", "--yes"]).unwrap_err();
    assert!(format!("{error:#}").contains("interrupted before staging publication"));
    assert!(
        f.service
            .prs
            .lock()
            .unwrap()
            .values()
            .all(|pr| pr["stack"].is_null())
    );
    assert_eq!(
        f.git(&["ls-remote", "--heads", "origin"]).lines().count(),
        3
    );
    f.git(&["checkout", "main"]);
    f.write("advanced-trunk.txt", "New trunk revision\n");
    f.git(&["add", "advanced-trunk.txt"]);
    f.git(&["commit", "-m", "Advance trunk"]);
    f.git(&["push", "origin", "main"]);
    f.git(&["checkout", "--detach", &old_tip]);
    f.git(&["rebase", "--onto", "main", &f.trunk]);
    assert_ne!(f.git(&["rev-parse", "HEAD"]), old_tip);
    let rebased = f.git(&["rev-list", "--reverse", "main..HEAD"]);
    let hashes = rebased.lines().collect::<Vec<_>>();
    assert_eq!(hashes.len(), 9);
    f.service.command_errors.lock().unwrap().clear();
    f.run(&["--no-stack", "--draft", "--yes"]).unwrap();
    let prs = f.service.prs.lock().unwrap().clone();
    assert_eq!(prs.len(), 9);
    for (index, (head, _)) in order.iter().enumerate() {
        let pr = prs.values().find(|pr| pr["head"]["ref"] == *head).unwrap();
        assert_eq!(pr["state"], "open");
        assert_eq!(pr["merged"], false);
        assert_eq!(pr["draft"], true);
        assert!(pr["stack"].is_null());
        assert_eq!(
            pr["base"]["ref"],
            if index == 0 {
                "main"
            } else {
                &order[index - 1].0
            }
        );
        assert_eq!(
            f.git(&["rev-parse", &format!("refs/remotes/origin/{head}")]),
            hashes[index]
        );
    }
    for number in [101, 102] {
        assert_eq!(prs[&number]["node_id"], original[&number]["node_id"]);
    }
    assert!(
        !f.git(&["config", "--local", "--list"])
            .contains("transitionjournal")
    );
    assert!(
        !f.git(&["ls-remote", "--heads", "origin"])
            .contains("git-pr/transition/")
    );
    f.run(&["--no-stack", "--draft", "--yes"]).unwrap();
    assert_eq!(f.service.prs.lock().unwrap().len(), 9);
}

#[test]
fn independent_stacks_retry_from_current_state_without_a_repository_journal() {
    for reverse in [false, true] {
        let mut f = Fixture::with_service(
            3,
            true,
            Service {
                indirect_merges: Some(true),
                ..Service::default()
            },
        );
        f.git(&[
            "config",
            "git-pr.transitionJournal",
            "obsolete ignored state",
        ]);
        f.run(&["--no-stack", "--yes"]).unwrap();
        hydrate_publication_heads(&f);
        let first = [2, 1, 3].map(|number| f.git(&["rev-parse", &format!("topic/{number}")]));
        f.git(&["checkout", "--detach", &f.trunk]);
        for hash in &first {
            f.git(&["cherry-pick", hash]);
        }
        let first_tip = f.git(&["rev-parse", "HEAD"]);
        let interrupt = |number| {
            format!(
                "git push --porcelain --no-follow-tags --force-with-lease=refs/heads/git-pr/transition/{number}: -- origin {}:refs/heads/git-pr/transition/{number}",
                f.trunk
            )
        };
        let first_push = interrupt(101);
        f.service
            .command_errors
            .lock()
            .unwrap()
            .insert(first_push, (1, "interrupted transition".into()));
        assert!(f.run(&["--no-stack", "--yes"]).is_err());
        f.service.command_errors.lock().unwrap().clear();
        f.git(&["checkout", "main"]);
        for number in 4..=6 {
            f.add_commit(number, true, "fixture@example.com");
        }
        f.run(&["--no-stack", "--yes"]).unwrap();
        hydrate_publication_heads(&f);
        let identities = f.service.prs.lock().unwrap().clone();
        let second = [5, 4, 6].map(|number| f.git(&["rev-parse", &format!("topic/{number}")]));
        f.git(&["checkout", "--detach", &f.trunk]);
        for hash in &second {
            f.git(&["cherry-pick", hash]);
        }
        let second_tip = f.git(&["rev-parse", "HEAD"]);
        let second_push = format!(
            "git push --porcelain --no-follow-tags --force-with-lease=refs/heads/git-pr/transition/104: -- origin {}:refs/heads/git-pr/transition/104",
            f.trunk
        );
        f.service
            .command_errors
            .lock()
            .unwrap()
            .insert(second_push, (1, "interrupted transition".into()));
        assert!(f.run(&["--no-stack", "--yes"]).is_err());
        f.service.command_errors.lock().unwrap().clear();
        let tips = if reverse {
            [&second_tip, &first_tip]
        } else {
            [&first_tip, &second_tip]
        };
        for tip in tips {
            f.git(&["checkout", "--detach", tip]);
            f.run(&["--no-stack", "--yes"]).unwrap();
        }
        let prs = f.service.prs.lock().unwrap();
        for (number, base) in [
            (102, "main"),
            (101, "topic/2"),
            (103, "topic/1"),
            (105, "main"),
            (104, "topic/5"),
            (106, "topic/4"),
        ] {
            assert_eq!(prs[&number]["state"], "open");
            assert_eq!(prs[&number]["merged"], false);
            assert_eq!(prs[&number]["node_id"], identities[&number]["node_id"]);
            assert_eq!(prs[&number]["base"]["ref"], base);
        }
        drop(prs);
        assert!(
            !f.git(&["ls-remote", "--heads", "origin"])
                .contains("git-pr/transition/")
        );
        assert_eq!(
            f.git(&["config", "--get", "git-pr.transitionJournal"]),
            "obsolete ignored state"
        );
    }
}

#[test]
fn failed_worktree_discovery_preserves_checked_out_refs_and_index() {
    let f = Fixture::new(1, true);
    let old = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "-b", "new-source"]);
    f.write("new.txt", "new\n");
    f.git(&["add", "new.txt"]);
    f.git(&["commit", "-m", "New descendant"]);
    let new = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "topic/1"]);
    f.service.command_errors.lock().unwrap().insert(
        "git worktree list --porcelain".into(),
        (1, "discovery failed".into()),
    );
    Git {
        commands: &f.app.commands,
        repo: &Repository::default(),
    }
    .sync_refs(&[Commit {
        hash: new,
        attrs: vec![(REMOTE_REF.into(), "topic/1".into())],
        ..Commit::default()
    }]);
    assert_eq!(f.git(&["rev-parse", "refs/heads/topic/1"]), old);
    assert_eq!(f.git(&["rev-parse", "HEAD"]), old);
    assert_eq!(f.git(&["status", "--porcelain"]), "");
    assert!(
        f.terminal
            .stderr
            .lock()
            .unwrap()
            .contains("local branches are unchanged")
    );
}

#[derive(Debug)]
struct TransientHistoryFailure {
    service: Arc<Service>,
    failed: AtomicBool,
}

impl Transport for TransientHistoryFailure {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        if request.method == "POST"
            && request.url.ends_with("/graphql")
            && !self.failed.swap(true, Ordering::SeqCst)
        {
            bail!("transient history unavailable");
        }
        self.service.request(request)
    }
}

#[test]
fn transient_history_read_failure_stops_patches_and_preserves_history_on_retry() {
    let mut f = Fixture::new(1, true);
    f.run(&["--no-stack"]).unwrap();
    let original = {
        let mut prs = f.service.prs.lock().unwrap();
        let body = prs[&101]["body"]
            .as_str()
            .unwrap()
            .replace(description::END, &format!("* #99\n{}", description::END));
        assert!(
            description::extract_history(&body)
                .iter()
                .any(|h| h.number == 99)
        );
        prs.get_mut(&101).unwrap()["body"] = json!(body);
        prs[&101]["body"].clone()
    };
    f.terminal.stdout.lock().unwrap().clear();
    f.service.trace.lock().unwrap().clear();
    let transport = Arc::new(TransientHistoryFailure {
        service: f.service.clone(),
        failed: AtomicBool::new(false),
    });
    f.app.transport = transport.clone();
    let error = f.run(&["--no-stack"]).unwrap_err();
    assert_eq!(errors::status(&error), 2);
    assert!(format!("{error:#}").contains("transient history unavailable"));
    assert!(transport.failed.load(Ordering::SeqCst));
    assert!(!f.trace().iter().any(|v| v["method"] == "PATCH"));
    assert_eq!(f.service.prs.lock().unwrap()[&101]["body"], original);
    f.run(&["--no-stack"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    let history = description::extract_history(prs[&101]["body"].as_str().unwrap());
    drop(prs);
    assert!(history.iter().any(|h| h.number == 99));
}

#[test]
fn failed_pr_lookup_stops_creation_instead_of_duplicating_an_existing_pr() {
    let f = Fixture::new(1, true);
    f.run(&["--no-stack"]).unwrap();
    f.service
        .http_errors
        .lock()
        .unwrap()
        .insert("POST graphql".into(), "lookup unavailable".into());
    f.service.trace.lock().unwrap().clear();
    let error = f.run(&["--no-stack"]).unwrap_err();
    assert_eq!(errors::status(&error), 2);
    assert!(format!("{error:#}").contains("lookup unavailable"));
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
    assert!(
        !f.trace()
            .iter()
            .any(|v| v["program"] == "gh" && v["args"][1] == "create")
    );
}

#[test]
fn overlapping_common_and_tip_labels_remain_on_every_pr() {
    let f = Fixture::new(2, true);
    f.run(&[
        "--no-stack",
        "--add-label=shared",
        "--add-tip-label=shared",
        "--add-tip-label=tip",
    ])
    .unwrap();
    {
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs[&101]["labels"], json!(["shared"]));
        assert_eq!(prs[&102]["labels"], json!(["shared", "tip"]));
    }
    f.run(&[
        "--no-stack",
        "--add-label=shared",
        "--add-tip-label=shared",
        "--add-tip-label=tip",
    ])
    .unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs[&101]["labels"], json!(["shared"]));
    assert_eq!(prs[&102]["labels"], json!(["shared", "tip"]));
}

#[test]
fn native_merge_and_empty_message_changes_prevent_submission() {
    for merge in [false, true] {
        let f = Fixture::new(1, true);
        if merge {
            f.git(&["checkout", "-b", "side", "main"]);
            f.write("side.txt", "side work\n");
            f.git(&["add", "."]);
            f.git(&["commit", "-m", "Side work"]);
            f.git(&["checkout", "topic/1"]);
            f.git(&["merge", "--no-commit", "--no-ff", "side"]);
        }
        f.write("additional.txt", "work which must not disappear\n");
        f.git(&["add", "."]);
        f.git(&[
            "commit",
            "--allow-empty-message",
            "-m",
            if merge { "Merge resolution" } else { "" },
        ]);
        let before = f.git(&["rev-parse", "HEAD"]);
        f.service.trace.lock().unwrap().clear();
        let error = f.run(&[]).unwrap_err();
        assert!(format!("{error:#}").contains(if merge {
            "merge commit"
        } else {
            "validation failed"
        }));
        assert_eq!(f.git(&["rev-parse", "HEAD"]), before);
        assert!(f.service.prs.lock().unwrap().is_empty());
        assert!(!f.trace().iter().any(|v| {
            v["kind"] == "http"
                || ["push", "reword", "describe"]
                    .iter()
                    .any(|arg| v["args"][0] == *arg)
        }));
    }
}

#[test]
fn native_batched_revision_resolution_preserves_requested_order() {
    let f = Fixture::new(3, true);
    let repo = Repository::default();
    let git = Git {
        commands: &f.app.commands,
        repo: &repo,
    };
    assert_eq!(
        git.at_depths("HEAD", &[2, 0, 1, 1]).unwrap(),
        [
            f.hashes[0].clone(),
            f.hashes[2].clone(),
            f.hashes[1].clone(),
            f.hashes[1].clone()
        ]
    );
    assert!(git.at_depths("HEAD", &[0, 100]).is_err());
}

#[test]
fn remote_ref_cannot_force_replace_the_trunk() {
    let f = Fixture::new(1, true);
    f.git(&["commit", "--amend", "-m", "Feature\n\nRemote-Ref: main"]);
    let feature = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "main"]);
    f.write("protected.txt", "already accepted trunk work\n");
    f.git(&["add", "."]);
    f.git(&["commit", "-m", "Advance trunk"]);
    let accepted = f.git(&["rev-parse", "HEAD"]);
    f.git(&["push", "origin", "main"]);
    f.git(&["checkout", "topic/1"]);
    let error = f.run(&["--no-stack"]).unwrap_err();
    assert!(format!("{error:#}").contains("remote trunk"));
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert_eq!(f.git(&["rev-parse", "HEAD"]), feature);
    let remote = f.git(&["ls-remote", "origin", "refs/heads/main"]);
    assert!(!remote.starts_with(&feature));
    assert!(remote.starts_with(&accepted));
    assert!(
        !f.git(&["ls-tree", "--name-only", &feature])
            .contains("protected.txt")
    );
}

#[test]
fn diverged_trunk_preserves_selected_commits_after_rewrite() {
    let f = Fixture::with_service(
        3,
        false,
        Service {
            branchless: true,
            ..Service::default()
        },
    );
    f.git(&["checkout", "main"]);
    f.write("upstream.txt", "new trunk work\n");
    f.git(&["add", "."]);
    f.git(&["commit", "-m", "Advance trunk"]);
    f.git(&["push", "origin", "main"]);
    f.git(&["checkout", "topic/3"]);
    f.run(&["--no-stack", "topic/1", "topic/3"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[&101]["head"]["ref"], "topic/1");
    assert_eq!(prs[&102]["head"]["ref"], "topic/3");
}

#[test]
fn signature_display_cannot_contaminate_metadata_or_launch_verifiers() {
    use std::os::unix::fs::PermissionsExt as _;
    let f = Fixture::new(1, true);
    let verifier = f.cwd.join("signature-verifier");
    f.write(
        "signature-verifier",
        "#!/bin/sh\necho signature-diagnostic >&2\ntouch verifier-ran\nexit 1\n",
    );
    std::fs::set_permissions(&verifier, std::fs::Permissions::from_mode(0o700)).unwrap();
    let old_trunk = f.git(&["rev-parse", "main"]);
    let mut new_trunk = String::new();
    for reference in ["main", "HEAD"] {
        let mut body = f.git(&["cat-file", "commit", reference]);
        if reference == "HEAD" {
            body = body.replace(&old_trunk, &new_trunk);
        }
        body = body.replacen(
            "\n\n",
            "\ngpgsig -----BEGIN PGP SIGNATURE-----\n invalid\n -----END PGP SIGNATURE-----\n\n",
            1,
        );
        f.write("signed-object", &body);
        let oid = f.git(&["hash-object", "-t", "commit", "-w", "signed-object"]);
        f.git(&[
            "update-ref",
            if reference == "main" {
                "refs/heads/main"
            } else {
                "HEAD"
            },
            oid.trim(),
        ]);
        if reference == "main" {
            new_trunk = oid.trim().to_owned();
        }
    }
    f.git(&["push", "--force", "origin", "main"]);
    f.git(&["config", "gpg.program", verifier.to_str().unwrap()]);
    f.git(&["config", "log.showSignature", "true"]);
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
    assert!(!f.cwd.join("verifier-ran").exists());
}

#[test]
fn empty_range_bases_preserve_explicit_selection_before_and_after_rewrite() {
    for rewrite in [false, true] {
        for range in [false, true] {
            let f = Fixture::with_service(
                1,
                true,
                Service {
                    branchless: rewrite,
                    ..Service::default()
                },
            );
            f.run(&["--no-stack"]).unwrap();
            f.git(&["commit", "--allow-empty", "-m", "Empty anchor"]);
            let base = f.git(&["rev-parse", "HEAD"]);
            f.git(&["checkout", "-b", "topic/2"]);
            f.write("second.txt", "selected change\n");
            f.git(&["add", "."]);
            f.git(&[
                "commit",
                "-m",
                if rewrite {
                    "Second feature"
                } else {
                    "Second feature\n\nRemote-Ref: topic/2"
                },
            ]);
            let selection = if range {
                format!("{base}..topic/2")
            } else {
                "topic/2".into()
            };
            f.run(&["--no-stack", &selection]).unwrap();
            let prs = f.service.prs.lock().unwrap();
            assert_eq!(prs.len(), 2);
            assert_eq!(prs[&102]["head"]["ref"], "topic/2");
            assert_eq!(prs[&102]["base"]["ref"], "topic/1");
        }
    }
}

#[test]
fn named_branch_rewrite_preserves_selection_from_another_checkout() {
    for range in [false, true] {
        let f = Fixture::with_service(
            2,
            false,
            Service {
                branchless: true,
                ..Service::default()
            },
        );
        f.git(&["tag", "topic/2"]);
        f.git(&["checkout", "main"]);
        let checkout = f.git(&["rev-parse", "HEAD"]);
        f.run(&[
            "--no-stack",
            if range {
                "refs/heads/topic/1..refs/heads/topic/2"
            } else {
                "refs/heads/topic/2"
            },
        ])
        .unwrap();
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[&101]["head"]["ref"], "topic/2");
        assert_eq!(f.git(&["rev-parse", "HEAD"]), checkout);
    }
}
#[test]
fn publication_does_not_follow_unselected_annotated_tags() {
    let f = Fixture::new(1, true);
    f.git(&["config", "push.followTags", "true"]);
    f.git(&[
        "tag",
        "-a",
        "unselected-tag",
        "-m",
        "Unselected annotated tag",
    ]);
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(
        f.git(&["ls-remote", "origin", "refs/tags/unselected-tag"]),
        ""
    );
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
}

#[test]
fn replacement_metadata_cannot_select_a_different_publication_branch() {
    for mode in [
        "loose",
        "packed",
        "custom",
        "disabled-empty",
        "disabled-zero",
    ] {
        let mut f = Fixture::new(1, true);
        if mode == "custom" {
            f.app.commands.env.insert(
                "GIT_REPLACE_REF_BASE".into(),
                "refs/custom-replacements/".into(),
            );
        }
        let original = f.git(&["rev-parse", "HEAD"]);
        let tree = f.git(&["rev-parse", "HEAD:"]);
        let replacement = f.git(&[
            "commit-tree",
            &tree,
            "-p",
            &f.trunk,
            "-m",
            "Replacement title\n\nRemote-Ref: topic/replacement",
        ]);
        f.git(&["replace", &original, &replacement]);
        if mode == "packed" {
            f.git(&["pack-refs", "--all"]);
        }
        if let Some(value) = mode.strip_prefix("disabled-") {
            f.app.commands.env.insert(
                "GIT_NO_REPLACE_OBJECTS".into(),
                if value == "empty" { "" } else { "0" }.into(),
            );
        }
        let environment = f.app.commands.env.clone();
        f.run(&["--no-stack"]).unwrap();
        assert_eq!(f.app.commands.env, environment);
        assert!(
            f.git(&["ls-remote", "origin", "refs/heads/topic/1"])
                .starts_with(&original)
        );
        assert_eq!(
            f.git(&["ls-remote", "origin", "refs/heads/topic/replacement"]),
            ""
        );
        let prs = f.service.prs.lock().unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[&101]["title"], "Feature 1");
        assert_eq!(prs[&101]["head"]["ref"], "topic/1");
    }
}

#[test]
fn a_replacement_cannot_conceal_actual_merge_parents() {
    for mode in ["replacement", "graft", "custom-graft"] {
        let mut f = Fixture::new(2, true);
        let tree = f.git(&["rev-parse", "HEAD:"]);
        let merge = f.git(&[
            "commit-tree",
            &tree,
            "-p",
            &f.hashes[1],
            "-p",
            &f.trunk,
            "-m",
            "Merge feature\n\nRemote-Ref: topic/merge",
        ]);
        f.git(&["reset", "--hard", &merge]);
        if mode == "replacement" {
            let replacement = f.git(&[
                "commit-tree",
                &tree,
                "-p",
                &f.hashes[1],
                "-m",
                "Linear view\n\nRemote-Ref: topic/merge",
            ]);
            f.git(&["replace", &merge, &replacement]);
        } else {
            let graft = format!("{merge} {}\n", f.hashes[1]);
            if mode == "graft" {
                f.write(".git/info/grafts", &graft);
            } else {
                let path = f.temp.path().join("custom-grafts");
                std::fs::write(&path, &graft).unwrap();
                f.app
                    .commands
                    .env
                    .insert("GIT_GRAFT_FILE".into(), path.into_os_string());
            }
        }
        assert_eq!(
            f.git(&[
                "-c",
                "advice.graftFileDeprecated=false",
                "show",
                "-s",
                "--format=%P",
                &merge
            ])
            .split_whitespace()
            .count(),
            1
        );
        let error = f.run(&["--no-stack"]).unwrap_err();
        assert!(format!("{error:#}").contains("merge commit"));
        assert!(f.service.prs.lock().unwrap().is_empty());
        assert_eq!(
            f.git(&["ls-remote", "origin", "refs/heads/topic/merge"]),
            ""
        );
    }
}

#[test]
fn publication_accepts_a_named_remote_beginning_with_a_dash() {
    let f = Fixture::new(1, true);
    f.git(&["remote", "rename", "--", "origin", "--publish"]);
    f.git(&["config", "remote.pushDefault", "--publish"]);
    f.service.command_errors.lock().unwrap().insert(
        "git remote -v".into(),
        (0, "--publish\thttps://github.com/org/repo.git (fetch)\n--publish\thttps://github.com/org/repo.git (push)".into()),
    );
    f.run(&["--no-stack"]).unwrap();
    assert!(
        f.git(&["ls-remote", "--", "--publish", "refs/heads/topic/1"])
            .starts_with(&f.hashes[0])
    );
    f.service.command_errors.lock().unwrap().insert(
        format!(
            "git push --porcelain --no-follow-tags -f -- --publish {}:refs/heads/topic/1",
            f.hashes[0]
        ),
        (1, "connection lost after publication".into()),
    );
    f.run(&["--no-stack"]).unwrap();
    assert!(f.trace().iter().any(|event| {
        event["args"] == json!(["remote", "get-url", "--push", "--all", "--", "--publish"])
    }));
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
}
