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
                    recovery_destination: true,
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
fn failed_publication_verifies_every_push_destination_instead_of_the_fetch_url() {
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
    assert!(format!("{error:#}").contains("failed to push 1 branch"));
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(
        f.git(&["ls-remote", second, "refs/heads/topic/1"])
            .starts_with(&f.trunk)
    );

    f.git(&["push", "-f", second, "HEAD:refs/heads/topic/1"]);
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
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
        assert_ne!(rewritten, f.hashes[0]);
        f.run(&["--no-stack", reference]).unwrap();
        assert_eq!(f.git(&["rev-parse", "HEAD"]), rewritten);
        assert!(
            f.git(&["show", "-s", "--format=%B", "HEAD"])
                .contains(&format!("Remote-Ref: {reference}"))
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

#[test]
fn reordered_stack_cannot_reach_the_hosts_indirect_merge_transition() {
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
            let remote_before = f.git(&["ls-remote", "--heads", "origin"]);
            let prs_before = f.service.prs.lock().unwrap().clone();
            f.git(&["checkout", "--detach", &f.trunk]);
            let mut rewritten = Vec::new();
            for index in [1, 0, 2] {
                f.git(&["cherry-pick", &f.hashes[index]]);
                rewritten.push(f.git(&["rev-parse", "HEAD"]));
            }
            f.service.trace.lock().unwrap().clear();
            let error = format!("{:#}", f.run(&flags).unwrap_err());
            assert!(error.contains("unsafe publication: PR #102"), "{error}");
            assert!(!f.trace().iter().any(|e| e["args"][0] == "push"));
            assert_eq!(*f.service.prs.lock().unwrap(), prs_before);
            assert_eq!(f.git(&["ls-remote", "--heads", "origin"]), remote_before);

            // Exercise the oracle: the legacy ordering really does merge PR #102.
            // Every ref and service state below belongs to this disposable fixture.
            let specs: Vec<_> = [2, 1, 3]
                .iter()
                .zip(&rewritten)
                .map(|(branch, oid)| format!("{oid}:refs/heads/topic/{branch}"))
                .collect();
            f.git(&["push", "-f", "origin", &specs[0], &specs[1], &specs[2]]);
            assert_eq!(f.service.prs.lock().unwrap()[&102]["merged"], true);
            assert_eq!(
                !f.git(&["ls-remote", "--heads", "origin"])
                    .contains("refs/heads/topic/2"),
                delete
            );
            f.service.trace.lock().unwrap().clear();
            let error = format!("{:#}", f.run(&flags).unwrap_err());
            assert!(error.contains("is merged"), "{error}");
            assert!(
                !f.trace().iter().any(|e| e["args"][0] == "push"
                    || e["method"] == "PATCH"
                    || e["path"] == "pulls")
            );
            assert_eq!(f.service.prs.lock().unwrap().len(), 3);
        }
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
