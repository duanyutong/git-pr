use super::support::Fixture;
use crate::config::files::{Files, NativeFiles};
use crate::github::http::{HttpStatusError, Request, Transport};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

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
