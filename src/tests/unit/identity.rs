//! Independent identity, evidence and interruption invariants.

use super::{
    Attempt, Head, parse_review, repository, resolve,
    store::{self, Records},
};
use crate::{
    github::GitHub,
    model::REMOTE_REF,
    tests::unit::support::{World, commit},
};
use serde_json::json;
use std::collections::BTreeMap;

fn clean_stack(count: u64) -> std::sync::Arc<World> {
    let world = World::new(count);
    for commit in &mut world.0.lock().unwrap().commits {
        commit.attrs.clear();
    }
    world
}

fn config_writes(world: &World) -> usize {
    world
        .trace()
        .iter()
        .filter(|event| event["args"][0] == "config" && event["args"][1] == "--local")
        .count()
}

#[test]
fn new_stacks_and_repeat_runs_preserve_all_commit_objects() {
    for count in [1, 4, 40] {
        let world = clean_stack(count);
        let before = world.0.lock().unwrap().commits.clone();
        world.run(&["--no-stack"]).unwrap();
        {
            let memory = world.0.lock().unwrap();
            assert_eq!(memory.commits, before);
            assert_eq!(memory.prs.len(), usize::try_from(count).unwrap());
            for i in 1..=count {
                assert_eq!(
                    memory.config[&format!("branch.topic/{i}.gitPrHead")],
                    format!("github.com/org/repo refs/heads/topic/{i}")
                );
                assert_eq!(
                    memory.remote_refs[&format!("refs/heads/topic/{i}")],
                    commit(i).hash
                );
                assert!(
                    memory
                        .config
                        .contains_key(&format!("branch.topic/{i}.gitPrPullRequest"))
                );
            }
            assert!(!memory.config.contains_key("git-pr.pendingCreate"));
            drop(memory);
        }
        world.0.lock().unwrap().trace.clear();
        world.run(&["--no-stack"]).unwrap();
        assert_eq!(config_writes(&world), 0);
        assert_eq!(world.0.lock().unwrap().commits, before);
        assert!(!world.trace().iter().any(|e| matches!(
            e["args"][0].as_str(),
            Some("reword" | "describe" | "rebase")
        )));
    }
}

#[test]
fn every_subset_of_deleted_binding_fields_recovers_the_same_reviews() {
    for mask in 1..4 {
        let world = clean_stack(4);
        world.run(&["--no-stack"]).unwrap();
        let before = world.0.lock().unwrap().prs.clone();
        {
            let mut memory = world.0.lock().unwrap();
            memory.config.retain(|key, _| {
                !(mask & 1 != 0 && key.ends_with("gitPrHead")
                    || mask & 2 != 0 && key.ends_with("gitPrPullRequest"))
            });
            memory.trace.clear();
        }
        world.run(&["--no-stack"]).unwrap();
        assert_eq!(world.0.lock().unwrap().prs, before);
        assert!(
            !world
                .trace()
                .iter()
                .any(|e| e["method"] == "POST" && e["path"] == "pulls")
        );
        assert!(config_writes(&world) > 0);
    }
}

#[test]
fn intact_names_recover_after_rebase_without_hash_equality() {
    let world = clean_stack(2);
    world.run(&["--no-stack"]).unwrap();
    {
        let mut memory = world.0.lock().unwrap();
        memory.config.retain(|key, _| !key.starts_with("branch."));
        for (i, commit) in memory.commits.iter_mut().enumerate() {
            commit.hash = format!("a{i:039x}");
        }
        memory.trace.clear();
    }
    let rewritten = world.0.lock().unwrap().commits.clone();
    world.run(&["--no-stack"]).unwrap();
    assert_eq!(world.0.lock().unwrap().commits, rewritten);
    assert_eq!(world.0.lock().unwrap().prs.len(), 2);
    assert!(
        !world
            .trace()
            .iter()
            .any(|e| e["method"] == "POST" && e["path"] == "pulls")
    );
}

#[test]
fn legacy_trailers_and_new_descendants_keep_their_original_messages() {
    let world = World::new(3);
    world.0.lock().unwrap().commits[2].attrs.clear();
    let before = world.0.lock().unwrap().commits.clone();
    world.run(&["--no-stack"]).unwrap();
    assert_eq!(world.0.lock().unwrap().commits, before);
    assert_eq!(world.0.lock().unwrap().commits[2].attr(REMOTE_REF), "");
}

#[test]
fn checkpoints_neither_persist_nor_request_remote_evidence() {
    for mode in ["--dry-run", "--stop-after=prepare", "--stop-after=rewrite"] {
        let world = clean_stack(2);
        world.run(&[mode]).unwrap();
        assert_eq!(config_writes(&world), 0);
        assert!(
            !world.trace().iter().any(|e| e["kind"] == "http"
                || e["args"][0] == "push"
                || e["args"][0] == "ls-remote")
        );
    }
}

#[test]
fn claims_constrain_identity_while_irrelevant_hints_do_not_override_it() {
    let world = World::new(1);
    world.run(&["--no-stack"]).unwrap();
    {
        let mut memory = world.0.lock().unwrap();
        memory
            .config
            .insert("branch.topic/1.merge".into(), "refs/heads/main".into());
        memory.config.insert(
            "branch.topic/1.github-pr-owner-number".into(),
            "malformed ignored hint".into(),
        );
    }
    world.run(&["--no-stack"]).unwrap();
    world.0.lock().unwrap().config.insert(
        "branch.topic/1.gitPrHead".into(),
        "github.com/org/repo refs/heads/other".into(),
    );
    world.0.lock().unwrap().trace.clear();
    assert!(format!("{:#}", world.run(&["--no-stack"]).unwrap_err()).contains("conflicts"));
    assert_eq!(config_writes(&world), 0);
    assert!(!world.trace().iter().any(|e| e["args"][0] == "push"));
}

#[test]
fn adopted_review_cannot_be_replaced_by_a_new_review_on_the_same_head() {
    let world = clean_stack(1);
    world.run(&["--no-stack"]).unwrap();
    {
        let mut memory = world.0.lock().unwrap();
        let mut other = memory.prs[&101].clone();
        other["number"] = json!(102);
        other["node_id"] = json!("PR102");
        memory.prs.get_mut(&101).unwrap()["state"] = json!("closed");
        memory.prs.insert(102, other);
        memory.trace.clear();
    }
    assert!(world.run(&["--no-stack"]).is_err());
    assert_eq!(config_writes(&world), 0);
    assert!(!world.trace().iter().any(|e| e["args"][0] == "push"));
}

#[test]
fn unknown_discovery_and_remote_collision_never_authorise_publication() {
    for unavailable in [true, false] {
        let world = clean_stack(1);
        if unavailable {
            world.http(
                &format!("GET commits/{}/pulls", commit(1).hash),
                vec![crate::tests::unit::support::Response::Failure(
                    "offline".into(),
                )],
            );
        } else {
            world
                .0
                .lock()
                .unwrap()
                .remote_refs
                .insert("refs/heads/topic/1".into(), "f".repeat(40));
            world.reply(
                &format!(
                    "git merge-base --is-ancestor {} {}",
                    "f".repeat(40),
                    commit(1).hash
                ),
                1,
                "",
            );
        }
        assert!(world.run(&["--no-stack"]).is_err());
        assert_eq!(config_writes(&world), 0);
        assert!(!world.trace().iter().any(|e| e["args"][0] == "push"));
    }
}

#[test]
fn pending_creation_is_durable_and_a_restart_does_not_repeat_it() {
    let world = clean_stack(1);
    world.http(
        "POST pulls",
        vec![crate::tests::unit::support::Response::Failure(
            "lost acknowledgement".into(),
        )],
    );
    assert!(world.run(&["--no-stack"]).is_err());
    assert!(
        world
            .0
            .lock()
            .unwrap()
            .config
            .contains_key("git-pr.pendingCreate")
    );
    world.0.lock().unwrap().trace.clear();
    assert!(format!("{:#}", world.run(&["--no-stack"]).unwrap_err()).contains("unknown outcome"));
    assert!(
        !world
            .trace()
            .iter()
            .any(|e| e["method"] == "POST" && e["path"] == "pulls")
    );
    assert!(!world.trace().iter().any(|e| e["args"][0] == "push"));
}

#[test]
fn completed_creation_is_adopted_before_clearing_its_attempt() {
    let world = clean_stack(1);
    world.run(&["--no-stack"]).unwrap();
    let config = world.config();
    let attempt = Attempt {
        id: "00000000-0000-4000-8000-000000000001".into(),
        head: Head::new(&config.repo, "topic/1").unwrap(),
        base: "main".into(),
    };
    {
        let mut memory = world.0.lock().unwrap();
        memory.config.remove("branch.topic/1.gitPrPullRequest");
        memory
            .config
            .insert("git-pr.pendingCreate".into(), attempt.value());
        memory.trace.clear();
    }
    world.run(&["--no-stack"]).unwrap();
    let memory = world.0.lock().unwrap();
    assert!(!memory.config.contains_key("git-pr.pendingCreate"));
    assert!(
        memory
            .config
            .contains_key("branch.topic/1.gitPrPullRequest")
    );
    assert_eq!(memory.prs.len(), 1);
}

#[test]
fn aliases_and_copied_bindings_do_not_depend_on_inventory_order() {
    for reverse in [false, true] {
        let world = clean_stack(1);
        let mut branches = [
            format!("a|{}", commit(1).hash),
            format!("b|{}", commit(1).hash),
        ];
        if reverse {
            branches.reverse();
        }
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            &branches.join("\n"),
        );
        assert!(
            format!("{:#}", world.run(&["--no-stack"]).unwrap_err())
                .contains("ambiguous local anchors")
        );
        assert_eq!(config_writes(&world), 0);
    }
}

#[test]
fn metadata_schema_round_trips_literal_refs_and_rejects_malformed_claims() {
    let world = World::new(0);
    let repo = &world.config().repo;
    for reference in ["dyt/pipe|café#x", "topic/one", "CaseSensitive"] {
        let head = Head::new(repo, reference).unwrap();
        assert_eq!(Head::parse(&head.value(), repo).unwrap(), head);
        assert_eq!(
            parse_review(&super::review_url(repo, 123), repo).unwrap(),
            123
        );
        let attempt = Attempt {
            id: "12345678-abcd-4123-8abc-123456789abc".into(),
            head,
            base: "main".into(),
        };
        assert_eq!(Attempt::parse(&attempt.value(), repo).unwrap(), attempt);
    }
    for bad in [
        "",
        "github.com/other/repo refs/heads/a",
        "github.com/org/repo a",
        "github.com/org/repo  refs/heads/a",
        "github.com/org/repo refs/heads/a ",
    ] {
        assert!(Head::parse(bad, repo).is_err());
    }
    for bad in [
        "",
        "https://elsewhere/org/repo/pull/1",
        "https://github.com/org/repo/pull/0",
        "https://github.com/org/repo/pull/01",
        "https://github.com/org/repo/pull/no",
    ] {
        assert!(parse_review(bad, repo).is_err());
    }
    for bad in [
        "v2",
        "v1 bad github.com/org/repo refs/heads/a github.com/org/repo refs/heads/main",
        "v1 12345678-abcd-4123-8abc-123456789abc github.com/org/repo refs/heads/a other/repo refs/heads/main",
        "v1 12345678-abcd-4123-8abc-123456789abc github.com/org/repo refs/heads/a github.com/org/repo main",
    ] {
        assert!(Attempt::parse(bad, repo).is_err());
    }
    assert_eq!(repository(repo), "github.com/org/repo");
}

#[test]
fn configuration_provenance_preserves_conflicts_and_scope() {
    for scope in ["global", "worktree", "command"] {
        let records =
            Records::decode(&format!("{scope}\0file:config\0branch.a.gitprhead\nx\0")).unwrap();
        assert!(records.claim("a", "gitprhead").is_err());
    }
    for duplicate in ["x", "y"] {
        let records = Records::decode(&format!("local\0file:first\0branch.a.gitprhead\nx\0local\0file:include\0branch.a.gitprhead\n{duplicate}\0")).unwrap();
        assert_eq!(records.claim("a", "gitprhead").is_ok(), duplicate == "x");
    }
    assert!(Records::decode("truncated").is_err());
    assert!(Records::decode("local\0").is_err());
    assert_eq!(
        Records::default().claim("absent", "gitprhead").unwrap(),
        None
    );
    let records = Records::decode("local\0file:config\0branch.a.gitprhead\0").unwrap();
    assert_eq!(records.claim("a", "gitprhead").unwrap(), Some(""));
}

#[test]
fn configuration_repair_never_overwrites_a_different_binding() {
    let world = World::new(0);
    store::insert(
        &world.commands(),
        "branch.a.gitPrHead",
        "github.com/org/repo refs/heads/a",
    )
    .unwrap();
    let writes = config_writes(&world);
    store::insert(
        &world.commands(),
        "branch.a.gitPrHead",
        "github.com/org/repo refs/heads/a",
    )
    .unwrap();
    assert_eq!(writes, config_writes(&world));
    assert!(
        store::insert(
            &world.commands(),
            "branch.a.gitPrHead",
            "github.com/org/repo refs/heads/b"
        )
        .is_err()
    );
}

#[test]
fn lost_anchors_require_explicit_identity_but_legacy_stacks_remain_valid() {
    for legacy in [true, false] {
        let world = if legacy {
            World::new(1)
        } else {
            clean_stack(1)
        };
        let config = world.config();
        let commands = world.commands();
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            "",
        );
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: world.as_ref(),
            template: "",
        };
        let mut commits = world.0.lock().unwrap().commits.clone();
        assert_eq!(resolve::prepare(&github, &mut commits, 0).is_ok(), legacy);
        if legacy {
            assert!(commits[0].publication.as_ref().unwrap().anchor.is_none());
        }
    }
}

#[test]
fn pure_uniqueness_validation_rejects_a_copied_binding() {
    let world = World::new(1);
    let repo = &world.config().repo;
    let mut selected = commit(1);
    selected.publication = Some(super::Binding {
        anchor: Some("a".into()),
        anchor_oid: Some(commit(1).hash),
        head: Head::new(repo, "topic/1").unwrap(),
        review: None,
        adopted: true,
        previous_anchor: None,
    });
    let records = Records::decode(
        "local\0file:config\0branch.b.gitprhead\ngithub.com/org/repo refs/heads/topic/1\0",
    )
    .unwrap();
    assert!(
        resolve::validate_unique(
            &[selected],
            0,
            &records,
            &BTreeMap::from([("a".into(), commit(1).hash), ("b".into(), commit(1).hash)]),
        )
        .is_err()
    );
}

#[test]
fn jujutsu_renames_relocate_one_association_without_changing_the_publication() {
    let world = clean_stack(1);
    world.0.lock().unwrap().jj = true;
    world.run(&["--no-stack"]).unwrap();
    for name in ["renamed", "renamed-again"] {
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            &format!("{name}|{}", commit(1).hash),
        );
        world.run(&["--no-stack"]).unwrap();
        let memory = world.0.lock().unwrap();
        let keys: Vec<_> = memory
            .config
            .keys()
            .filter(|key| key.to_ascii_lowercase().ends_with("gitprhead"))
            .collect();
        assert_eq!(keys, [&format!("branch.{name}.gitPrHead")]);
        assert_eq!(
            memory.config[&format!("branch.{name}.gitPrHead")],
            "github.com/org/repo refs/heads/topic/1"
        );
        assert_eq!(memory.prs.len(), 1);
        assert_eq!(memory.remote_refs["refs/heads/topic/1"], commit(1).hash);
    }
}

#[test]
fn discovery_hints_must_identify_one_review_and_one_destination() {
    for source in [
        "editor",
        "push",
        "upstream",
        "exact",
        "ambiguous",
        "foreign-editor",
        "malformed-editor",
        "truncated-push",
    ] {
        let world = clean_stack(1);
        world.run(&["--no-stack"]).unwrap();
        {
            let mut memory = world.0.lock().unwrap();
            memory.config.retain(|key, _| !key.starts_with("branch."));
            memory.prs.get_mut(&101).unwrap()["head"]["sha"] = json!(commit(1).hash);
            memory.trace.clear();
        }
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            &format!("renamed|{}", commit(1).hash),
        );
        match source {
            "editor" | "foreign-editor" | "malformed-editor" => {
                world.0.lock().unwrap().config.insert(
                    "branch.renamed.github-pr-owner-number".into(),
                    match source {
                        "editor" => "org#repo#101",
                        "foreign-editor" => "other#repo#101",
                        _ => "invalid",
                    }
                    .into(),
                );
            }
            "push" | "ambiguous" | "truncated-push" => {
                world.reply("git for-each-ref --format=%(push:remotename)%00%(push:remoteref) refs/heads/renamed", 0, match source { "push" => "origin\0refs/heads/topic/1", "ambiguous" => "origin\0refs/heads/other", _ => "origin" });
            }
            "upstream" => {
                let mut memory = world.0.lock().unwrap();
                memory
                    .config
                    .insert("branch.renamed.remote".into(), "origin".into());
                memory
                    .config
                    .insert("branch.renamed.merge".into(), "refs/heads/topic/1".into());
            }
            _ => {}
        }
        let result = world.run(&["--no-stack"]);
        if matches!(
            source,
            "ambiguous" | "foreign-editor" | "malformed-editor" | "truncated-push"
        ) {
            assert!(result.is_err(), "{source}");
            assert_eq!(config_writes(&world), 0);
            assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
        } else {
            result.unwrap_or_else(|error| panic!("{source}: {error:#}"));
            assert_eq!(
                world.0.lock().unwrap().config["branch.renamed.gitPrHead"],
                "github.com/org/repo refs/heads/topic/1"
            );
            assert_eq!(world.0.lock().unwrap().prs.len(), 1);
        }
    }
}

#[test]
fn migration_requires_unique_stable_identity_and_a_writable_origin() {
    for failure in [
        "live",
        "multiple",
        "different-change",
        "invalid-change",
        "included",
        "primary-conflict",
        "divergent",
    ] {
        let world = clean_stack(if failure == "divergent" { 2 } else { 1 });
        world.0.lock().unwrap().jj = true;
        world.run(&["--no-stack"]).unwrap();
        let hash = commit(1).hash;
        let mut inventory = format!("renamed|{hash}");
        {
            let mut memory = world.0.lock().unwrap();
            match failure {
                "live" => inventory = format!("{inventory}\ntopic/1|{}", commit(2).hash),
                "multiple" => {
                    memory
                        .config
                        .insert("branch.another.gitPrChangeId".into(), commit(1).change_id);
                }
                "different-change" | "invalid-change" => {
                    inventory = format!("topic/1|{hash}");
                    memory.config.insert(
                        "branch.topic/1.gitPrChangeId".into(),
                        if failure == "different-change" {
                            "another"
                        } else {
                            "invalid id"
                        }
                        .into(),
                    );
                }
                "primary-conflict" => {
                    memory.config.insert(
                        "branch.renamed.gitPrHead".into(),
                        "github.com/org/repo refs/heads/other".into(),
                    );
                }
                "divergent" => {
                    memory.commits[1].change_id = commit(1).change_id;
                }
                _ => {}
            }
            memory.trace.clear();
        }
        if failure != "divergent" {
            world.reply(
                "git branch --format=%(refname:strip=2)|%(objectname)",
                0,
                &inventory,
            );
        }
        if failure == "included" {
            let commands = world.commands();
            let mut records = Records::read(&commands).unwrap();
            for record in records.0.values_mut().flatten() {
                record.origin = "file:/outside/include".into();
            }
            assert!(
                records
                    .relocation(&commands, "topic/1")
                    .unwrap_err()
                    .to_string()
                    .contains("origin")
            );
            continue;
        }
        assert!(world.run(&["--no-stack"]).is_err(), "{failure}");
        assert_eq!(config_writes(&world), 0, "{failure}");
        assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
    }
}

#[test]
fn stale_anchor_and_changed_bindings_stop_before_effects() {
    let world = clean_stack(1);
    world.run(&["--no-stack"]).unwrap();
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
    let mut commits = world.0.lock().unwrap().commits.clone();
    resolve::prepare(&github, &mut commits, 0).unwrap();
    world.reply(
        "git branch --format=%(refname:strip=2)|%(objectname)",
        0,
        &format!("topic/1|{}", commit(2).hash),
    );
    assert!(resolve::verify_anchors(&commands, &commits).is_err());
    for field in ["gitPrPullRequest", "gitPrHead"] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .remove(&format!("branch.topic/1.{field}"));
        assert!(resolve::verify_bindings(&commands, &config.repo, &commits).is_err());
    }
}

#[test]
fn repository_lock_refuses_a_second_submission_before_persistence() {
    let world = clean_stack(1);
    world.0.lock().unwrap().file_errors.insert(
        "/unit/repo/.git/git-pr.lock".into(),
        std::io::ErrorKind::WouldBlock,
    );
    assert!(
        format!("{:#}", world.run(&["--no-stack"]).unwrap_err()).contains("another submission")
    );
    assert_eq!(config_writes(&world), 0);
    assert!(
        !world
            .trace()
            .iter()
            .any(|event| event["kind"] == "http" || event["args"][0] == "push")
    );
}

#[test]
fn equivalent_repository_names_preserve_existing_records_and_literal_ref_case() {
    let world = clean_stack(1);
    world.run(&["--no-stack"]).unwrap();
    {
        let mut memory = world.0.lock().unwrap();
        memory.config.insert(
            "branch.topic/1.gitPrHead".into(),
            "GitHub.com/Org/Repo refs/heads/topic/1".into(),
        );
        memory.config.insert(
            "branch.topic/1.gitPrPullRequest".into(),
            "https://GitHub.com/Org/Repo/pull/101".into(),
        );
        memory.trace.clear();
    }
    world.run(&["--no-stack"]).unwrap();
    assert_eq!(config_writes(&world), 0);
    assert!(
        store::insert(
            &world.commands(),
            "branch.topic/1.gitPrHead",
            "github.com/org/repo refs/heads/Topic/1"
        )
        .is_err()
    );
    let records = Records::decode("local\0file:a\0branch.a.gitprhead\nGitHub.com/Org/Repo refs/heads/a\0local\0file:b\0branch.a.gitprhead\ngithub.com/org/repo refs/heads/a\0").unwrap();
    assert!(records.head("a", &world.config().repo).unwrap().is_some());
}

#[test]
fn configuration_failures_are_not_missing_evidence_or_confirmed_writes() {
    let query = format!(
        "git config --null --show-scope --show-origin --includes --get-regexp {}",
        store::PATTERN
    );
    for (status, text) in [(2, "cannot parse configuration"), (1, "unexpected output")] {
        let world = World::new(0);
        world.reply(&query, status, text);
        assert!(Records::read(&world.commands()).is_err());
    }
    let world = World::new(0);
    world.reply(
        "git config --local --add branch.a.gitPrHead github.com/org/repo refs/heads/a",
        0,
        "",
    );
    assert!(
        store::insert(
            &world.commands(),
            "branch.a.gitPrHead",
            "github.com/org/repo refs/heads/a"
        )
        .unwrap_err()
        .to_string()
        .contains("could not be verified")
    );
    let records = Records::decode(
        "local\0file:config\0branch.a.gitprhead\ngithub.com/org/repo refs/heads/a\0",
    )
    .unwrap();
    assert!(
        store::persist_observed(
            &world.commands(),
            &records,
            &[(
                "branch.a.gitPrHead".into(),
                "github.com/org/repo refs/heads/b".into()
            )]
        )
        .is_err()
    );
    for origin in ["command line", "file:/shared/config"] {
        let records =
            Records::decode(&format!("local\0{origin}\0branch.a.gitprhead\nx\0")).unwrap();
        assert!(records.relocation(&world.commands(), "a").is_err());
    }
}

#[test]
fn creation_records_require_local_scope_and_verified_persistence() {
    let world = World::new(0);
    let commands = world.commands();
    let config = world.config();
    let attempt = Attempt {
        id: "00000000-0000-4000-8000-000000000001".into(),
        head: Head::new(&config.repo, "a").unwrap(),
        base: "main".into(),
    };
    let inherited = Records::decode(&format!(
        "global\0file:global\0git-pr.pendingcreate\n{}\0",
        attempt.value()
    ))
    .unwrap();
    assert!(inherited.attempts(&config.repo).is_err());
    world.reply(
        &format!(
            "git config --local --add git-pr.pendingCreate {}",
            attempt.value()
        ),
        0,
        "",
    );
    assert!(
        store::begin(&commands, &config.repo, &attempt)
            .unwrap_err()
            .to_string()
            .contains("could not be verified")
    );
    world.0.lock().unwrap().responses.clear();
    store::begin(&commands, &config.repo, &attempt).unwrap();
    assert!(
        store::begin(&commands, &config.repo, &attempt)
            .unwrap_err()
            .to_string()
            .contains("unresolved attempt")
    );
    let other = Attempt {
        head: Head::new(&config.repo, "b").unwrap(),
        ..attempt.clone()
    };
    assert_ne!(other.value(), attempt.value());
    store::finish(&commands, &config.repo, "b").unwrap();
    assert_eq!(
        Records::read(&commands)
            .unwrap()
            .attempts(&config.repo)
            .unwrap(),
        std::slice::from_ref(&attempt)
    );
    store::finish(&commands, &config.repo, "a").unwrap();
    for id in [
        "00000000x0000-4000-8000-000000000001",
        "z0000000-0000-4000-8000-000000000001",
    ] {
        assert!(
            Attempt::parse(
                &Attempt {
                    id: id.into(),
                    ..attempt.clone()
                }
                .value(),
                &config.repo
            )
            .is_err()
        );
    }
}

#[test]
fn interrupted_relocation_never_removes_a_concurrent_different_binding() {
    let world = World::new(0);
    let commands = world.commands();
    let key = "branch.old.gitprhead";
    let value = "github.com/org/repo refs/heads/a";
    let values = vec![(key.to_owned(), value.to_owned())];
    store::retire(&commands, &values).unwrap();
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert(key.into(), "another association".into());
    assert!(
        store::retire(&commands, &values)
            .unwrap_err()
            .to_string()
            .contains("changed during relocation")
    );
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert(key.into(), value.into());
    world.reply(
        &format!("git config --local --fixed-value --unset-all {key} {value}"),
        0,
        "",
    );
    assert!(
        store::retire(&commands, &values)
            .unwrap_err()
            .to_string()
            .contains("could not be retired")
    );
    world.0.lock().unwrap().responses.clear();
    store::retire(&commands, &values).unwrap();
    store::retire(&commands, &values).unwrap();
}

#[test]
fn inventory_validates_observations_and_legacy_aliases_without_choosing_by_order() {
    for invalid in ["incomplete", "bad..branch|oid", "a|one\na|two"] {
        let world = World::new(0);
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            invalid,
        );
        assert!(resolve::inventory(&world.commands()).is_err());
    }
    for (names, binding, expected) in [
        (vec!["other", "topic/1"], None, Some("topic/1")),
        (vec!["a", "b"], None, None),
        (vec!["other", "topic/1"], Some("other"), Some("other")),
    ] {
        let world = World::new(1);
        let config = world.config();
        let commands = world.commands();
        let inventory = names
            .iter()
            .map(|name| format!("{name}|{}", commit(1).hash))
            .collect::<Vec<_>>()
            .join("\n");
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            &inventory,
        );
        if let Some(name) = binding {
            world.0.lock().unwrap().config.insert(
                format!("branch.{name}.gitPrHead"),
                "github.com/org/repo refs/heads/topic/1".into(),
            );
        }
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: world.as_ref(),
            template: "",
        };
        let mut commits = vec![commit(1)];
        resolve::prepare(&github, &mut commits, 0).unwrap();
        assert_eq!(
            commits[0].publication.as_ref().unwrap().anchor.as_deref(),
            expected
        );
    }
}

#[test]
fn missing_context_and_preview_review_only_binding_do_not_invent_identity() {
    let world = clean_stack(1);
    world.0.lock().unwrap().config.insert(
        "branch.topic/1.gitPrPullRequest".into(),
        "https://github.com/org/repo/pull/101".into(),
    );
    assert!(
        format!("{:#}", world.run(&["--dry-run"]).unwrap_err()).contains("remote verification")
    );
    world
        .0
        .lock()
        .unwrap()
        .config
        .remove("branch.topic/1.gitPrPullRequest");
    world.reply(
        "git branch --format=%(refname:strip=2)|%(objectname)",
        0,
        "main|trunk\n(no branch)|detached",
    );
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
    let mut commits = world.0.lock().unwrap().commits.clone();
    resolve::prepare(&github, &mut commits, 1).unwrap();
    assert!(commits[0].publication.is_none());
}

#[test]
fn excessive_remote_history_remains_unknown_instead_of_becoming_absence() {
    use crate::tests::unit::support::Response;
    let world = clean_stack(1);
    let config = world.config();
    let commands = world.commands();
    let pages = (0..100).map(|page| {
        let nodes: Vec<_> = (1..=100).map(|offset| json!({"number":page * 100 + offset,"state":"open","head":{"ref":"topic/1","sha":commit(1).hash,"repo":{"full_name":"org/repo"}}})).collect();
        Response::Output(200, serde_json::to_string(&nodes).unwrap())
    }).collect();
    world.http(&format!("GET commits/{}/pulls", commit(1).hash), pages);
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "",
    };
    assert!(
        github
            .identity_heads(&commit(1).hash)
            .unwrap_err()
            .to_string()
            .contains("page limit")
    );
    assert_eq!(config_writes(&world), 0);
}

#[test]
fn pinned_review_and_live_head_claims_must_agree_before_adoption() {
    for conflict in ["review-head", "missing-review", "former-review"] {
        let world = clean_stack(1);
        world.run(&["--no-stack"]).unwrap();
        {
            let mut memory = world.0.lock().unwrap();
            match conflict {
                "review-head" => memory.prs.get_mut(&101).unwrap()["head"]["ref"] = json!("other"),
                "missing-review" => {
                    memory.prs.clear();
                }
                _ => {
                    memory.jj = true;
                    memory
                        .config
                        .insert("branch.former.gitPrChangeId".into(), commit(1).change_id);
                    memory.config.insert(
                        "branch.former.gitPrPullRequest".into(),
                        "https://github.com/org/repo/pull/102".into(),
                    );
                }
            }
            memory.trace.clear();
        }
        assert!(world.run(&["--no-stack"]).is_err(), "{conflict}");
        assert_eq!(config_writes(&world), 0);
        assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
    }
}

#[test]
fn pure_identity_validation_rejects_incomplete_and_duplicate_plans() {
    let world = clean_stack(2);
    let branches = BTreeMap::new();
    let records = Records::default();
    let commits = world.0.lock().unwrap().commits.clone();
    assert!(resolve::validate_unique(&commits, 0, &records, &branches).is_err());
    assert!(resolve::validate_unique(&[commit(1), commit(1)], 0, &records, &branches).is_err());
}

#[test]
fn jujutsu_single_revision_and_invalid_creation_content_are_verified() {
    let world = World::new(1);
    world.0.lock().unwrap().jj = true;
    world.run(&["--no-stack", "topic/1"]).unwrap();
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "<!-- git-pr-stack-start -->",
    };
    let mut c = commit(2);
    c.message.clear();
    assert!(
        format!("{:#}", github.create(&mut c, "main").unwrap_err())
            .contains("failed to prepare PR creation")
    );
}

#[derive(Debug)]
struct InterruptedObservation {
    world: std::sync::Arc<World>,
    remaining: std::sync::atomic::AtomicUsize,
}
impl InterruptedObservation {
    fn observe(&self) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;
        if self
            .remaining
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .unwrap_or(0)
            <= 1
        {
            anyhow::bail!("injected unavailable observation");
        }
        Ok(())
    }
}
impl crate::commands::CommandRunner for InterruptedObservation {
    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    ) -> anyhow::Result<crate::commands::CommandOutput> {
        self.observe()?;
        self.world.execute(program, args, cwd, env)
    }
}
impl crate::github::http::Transport for InterruptedObservation {
    fn request(&self, request: crate::github::http::Request<'_>) -> anyhow::Result<Vec<u8>> {
        self.observe()?;
        self.world.request(request)
    }
}

fn interruption_fixture(scenario: &str) -> std::sync::Arc<World> {
    let world = clean_stack(1);
    if scenario != "new" {
        world.run(&["--no-stack"]).unwrap();
        let mut memory = world.0.lock().unwrap();
        if scenario == "renamed" {
            memory.jj = true;
            memory
                .config
                .insert("branch.topic/1.gitPrChangeId".into(), commit(1).change_id);
        } else {
            let repo = crate::model::Repository {
                host: "github.com".into(),
                name: "org/repo".into(),
                ..crate::model::Repository::default()
            };
            let attempt = Attempt {
                id: "00000000-0000-4000-8000-000000000001".into(),
                head: Head::new(&repo, "topic/1").unwrap(),
                base: "main".into(),
            };
            memory
                .config
                .insert("git-pr.pendingCreate".into(), attempt.value());
        }
        memory.trace.clear();
    }
    if scenario == "renamed" {
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            &format!("renamed|{}", commit(1).hash),
        );
    }
    world
}

#[test]
fn unavailable_observations_at_every_boundary_preserve_identity_and_resume_safely() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for scenario in ["new", "renamed", "pending"] {
        let baseline = interruption_fixture(scenario);
        let observer = Arc::new(InterruptedObservation {
            world: baseline.clone(),
            remaining: AtomicUsize::new(1000),
        });
        let flags = crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap();
        let env = BTreeMap::from([("HOME".into(), "/unit/home".into())]);
        let mut app = baseline.app();
        app.commands.runner = observer.clone();
        app.transport = observer.clone();
        app.execute(&flags, &env).unwrap();
        let boundaries = 1000 - observer.remaining.load(Ordering::SeqCst);
        for cut in 1..=boundaries {
            let world = interruption_fixture(scenario);
            let before = world.0.lock().unwrap().commits.clone();
            let observer = Arc::new(InterruptedObservation {
                world: world.clone(),
                remaining: AtomicUsize::new(cut),
            });
            let mut app = world.app();
            app.commands.runner = observer.clone();
            app.transport = observer;
            let _ = app.execute(&flags, &env);
            assert_eq!(world.0.lock().unwrap().commits, before, "{scenario}/{cut}");
            // This prefix oracle proves that an absent creation was never dispatched.
            if world.0.lock().unwrap().prs.is_empty() {
                world
                    .0
                    .lock()
                    .unwrap()
                    .config
                    .remove("git-pr.pendingCreate");
            }
            world
                .run(&["--no-stack"])
                .unwrap_or_else(|error| panic!("{scenario}/{cut}: {error:#}"));
            assert_eq!(world.0.lock().unwrap().prs.len(), 1, "{scenario}/{cut}");
        }
    }
}

fn scoped_query(key: &str) -> String {
    format!(
        "git config --null --show-scope --show-origin --includes --get-regexp ^{}$",
        regex::escape(key)
    )
}

#[test]
fn inherited_or_racing_configuration_never_becomes_a_local_binding() {
    use crate::tests::unit::support::Response;
    for after in [false, true] {
        let world = World::new(0);
        let value = "github.com/org/repo refs/heads/a";
        let key = "branch.a.gitprhead";
        let inherited = format!("global\0file:global\0{key}\n{value}\0");
        let mut replies = std::collections::VecDeque::new();
        if after {
            replies.push_back(Response::Output(1, String::new()));
        }
        replies.push_back(Response::Output(0, inherited));
        world
            .0
            .lock()
            .unwrap()
            .responses
            .insert(scoped_query(key), replies);
        assert!(store::insert(&world.commands(), key, value).is_err());
        assert_eq!(config_writes(&world), usize::from(after));
    }
    let world = World::new(0);
    let commands = world.commands();
    let config = world.config();
    let head = Head::new(&config.repo, "a").unwrap();
    let attempt = Attempt {
        id: "00000000-0000-4000-8000-000000000001".into(),
        head: head.clone(),
        base: "main".into(),
    };
    let inherited = format!(
        "global\0file:global\0git-pr.pendingcreate\n{}\0",
        attempt.value()
    );
    world.reply(&scoped_query("git-pr.pendingcreate"), 0, &inherited);
    assert!(store::begin(&commands, &config.repo, &attempt).is_err());
    assert!(store::finish(&commands, &config.repo, "a").is_err());
    world.0.lock().unwrap().responses.insert(
        scoped_query("git-pr.pendingcreate"),
        [
            Response::Output(1, String::new()),
            Response::Output(0, inherited),
        ]
        .into(),
    );
    assert!(store::begin(&commands, &config.repo, &attempt).is_err());
    let records = Records::decode("global\0file:global\0branch.a.gitprhead\ninvalid\0global\0file:global\0branch.a.gitprpullrequest\ninvalid\0").unwrap();
    assert!(records.head("a", &config.repo).is_err());
    assert!(records.review("a", &config.repo).is_err());
    assert!(records.relocation(&commands, "a").is_err());
    assert!(
        store::persist_observed(
            &commands,
            &records,
            &[("branch.a.gitPrHead".into(), head.value())]
        )
        .is_err()
    );
    assert!(!head.matches("invalid"));
    assert!(!head.matches("github.com/other/repo refs/heads/a"));
    assert!(!head.matches("github.com/org/repo refs/heads/A"));
}

#[test]
fn malformed_pending_records_prevent_recovery_and_invalid_git_keys_remain_git_errors() {
    let world = World::new(0);
    let repo = &world.config().repo;
    for value in [
        "v1 00000000-0000-4000-8000-000000000001 github.com/other/repo refs/heads/a github.com/org/repo refs/heads/main",
        "v1 00000000-0000-4000-8000-000000000001 github.com/org/repo refs/heads/a github.com/org/repo refs/heads/bad..base",
    ] {
        assert!(Attempt::parse(value, repo).is_err());
    }
    let records = Records::decode("local\0file:config\0git-pr.pendingcreate\ninvalid\0local\0file:config\0branch.a\nignored\0").unwrap();
    assert_eq!(records.anchors().count(), 0);
    assert!(records.attempts(repo).is_err());
    let world = clean_stack(1);
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert("git-pr.pendingCreate".into(), "invalid".into());
    assert!(world.run(&["--no-stack"]).is_err());
    assert_eq!(config_writes(&world), 0);
    let world = World::new(0);
    world.fail("git config --local --add invalid value");
    assert!(store::insert(&world.commands(), "invalid", "value").is_err());
}

#[derive(Debug)]
struct InvalidConfigEncoding;
impl crate::commands::CommandRunner for InvalidConfigEncoding {
    fn execute(
        &self,
        _: &str,
        _: &[String],
        _: &std::path::Path,
        _: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    ) -> anyhow::Result<crate::commands::CommandOutput> {
        anyhow::bail!("unexpected text request")
    }
    fn execute_data(
        &self,
        _: &str,
        _: &[String],
        _: &std::path::Path,
        _: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        _: &[u8],
    ) -> anyhow::Result<crate::commands::StructuredOutput> {
        Ok(crate::commands::StructuredOutput {
            status: 0,
            stdout: vec![0xff],
            stderr: Vec::new(),
        })
    }
}

#[test]
fn undecodable_configuration_is_an_error_without_partial_records() {
    let world = World::new(0);
    let mut commands = world.commands();
    commands.runner = std::sync::Arc::new(InvalidConfigEncoding);
    assert!(
        Records::read(&commands)
            .unwrap_err()
            .to_string()
            .contains("not UTF-8")
    );
}

#[test]
fn relocation_refuses_inherited_records_before_and_after_its_literal_removal() {
    use crate::tests::unit::support::Response;
    for after in [false, true] {
        let world = World::new(0);
        let key = "branch.old.gitprhead";
        let value = "github.com/org/repo refs/heads/a";
        let global = format!("global\0file:global\0{key}\n{value}\0");
        let local = format!("local\0file:local\0{key}\n{value}\0");
        let mut replies = std::collections::VecDeque::new();
        if after {
            replies.push_back(Response::Output(0, local));
        }
        replies.push_back(Response::Output(0, global));
        world
            .0
            .lock()
            .unwrap()
            .responses
            .insert(scoped_query(key), replies);
        assert!(store::retire(&world.commands(), &[(key.into(), value.into())]).is_err());
    }
}

#[test]
fn pending_renamed_heads_require_a_fresh_open_review_observation() {
    for unavailable in [false, true] {
        let world = clean_stack(1);
        world.0.lock().unwrap().jj = true;
        world.run(&["--no-stack"]).unwrap();
        let repo = world.config().repo.clone();
        {
            let mut memory = world.0.lock().unwrap();
            memory.config.remove("branch.topic/1.gitPrPullRequest");
            memory.config.insert(
                "git-pr.pendingCreate".into(),
                Attempt {
                    id: "00000000-0000-4000-8000-000000000001".into(),
                    head: Head::new(&repo, "topic/1").unwrap(),
                    base: "main".into(),
                }
                .value(),
            );
            if !unavailable {
                memory.prs.get_mut(&101).unwrap()["state"] = json!("closed");
            }
            memory.trace.clear();
        }
        if unavailable {
            world.http(
                "POST graphql",
                vec![crate::tests::unit::support::Response::Failure(
                    "unavailable".into(),
                )],
            );
        }
        world.reply(
            "git branch --format=%(refname:strip=2)|%(objectname)",
            0,
            &format!("renamed|{}", commit(1).hash),
        );
        assert!(world.run(&["--no-stack"]).is_err());
        assert_eq!(config_writes(&world), 0);
    }
}

#[test]
fn corrupt_records_remain_errors_at_each_mutation_barrier() {
    let world = clean_stack(1);
    world.run(&["--no-stack"]).unwrap();
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
    let mut commits = world.0.lock().unwrap().commits.clone();
    resolve::prepare(&github, &mut commits, 0).unwrap();
    for field in ["gitPrPullRequest", "gitPrHead"] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert(format!("branch.topic/1.{field}"), "invalid".into());
        assert!(resolve::verify_bindings(&commands, &config.repo, &commits).is_err());
    }
    assert!(world.run(&["--no-stack"]).is_err());
    world
        .0
        .lock()
        .unwrap()
        .config
        .insert("git-pr.pendingCreate".into(), "invalid".into());
    assert!(resolve::persist(&github, &mut commits).is_err());
}

#[test]
fn persistence_requires_a_resolved_binding_before_any_repair() {
    let world = World::new(1);
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
    assert!(
        resolve::persist(&github, &mut [commit(1)])
            .unwrap_err()
            .to_string()
            .contains("no resolved publication binding")
    );
    assert_eq!(config_writes(&world), 0);
}
