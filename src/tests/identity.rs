use super::support::Fixture;
use crate::config::files::{Files, NativeFiles};
use crate::github::http::{HttpStatusError, Request, Transport};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn native_standalone_branch_pushes_original_bot_commits_and_sets_upstream_without_github_authentication()
 {
    for count in [1, 3] {
        let mut fixture = Fixture::new(0, false);
        for index in 1..=count {
            fixture.add_commit(index, false, "automation[bot]@example.com");
        }
        fixture.git(&[
            "commit",
            "--amend",
            "-m",
            "Dependency update\n\nSigned-off-by: Automation <automation[bot]@example.com>",
        ]);
        for index in 1..count {
            fixture.git(&["branch", "-D", &format!("topic/{index}")]);
        }
        fixture.git(&["config", "git-pr.githubStack", "true"]);
        fixture.git(&["config", "push.followTags", "true"]);
        fixture.git(&["tag", "-a", "unselected-tag", "-m", "Unselected tag"]);
        std::fs::remove_file(fixture.home.join(".config/gh/hosts.yml")).unwrap();
        let tip = fixture.git(&["rev-parse", "HEAD"]);
        let objects = fixture.git(&["log", "--format=%H %B", "origin/main..HEAD"]);
        for _ in 0..2 {
            fixture.service.trace.lock().unwrap().clear();
            fixture.run(&[]).unwrap();
            assert_eq!(fixture.git(&["rev-parse", "HEAD"]), tip);
            assert_eq!(
                fixture.git(&["log", "--format=%H %B", "origin/main..HEAD"]),
                objects
            );
            assert_eq!(fixture.git(&["rev-parse", "@{upstream}"]), tip);
            assert_eq!(
                fixture.git(&["config", "--get", &format!("branch.topic/{count}.remote")]),
                "origin"
            );
            assert_eq!(
                fixture.git(&["config", "--get", &format!("branch.topic/{count}.merge")]),
                format!("refs/heads/topic/{count}")
            );
            assert_eq!(
                fixture.git(&["ls-remote", "origin", "refs/tags/unselected-tag"]),
                ""
            );
            assert!(fixture.service.prs.lock().unwrap().is_empty());
            assert!(!fixture.trace().iter().any(|event| event["kind"] == "http"));
            assert!(
                !fixture
                    .git(&["config", "--local", "--list"])
                    .contains(".gitpr")
            );
        }
    }
}

#[test]
fn native_standalone_branch_retains_merge_history() {
    let fixture = Fixture::new(2, false);
    fixture.git(&["checkout", "-b", "side", "topic/1"]);
    fixture.write("side.txt", "Side branch\n");
    fixture.git(&["add", "side.txt"]);
    fixture.git(&["commit", "-m", "Side change"]);
    fixture.git(&["checkout", "topic/2"]);
    fixture.git(&["merge", "--no-ff", "side", "-m", "Merge side"]);
    fixture.git(&["branch", "-D", "side", "topic/1"]);
    let tip = fixture.git(&["rev-parse", "HEAD"]);
    let parents = fixture.git(&["show", "--format=%P", "--no-patch", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 2);
    fixture.run(&[]).unwrap();
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), tip);
    assert_eq!(fixture.git(&["rev-parse", "origin/topic/2"]), tip);
    assert!(fixture.service.prs.lock().unwrap().is_empty());
}

#[derive(Debug)]
struct UnpublishedDescendant {
    service: Arc<super::support::Service>,
    hash: String,
    requests: AtomicUsize,
}

impl Transport for UnpublishedDescendant {
    fn request(&self, request: Request<'_>) -> anyhow::Result<Vec<u8>> {
        if request.method == "GET"
            && reqwest::Url::parse(request.url)?.path()
                == format!("/repos/org/repo/commits/{}/pulls", self.hash)
        {
            self.requests.fetch_add(1, Ordering::SeqCst);
            return Err(HttpStatusError {
                url: request.url.into(),
                status: reqwest::StatusCode::UNPROCESSABLE_ENTITY,
                body: serde_json::json!({"message":format!("No commit found for SHA: {}", self.hash),"status":"422"}).to_string(),
                quota: crate::github::limits::Quota::default(),
            }.into());
        }
        self.service.request(request)
    }
}

#[test]
fn native_partial_stack_submission_preserves_an_unpublished_descendant() {
    for flag in ["--no-stack", "--github-stack"] {
        let mut f = Fixture::new(3, false);
        let objects: Vec<_> = f
            .hashes
            .iter()
            .map(|oid| f.git(&["cat-file", "commit", oid]))
            .collect();
        f.git(&["checkout", "topic/2"]);
        let transport = Arc::new(UnpublishedDescendant {
            service: f.service.clone(),
            hash: f.hashes[2].clone(),
            requests: AtomicUsize::new(0),
        });
        f.app.transport = transport.clone();
        for attempt in 1..=2 {
            f.run(&[flag]).unwrap();
            assert_eq!(transport.requests.load(Ordering::SeqCst), attempt);
            assert_eq!(f.git(&["rev-parse", "HEAD"]), f.hashes[1]);
            assert_eq!(f.git(&["ls-remote", "origin", "refs/heads/topic/3"]), "");
            assert!(
                !f.git(&["config", "--local", "--list"])
                    .contains("branch.topic/3.gitpr")
            );
            for (index, oid) in f.hashes.iter().enumerate() {
                assert_eq!(f.git(&["rev-parse", &format!("topic/{}", index + 1)]), *oid);
                assert_eq!(f.git(&["cat-file", "commit", oid]), objects[index]);
            }
            let prs = f.service.prs.lock().unwrap();
            assert_eq!(prs.len(), 2);
            for pr in prs.values() {
                assert_eq!(pr["state"], "open");
                assert!(pr["body"].as_str().unwrap().contains("Feature 3"));
            }
        }
        assert_eq!(
            f.trace()
                .iter()
                .filter(|event| event["method"] == "POST" && event["path"] == "pulls")
                .count(),
            2
        );
    }
}

#[test]
fn native_git_recovers_deleted_metadata_without_changing_stored_objects() {
    let f = Fixture::new(3, false);
    let objects: Vec<_> = f
        .hashes
        .iter()
        .map(|oid| f.git(&["cat-file", "commit", oid]))
        .collect();
    for removal in [
        None,
        Some("gitPrHead"),
        Some("gitPrPullRequest"),
        Some("all"),
    ] {
        if let Some(field) = removal {
            for i in 1..=3 {
                if field == "all" {
                    f.git(&[
                        "config",
                        "--local",
                        "--remove-section",
                        &format!("branch.topic/{i}"),
                    ]);
                } else {
                    f.git(&[
                        "config",
                        "--local",
                        "--unset-all",
                        &format!("branch.topic/{i}.{field}"),
                    ]);
                }
            }
        }
        f.run(&["--no-stack"]).unwrap();
        for (index, oid) in f.hashes.iter().enumerate() {
            let branch = format!("topic/{}", index + 1);
            assert_eq!(f.git(&["rev-parse", &branch]), *oid);
            assert_eq!(f.git(&["cat-file", "commit", oid]), objects[index]);
            assert_eq!(
                f.git(&[
                    "config",
                    "--local",
                    "--get",
                    &format!("branch.{branch}.gitPrHead")
                ]),
                format!("github.com/org/repo refs/heads/{branch}")
            );
            assert!(
                f.git(&["ls-remote", "origin", &format!("refs/heads/{branch}")])
                    .starts_with(oid)
            );
        }
        assert_eq!(f.service.prs.lock().unwrap().len(), 3);
    }
    let creates = f
        .trace()
        .iter()
        .filter(|e| e["method"] == "POST" && e["path"] == "pulls")
        .count();
    assert_eq!(creates, 3);
}

#[test]
fn native_git_branch_rename_preserves_the_original_publication() {
    let f = Fixture::new(1, false);
    f.run(&["--no-stack"]).unwrap();
    f.git(&["branch", "-m", "renamed"]);
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(
        f.git(&["config", "--local", "--get", "branch.renamed.gitPrHead"]),
        "github.com/org/repo refs/heads/topic/1"
    );
    assert_eq!(f.git(&["rev-parse", "renamed"]), f.hashes[0]);
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
    assert_eq!(f.git(&["ls-remote", "origin", "refs/heads/renamed"]), "");
}

#[test]
fn native_cleanup_retires_merged_orphans_and_preserves_includes_and_editor_settings() {
    let f = Fixture::new(1, false);
    f.run(&["--no-stack"]).unwrap();
    let number = {
        let mut prs = f.service.prs.lock().unwrap();
        let mut historical = prs.values().next().unwrap().clone();
        historical["number"] = serde_json::json!(901);
        historical["head"]["ref"] = serde_json::json!("retired");
        historical["state"] = serde_json::json!("closed");
        historical["merged"] = serde_json::json!(true);
        prs.insert(901, historical);
        "https://github.com/org/repo/pull/901"
    };
    f.git(&["branch", "retired"]);
    for anchor in ["retired", "included"] {
        f.git(&[
            "config",
            &format!("branch.{anchor}.gitPrHead"),
            "github.com/org/repo refs/heads/retired",
        ]);
        f.git(&[
            "config",
            &format!("branch.{anchor}.gitPrPullRequest"),
            number,
        ]);
        f.git(&[
            "config",
            &format!("branch.{anchor}.description"),
            "Editor notes",
        ]);
    }
    let included = f.temp.path().join("included.gitconfig");
    std::fs::write(
        &included,
        format!("[branch \"included\"]\n    gitPrPullRequest = {number}\n"),
    )
    .unwrap();
    f.git(&["config", "include.path", included.to_str().unwrap()]);
    f.git(&["update-ref", "-d", "refs/heads/retired"]);
    f.run(&["--no-stack"]).unwrap();
    let configuration = f.git(&["config", "--local", "--list"]);
    assert!(!configuration.contains("branch.retired.gitpr"));
    assert!(configuration.contains("branch.retired.description=Editor notes"));
    assert!(configuration.contains("branch.included.gitprhead="));
    assert!(std::fs::read_to_string(included).unwrap().contains(number));
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(f.git(&["config", "--local", "--list"]), configuration);
}

#[cfg(unix)]
#[test]
fn native_repeated_submission_does_not_run_pre_push_hooks() {
    use std::os::unix::fs::PermissionsExt as _;

    let f = Fixture::new(1, false);
    f.run(&["--no-stack"]).unwrap();
    let hook = f.cwd.join(".git/hooks/pre-push");
    std::fs::write(&hook, "#!/bin/sh\ncat > .git/pre-push-observation\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    f.run(&["--no-stack"]).unwrap();
    assert!(!f.cwd.join(".git/pre-push-observation").exists());
    f.git(&[
        "--git-dir=../remote.git",
        "update-ref",
        "-d",
        "refs/heads/topic/1",
    ]);
    f.run(&["--no-stack"]).unwrap();
    assert!(
        std::fs::read_to_string(f.cwd.join(".git/pre-push-observation"))
            .unwrap()
            .contains("refs/heads/topic/1")
    );
}

#[test]
fn common_repository_lock_is_exclusive_and_released_on_drop() {
    let mut f = Fixture::new(1, false);
    let lock = f.cwd.join(".git/git-pr.lock");
    let guard = NativeFiles.lock(&lock).unwrap();
    assert!(NativeFiles.lock(&lock).is_err());
    assert!(format!("{:#}", f.run(&["--no-stack"]).unwrap_err()).contains("another submission"));
    let worktree = f.temp.path().join("linked");
    f.git(&[
        "worktree",
        "add",
        "--detach",
        worktree.to_str().unwrap(),
        "topic/1",
    ]);
    f.cwd = worktree.clone();
    f.app.commands.cwd = worktree;
    assert!(format!("{:#}", f.run(&["--no-stack"]).unwrap_err()).contains("another submission"));
    drop(guard);
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(f.service.prs.lock().unwrap().len(), 1);
    assert!(NativeFiles.lock(&f.cwd.join("missing/lock")).is_err());
}

#[test]
fn native_attempt_identifiers_have_unique_uuid_shape() {
    let first = NativeFiles.attempt_id().unwrap();
    let second = NativeFiles.attempt_id().unwrap();
    assert_ne!(first, second);
    assert!(
        regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
            .unwrap()
            .is_match(&first)
    );
}
