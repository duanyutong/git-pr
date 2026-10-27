//! Real-Git oracles and disposable-repository invariants for publication boundaries.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, bail};
use serde_json::Value;

use super::support::{Fixture, Service};
use crate::{
    commands::{CommandOutput, CommandRunner},
    git::Git,
    model::{Commit, REMOTE_REF, Repository},
    stack::selection,
};

const REF_SEED: u64 = 0x6a09_e667_f3bc_c909;

fn sample_count() -> usize {
    std::env::var("GIT_PR_ASSURANCE_CASES")
        .map_or(8, |value| value.parse::<usize>().unwrap().clamp(1, 256))
}

fn generated_refs() -> Vec<String> {
    let atoms = [
        "a",
        "z9",
        "topic|one",
        "é",
        "工程",
        "dash-name",
        "under_score",
        "plus+name",
        "semi;colon",
        "with@sign",
        "dollar$name",
        "round(name)",
    ];
    let mut state = REF_SEED;
    (0..sample_count())
        .map(|index| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let atom = atoms[usize::try_from(state % u64::try_from(atoms.len()).unwrap()).unwrap()];
            format!("generated/{index}/{atom}")
        })
        .collect()
}

#[test]
fn generated_literal_refs_agree_with_git_and_preserve_discovery_publication_and_recovery() {
    let f = Fixture::new(1, true);
    let refs = generated_refs();
    for name in &refs {
        assert_eq!(
            f.git(&["check-ref-format", "--branch", name]),
            *name,
            "seed={REF_SEED:#x}, name={name:?}"
        );
        Git::validate_remote_ref(name).unwrap();
        f.git(&["update-ref", &format!("refs/heads/{name}"), &f.trunk]);
    }
    let inventory = f.git(&[
        "for-each-ref",
        "--format=%(refname:strip=2)|%(objectname)",
        "refs/heads/",
    ]);
    let discovered = selection::local_branches(&inventory);
    assert!(
        refs.iter()
            .any(|name| discovered.get(f.trunk.as_str()) == Some(&name.as_str()))
    );
    for name in &refs {
        let one = format!("{name}|{}", f.trunk);
        assert_eq!(
            selection::local_branches(&one).get(f.trunk.as_str()),
            Some(&name.as_str()),
            "{name:?}"
        );
        assert_eq!(
            selection::local_branch(&format!("{name}|{}", f.hashes[0]), &f.hashes[0]),
            Some(name.clone())
        );
    }
    let commits: Vec<_> = refs
        .iter()
        .map(|name| Commit {
            hash: f.hashes[0].clone(),
            attrs: vec![(REMOTE_REF.into(), name.clone())],
            ..Commit::default()
        })
        .collect();
    let repo = Repository {
        remote: "origin".into(),
        trunk: "main".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &f.app.commands,
        repo: &repo,
    };
    git.push_all(&commits.iter().collect::<Vec<_>>()).unwrap();
    git.sync_refs(&commits);
    git.sync_refs(&commits);
    let remote = f.git(&["ls-remote", "--refs", "origin"]);
    for name in &refs {
        assert_eq!(
            f.git(&["rev-parse", &format!("refs/heads/{name}")]),
            f.hashes[0]
        );
        assert!(
            remote
                .lines()
                .any(|line| line == format!("{}\trefs/heads/{name}", f.hashes[0]))
        );
    }
    assert_eq!(f.git(&["rev-parse", "HEAD"]), f.hashes[0]);
    assert_eq!(f.git(&["status", "--porcelain"]), "");
}

#[test]
fn literal_ref_acceptance_matches_git_at_reserved_and_punctuation_boundaries() {
    let f = Fixture::new(1, true);
    let names = [
        "",
        "a",
        "@",
        "HEAD",
        "head",
        "-topic",
        ".topic",
        "topic.",
        "topic..next",
        "topic@{1}",
        "a//b",
        "a/.b",
        "a/b.lock",
        "a/b.LOCK",
        "a/",
        "/a",
        "topic|one",
        "topic;one",
        "topic$name",
        "é",
        "工程",
        "a b",
        "a\tb",
        "a\nb",
        "a\\b",
        "a?b",
        "a[b",
        "a*b",
        "a:b",
        "a^b",
        "a~b",
    ];
    for name in names {
        let accepted_by_git = f
            .app
            .commands
            .git(&["check-ref-format", "--branch", name])
            .is_ok();
        assert_eq!(
            crate::git::policy::valid_branch_name(name),
            accepted_by_git,
            "literal name={name:?}"
        );
    }
}

#[test]
fn in_memory_remote_head_observations_match_real_git_for_each_supported_remote_name() {
    for remote in ["origin", "publish"] {
        let f = Fixture::new(1, true);
        if remote != "origin" {
            f.git(&["remote", "rename", "origin", remote]);
        }
        let world = super::unit::support::World::new(1);
        let reference = format!("refs/remotes/{remote}/HEAD");
        assert_eq!(
            world.commands().git(&["symbolic-ref", &reference]).unwrap(),
            f.git(&["symbolic-ref", &reference])
        );
        let actual_refs = f.git(&[
            "for-each-ref",
            "--format=%(refname:strip=2)|%(objectname)",
            "refs/heads/",
        ]);
        let fake_refs = world
            .commands()
            .git(&[
                "for-each-ref",
                "--format=%(refname:strip=2)|%(objectname)",
                "refs/heads/",
            ])
            .unwrap();
        let expected = world.0.lock().unwrap().commits[0].hash.clone();
        assert_eq!(
            selection::local_branch(&actual_refs, &f.hashes[0]),
            selection::local_branch(&fake_refs, &expected)
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LocalSnapshot {
    heads: String,
    tags: String,
    checkout: String,
    index: Vec<u8>,
}

fn snapshot(f: &Fixture) -> LocalSnapshot {
    LocalSnapshot {
        heads: f.git(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads/",
        ]),
        tags: f.git(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/tags/",
        ]),
        checkout: f.git(&["symbolic-ref", "HEAD"]),
        index: std::fs::read(f.cwd.join(".git/index")).unwrap(),
    }
}

#[test]
fn nested_default_branches_and_alternate_remotes_preserve_literal_identity() {
    for remote in ["origin", "publish"] {
        for trunk in ["main", "release/stable"] {
            let f = Fixture::new(2, true);
            if remote != "origin" {
                f.git(&["remote", "rename", "origin", remote]);
                f.git(&["config", "remote.pushDefault", remote]);
            }
            f.git(&[
                "update-ref",
                &format!("refs/remotes/{remote}/{trunk}"),
                &f.trunk,
            ]);
            f.git(&[
                "symbolic-ref",
                &format!("refs/remotes/{remote}/HEAD"),
                &format!("refs/remotes/{remote}/{trunk}"),
            ]);
            f.git(&["push", remote, &format!("{}:refs/heads/{trunk}", f.trunk)]);
            let collision = format!("{remote}/{trunk}");
            for namespace in ["heads", "tags"] {
                f.git(&[
                    "update-ref",
                    &format!("refs/{namespace}/{collision}"),
                    &f.hashes[0],
                ]);
            }
            // Only host identity is substituted; Git owns the named remote and refs.
            f.service.command_errors.lock().unwrap().insert("git remote -v".into(), (0, format!("{remote}\thttps://github.com/org/repo.git (fetch)\n{remote}\thttps://github.com/org/repo.git (push)")));
            let before = snapshot(&f);
            let remote_before = f.git(&["ls-remote", "--refs", remote]);
            f.run(&["--dry-run", "--no-stack"]).unwrap();
            assert_eq!(snapshot(&f), before);
            assert_eq!(f.git(&["ls-remote", "--refs", remote]), remote_before);
            assert!(f.service.prs.lock().unwrap().is_empty());
            for _ in 0..2 {
                f.run(&["--no-stack"]).unwrap();
            }
            assert_eq!(snapshot(&f), before);
            let prs = f.service.prs.lock().unwrap();
            assert_eq!(prs.len(), 2);
            assert_eq!(prs[&101]["base"]["ref"], trunk);
            assert_eq!(prs[&102]["base"]["ref"], "topic/1");
            assert_eq!(prs[&101]["head"]["ref"], "topic/1");
            assert_eq!(prs[&102]["head"]["ref"], "topic/2");
        }
    }
}

#[derive(Debug)]
struct LostPushResponse {
    service: Arc<Service>,
    lost: AtomicBool,
}

impl CommandRunner for LostPushResponse {
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
        cwd: &Path,
        env: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        let result = self.service.execute(program, args, cwd, env)?;
        if program == "git"
            && args.first().is_some_and(|arg| arg == "push")
            && result.status == 0
            && !self.lost.swap(true, Ordering::SeqCst)
        {
            bail!("injected lost acknowledgement after the real push");
        }
        Ok(result)
    }
}

#[test]
fn real_push_url_rewriting_and_explicit_push_urls_recover_lost_acknowledgements() {
    for explicit in [false, true] {
        let mut f = Fixture::new(1, true);
        let fetch = f.git(&["remote", "get-url", "origin"]);
        let push = f.temp.path().join("publication.git");
        let push = push.to_str().unwrap();
        f.git(&["init", "--bare", push]);
        if explicit {
            f.git(&["remote", "set-url", "--push", "origin", push]);
        } else {
            f.git(&[
                "config",
                &format!(
                    "url.{}.pushInsteadOf",
                    f.temp.path().join("wrong.git").display()
                ),
                fetch.strip_suffix(".git").unwrap(),
            ]);
            f.git(&["config", &format!("url.{push}.pushInsteadOf"), &fetch]);
        }
        assert_eq!(f.git(&["remote", "get-url", "origin"]), fetch);
        assert_eq!(
            f.git(&["remote", "get-url", "--push", "--all", "origin"]),
            push
        );
        let runner = Arc::new(LostPushResponse {
            service: f.service.clone(),
            lost: AtomicBool::new(false),
        });
        f.app.commands.runner = runner.clone();
        f.run(&["--no-stack"]).unwrap();
        assert!(runner.lost.load(Ordering::SeqCst));
        assert!(
            f.git(&["ls-remote", push, "refs/heads/topic/1"])
                .starts_with(&f.hashes[0])
        );
        assert_eq!(f.git(&["ls-remote", &fetch, "refs/heads/topic/1"]), "");
        f.run(&["--no-stack"]).unwrap();
        assert_eq!(f.service.prs.lock().unwrap().len(), 1);
        assert!(f.trace().iter().any(|event| event["args"][0] == "ls-remote"
            && event["args"][3].as_str().is_some_and(|url| {
                f.service
                    .resolved_destinations
                    .lock()
                    .unwrap()
                    .get(url)
                    .is_some_and(|target| target == push)
            })));
    }
}

#[test]
fn every_partial_push_destination_prefix_remains_uncertain_until_all_destinations_match() {
    for prefix in 0..=3 {
        let f = Fixture::new(1, true);
        let mut destinations = Vec::new();
        for index in 0..3 {
            let path = f.temp.path().join(format!("push-{index}.git"));
            let path = path.to_str().unwrap().to_owned();
            f.git(&["init", "--bare", &path]);
            f.git(&["remote", "set-url", "--add", "--push", "origin", &path]);
            if index < prefix {
                f.git(&["push", &path, "HEAD:refs/heads/topic/1"]);
            }
            destinations.push(path);
        }
        let oracle = f.git(&["remote", "get-url", "--push", "--all", "origin"]);
        assert_eq!(oracle.lines().collect::<Vec<_>>(), destinations);
        let inventory = f.git(&["-C", ".", "remote", "-v"]);
        f.service
            .command_errors
            .lock()
            .unwrap()
            .insert("git remote -v".into(), (0, inventory));
        let refused = format!("{:#}", f.run(&["--no-stack"]).unwrap_err());
        assert!(refused.contains("multiple push URLs"), "{refused}");
        assert!(f.service.prs.lock().unwrap().is_empty());
        let push_key = format!(
            "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1",
            f.hashes[0]
        );
        f.service
            .command_errors
            .lock()
            .unwrap()
            .insert(push_key.clone(), (1, "partial destination failure".into()));
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
            attrs: vec![(REMOTE_REF.into(), "topic/1".into())],
            ..Commit::default()
        };
        let result = git.push_all(&[&commit]);
        assert_eq!(result.is_ok(), prefix == 3, "prefix={prefix}: {result:?}");
        for (index, destination) in destinations.iter().enumerate() {
            let actual = f.git(&["ls-remote", destination, "refs/heads/topic/1"]);
            assert_eq!(!actual.is_empty(), index < prefix);
        }
        f.service.command_errors.lock().unwrap().remove(&push_key);
        git.push_all(&[&commit]).unwrap();
        for destination in &destinations {
            assert!(
                f.git(&["ls-remote", destination, "refs/heads/topic/1"])
                    .starts_with(&f.hashes[0])
            );
        }
    }
}

#[test]
fn refused_dirty_worktrees_preserve_index_unrelated_refs_and_all_remote_effects() {
    let f = Fixture::new(1, true);
    f.git(&["branch", "unrelated", &f.trunk]);
    f.git(&["tag", "unrelated", &f.trunk]);
    f.write("base.txt", "uncommitted human changes\n");
    f.git(&["add", "base.txt"]);
    let before = snapshot(&f);
    let remote = f.git(&["ls-remote", "--refs", "origin"]);
    let error = format!("{:#}", f.run(&["--no-stack"]).unwrap_err());
    assert!(
        error.contains("working tree") || error.contains("commit or stash"),
        "{error}"
    );
    assert_eq!(snapshot(&f), before);
    assert_eq!(f.git(&["ls-remote", "--refs", "origin"]), remote);
    assert_eq!(
        std::fs::read_to_string(f.cwd.join("base.txt")).unwrap(),
        "uncommitted human changes\n"
    );
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(!f.trace().iter().any(submission_effect));
}

#[test]
fn successful_submission_preserves_unrelated_worktree_index_and_human_files() {
    let f = Fixture::new(2, true);
    let linked = f.temp.path().join("human-worktree");
    let linked = linked.to_str().unwrap();
    f.git(&["branch", "unrelated", &f.trunk]);
    f.git(&["worktree", "add", linked, "unrelated"]);
    std::fs::write(Path::new(linked).join("base.txt"), "staged human change\n").unwrap();
    std::fs::write(
        Path::new(linked).join("private.txt"),
        "untracked human notes\n",
    )
    .unwrap();
    f.git(&["-C", linked, "add", "base.txt"]);
    let metadata = f.git(&["-C", linked, "rev-parse", "--absolute-git-dir"]);
    let index = Path::new(&metadata).join("index");
    let before_index = std::fs::read(&index).unwrap();
    let before = snapshot(&f);
    f.run(&["--no-stack"]).unwrap();
    f.run(&["--no-stack"]).unwrap();
    assert_eq!(snapshot(&f), before);
    assert_eq!(std::fs::read(&index).unwrap(), before_index);
    assert_eq!(
        f.git(&["-C", linked, "symbolic-ref", "HEAD"]),
        "refs/heads/unrelated"
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(linked).join("base.txt")).unwrap(),
        "staged human change\n"
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(linked).join("private.txt")).unwrap(),
        "untracked human notes\n"
    );
    assert_eq!(f.service.prs.lock().unwrap().len(), 2);
}

fn submission_effect(event: &Value) -> bool {
    event["args"][0] == "push" && event["args"][1] == "--porcelain"
        || event["method"] == "PATCH"
        || event["method"] == "DELETE"
        || event["method"] == "POST" && event["path"] != "graphql"
}
