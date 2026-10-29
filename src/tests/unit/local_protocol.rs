//! Local snapshots, ref protection and bounded backend observations.

#[test]
fn jujutsu_conflicts_in_published_ancestry_stop_all_effects() {
    for position in 0..3 {
        let w = World::new(3);
        {
            let mut memory = w.0.lock().unwrap();
            memory.jj = true;
            memory.commits[position].conflicted = true;
            memory.commits[0].author_email = "other@example.com".into();
            memory.commits[1].attrs.clear();
        }
        let error = w.run(&["--no-stack"]).unwrap_err();
        assert!(format!("{error:#}").contains("unresolved Jujutsu conflicts"));
        let memory = w.0.lock().unwrap();
        assert!(memory.prs.is_empty());
        assert_eq!(memory.commits[1].remote_ref(), "");
        drop(memory);
        assert!(!w.trace().iter().any(|event| {
            matches!(
                event["args"][0].as_str(),
                Some("push" | "describe" | "reword")
            )
        }));
    }
    for excluded in [false, true] {
        let w = World::new(3);
        {
            let mut memory = w.0.lock().unwrap();
            memory.jj = true;
            memory.commits[2].conflicted = true;
            memory.commits[2].title = "wip: excluded conflict".into();
            if excluded {
                for commit in &mut memory.commits {
                    commit.title = "wip: excluded".into();
                }
            }
        }
        w.run(&["--no-stack", "--skip-draft"]).unwrap();
        assert_eq!(w.0.lock().unwrap().prs.len(), if excluded { 0 } else { 2 });
    }
    let w = World::new(3);
    let history = {
        let mut memory = w.0.lock().unwrap();
        memory.commits[0].conflicted = true;
        log(&memory.commits).replace("\0tree1\0", "\0tree0\0")
    };
    w.reply("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature base..HEAD", 0, &history);
    let commands = w.commands();
    let repo = Repository {
        jj: true,
        ..Repository::default()
    };
    let visible = Git {
        commands: &commands,
        repo: &repo,
    }
    .stack("base", "HEAD", false)
    .unwrap();
    assert_eq!(visible.len(), 2);
    assert!(visible.iter().all(|commit| commit.conflicted));
    assert_eq!(visible[0].hash, commit(2).hash);
}

#[test]
fn jujutsu_conflict_protocol_requires_explicit_boolean_markers() {
    let w = World::new(1);
    let commands = w.commands();
    let bulk = format!(
        "jj --at-operation operation log -r {} --no-graph -T commit_id ++ \" \" ++ change_id ++ \" \" ++ conflict ++ \"\\n\"",
        commit(1).hash
    );
    for output in [
        "malformed".into(),
        format!("{} change1", commit(1).hash),
        format!("{} change1 unknown", commit(1).hash),
    ] {
        w.reply(&bulk, 0, &output);
        assert!(jj::fill_change_ids(&commands, &mut [commit(1)]).is_err());
    }
    for conflicted in [false, true] {
        w.reply(
            &bulk,
            0,
            &format!("{} change1 {conflicted}", commit(1).hash),
        );
        let mut commits = [commit(1)];
        jj::fill_change_ids(&commands, &mut commits).unwrap();
        assert_eq!(commits[0].conflicted, conflicted);
    }
    let working = "jj --ignore-working-copy log -r @ --no-graph -T conflict ++ \"\\0\" ++ if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\") ++ \"\\0\" ++ author.name() ++ \"\\0\" ++ author.email() ++ \"\\0\" ++ change_id ++ \"|\" ++ commit_id ++ \"|\" ++ description";
    w.reply(
        working,
        0,
        "unknown\0NONEMPTY|HAS-DESC\0Author\0a@example.com\0change|hash|Title",
    );
    assert!(jj::working_copy(&commands).is_err());
    for conflicted in [false, true] {
        w.reply(
            working,
            0,
            &format!("{conflicted}\0NONEMPTY|HAS-DESC\0Author\0a@example.com\0change|hash|Title"),
        );
        assert_eq!(
            jj::working_copy(&commands).unwrap().unwrap().conflicted,
            conflicted
        );
    }
}

#[test]
fn invalid_revision_syntax_is_rejected_before_discovery_or_snapshot() {
    for (args, diagnostic) in [
        (vec!["main...topic/1"], "symmetric-difference range"),
        (vec!["..topic/1"], "both endpoints non-empty"),
        (vec!["main.."], "both endpoints non-empty"),
        (vec!["main..topic/1", "topic/2"], "cannot combine a range"),
        (vec!["topic/2", "main..topic/1"], "cannot combine a range"),
    ] {
        let world = World::new(2);
        world.0.lock().unwrap().jj = true;
        let error = world.run(&args).unwrap_err();
        assert!(format!("{error:#}").contains(diagnostic));
        assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
    }
}

#[test]
fn native_stack_overrides_omit_unused_boolean_configuration() {
    use crate::{
        cli::Flags,
        config::{self, Loaded},
    };
    for (args, variables, expected) in [
        (vec!["--github-stack"], vec![], true),
        (vec!["--github-stack=false"], vec![], false),
        (vec!["--no-stack", "--github-stack"], vec![], false),
        (vec![], vec![("GIT_PR_GITHUB_STACK", "true")], true),
        (vec![], vec![("GIT_PR_GITHUB_STACK", "0")], false),
        (vec![], vec![("GIT_PR_GITHUB_STACK", "")], false),
        (vec![], vec![("GIT_PR_NO_STACK", "1")], false),
        (
            vec!["--github-stack=false"],
            vec![("GIT_PR_GITHUB_STACK", "true")],
            false,
        ),
        (
            vec!["--github-stack"],
            vec![("GIT_PR_GITHUB_STACK", "0")],
            true,
        ),
    ] {
        let world = World::new(1);
        world.fail("git config --type=bool --get git-pr.githubStack");
        let flags = Flags::parse(args.into_iter().map(str::to_owned).collect()).unwrap();
        let env = std::iter::once(("HOME", "/unit/home"))
            .chain(variables)
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        let Loaded::Ready(config) = config::load(
            &flags,
            &env,
            &mut world.commands(),
            world.as_ref(),
            world.as_ref(),
            false,
        )
        .unwrap() else {
            panic!("expected configuration")
        };
        assert_eq!(config.github_stack, expected);
        assert_eq!(
            world
                .trace()
                .iter()
                .filter(|event| { event["program"] == "git" && event["args"][0] == "config" })
                .count(),
            3
        );
    }
}

#[test]
fn default_publication_remote_avoids_irrelevant_branch_discovery() {
    let world = World::new(1);
    world.0.lock().unwrap().config.extend([
        ("remote.pushDefault".into(), "origin".into()),
        ("branch.topic/1.remote".into(), "upstream".into()),
    ]);
    world.fail("git branch --show-current");
    assert_eq!(world.config().repo.remote, "origin");
    assert!(!world.trace().iter().any(|event| {
        event["program"] == "git"
            && event["args"] == serde_json::json!(["branch", "--show-current"])
    }));
}

#[test]
fn invalid_configuration_and_missing_credentials_do_not_snapshot_jujutsu() {
    for (key, value, diagnostic) in [
        ("git-pr.output", "invalid".into(), "invalid --output"),
        (
            "git-pr.draftPattern",
            "x".repeat(1024 * 1024 + 1),
            "draft patterns exceed matcher resource limits",
        ),
    ] {
        let world = World::new(1);
        {
            let mut memory = world.0.lock().unwrap();
            memory.jj = true;
            memory.config.insert(key.into(), value);
        }
        let error = world.run(&[]).unwrap_err();
        assert!(format!("{error:#}").contains(diagnostic));
        assert!(!world.trace().iter().any(|event| {
            event["program"] == "jj" || event["kind"] == "file" || event["kind"] == "credential"
        }));
    }
    let world = World::new(1);
    {
        let mut memory = world.0.lock().unwrap();
        memory.jj = true;
        memory.files.clear();
    }
    let error = world.run(&[]).unwrap_err();
    assert!(format!("{error:#}").contains("failed to load"));
    assert!(!world.trace().iter().any(|event| {
        event["program"] == "jj" && event["args"] == serde_json::json!(["util", "snapshot"])
    }));
}

#[test]
fn inaccessible_workspace_metadata_does_not_select_another_backend() {
    let world = World::new(1);
    world.0.lock().unwrap().file_errors.insert(
        "/unit/repo/.jj".into(),
        std::io::ErrorKind::PermissionDenied,
    );
    let error = world.run(&[]).unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(diagnostic.contains("Jujutsu workspace metadata at /unit/repo/.jj"));
    assert!(diagnostic.contains("check file access"));
    assert!(diagnostic.contains("permission denied"));
    assert!(!world.trace().iter().any(|event| {
        event["kind"] == "http"
            || event["program"] == "jj"
            || event["args"] == serde_json::json!(["branchless", "--version"])
            || event["args"][0] == "push"
    }));
}
use super::support::{World, commit, log};
use crate::{
    commands::{CommandOutput, CommandRunner},
    git::Git,
    jj,
    model::Repository,
};
use std::{collections::BTreeMap, ffi::OsString, path::Path};

#[test]
fn framed_history_rejects_truncation_and_keeps_full_stack() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let key = "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature base..tip";
    let fields = [
        commit(1).hash,
        "name".into(),
        "mail@example.com".into(),
        "2025-01-01T00:00:00Z".into(),
        "tree".into(),
        String::new(),
        "title".into(),
    ];
    for length in 1..7 {
        w.reply(key, 0, &format!("{}\0", fields[..length].join("\0")));
        assert!(git.stack("base", "tip", false).is_err());
    }
    for value in ["invalid\0".to_owned(), format!("{}\0", "z".repeat(40))] {
        w.reply(key, 0, &value);
        assert!(git.stack("base", "tip", false).is_err());
    }
    w.reply(key, 0, &format!("\0{}", fields.join("\0")));
    assert_eq!(git.stack("base", "tip", false).unwrap().len(), 1);
    w.reply(key, 0, &log(&[commit(1), commit(2)]));
    assert_eq!(git.stack("base", "tip", false).unwrap().len(), 2);
    w.reply("git show --no-show-signature -s --format=%H %T base", 0, "");
    assert!(
        git.stack("base", "tip", false)
            .unwrap_err()
            .to_string()
            .contains("invalid base commit/tree observation for base")
    );
}

#[test]
fn ref_inventory_guards_and_compare_and_swap_failures_are_observable() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let c = commit(1);
    let inventory = "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/";
    w.fail(inventory);
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(inventory, 0, "topic/1|old");
    w.fail(&format!("git merge-base --is-ancestor old {}", c.hash));
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(
        &format!("git merge-base --is-ancestor old {}", c.hash),
        0,
        "",
    );
    w.reply(
        "git worktree list --porcelain",
        0,
        "branch refs/heads/topic/1",
    );
    git.sync_refs(std::slice::from_ref(&c));
    w.reply("git worktree list --porcelain", 0, "");
    w.fail(&format!("git update-ref refs/heads/topic/1 {} old", c.hash));
    git.sync_refs(std::slice::from_ref(&c));
    assert!(w.0.lock().unwrap().stderr.contains("local branch is stale"));
    w.reply(
        &format!("git update-ref refs/heads/topic/1 {} old", c.hash),
        0,
        "",
    );
    git.sync_refs(&[c]);
}

#[test]
fn bookmark_inventory_handles_conflicts_absence_identity_and_export_failure() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository {
        jj: true,
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let c = commit(1);
    let key = "jj --ignore-working-copy bookmark list -T name ++ \"\\0\" ++ if(conflict, \"1\", \"0\") ++ \"\\0\" ++ added_targets.map(|c| c.commit_id() ++ \" \" ++ c.change_id()).join(\",\") ++ \"\\n\"";
    w.fail("jj git import");
    git.sync_refs(std::slice::from_ref(&c));
    w.reply("jj git import", 0, "");
    w.fail(key);
    git.sync_refs(std::slice::from_ref(&c));
    for malformed in ["name", "name\x000", "name\x000\0invalid"] {
        w.reply(key, 0, malformed);
        assert!(jj::bookmarks(&commands).is_err());
    }
    for value in [
        String::new(),
        "topic/1\x001\0old change".into(),
        "topic/1\x000\0old change,other change".into(),
        "topic/1\x000\0".into(),
    ] {
        w.reply(key, 0, &value);
        git.sync_refs(std::slice::from_ref(&c));
    }
    w.reply(key, 0, "topic/1\x000\0old different");
    w.fail(&format!("git merge-base --is-ancestor old {}", c.hash));
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(key, 0, "topic/1\x000\0old change1");
    w.fail(&format!(
        "jj bookmark set topic/1 -r {} --allow-backwards",
        c.hash
    ));
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(
        &format!("jj bookmark set topic/1 -r {} --allow-backwards", c.hash),
        0,
        "",
    );
    for (status, output) in [(0, ""), (0, "Failed to export bookmarks"), (1, "failed")] {
        w.reply("jj git export", status, output);
        git.sync_refs(std::slice::from_ref(&c));
    }
    w.reply(key, 0, &format!("topic/1\x000\0{} change1", c.hash));
    w.reply(
        "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
        0,
        "topic/1|stale",
    );
    git.sync_refs(&[c]);
    assert!(w.0.lock().unwrap().stderr.contains("conflicted"));
}

#[test]
fn rewrite_uses_bulk_editor_with_literal_fallback_for_reserved_delimiters() {
    let w = World::new(2);
    let commands = w.commands();
    let repo = Repository {
        branchless: true,
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let mut commits = [commit(1), commit(2)];
    git.reword_all(&[]).unwrap();
    git.reword_all(&[&commits[0], &commits[1]]).unwrap();
    for text in ["contains ++ reword marker", "JJ: reserved marker"] {
        w.0.lock().unwrap().commits = vec![commit(1), commit(2)];
        commits[0].message = text.into();
        git.reword_all(&[&commits[0], &commits[1]]).unwrap();
    }
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    assert!(git.reword_all(&[&commit(1), &commit(2)]).is_err());
}

#[derive(Debug)]
struct Minimal;
impl CommandRunner for Minimal {
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
        _: &str,
        args: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
    ) -> anyhow::Result<CommandOutput> {
        if args.first().is_some_and(|a| a == "transport") {
            anyhow::bail!("transport failure");
        }
        Ok(CommandOutput {
            status: i32::from(!args.is_empty()),
            text: "diagnostic".into(),
        })
    }
}
#[test]
fn command_default_adapters_reject_unsupported_input_and_propagate_status() {
    let w = World::new(0);
    let mut commands = w.commands();
    commands.runner = std::sync::Arc::new(Minimal);
    commands.verbose = true;
    assert!(commands.git_data(&[], b"binary\0input").is_err());
    assert!(commands.git_data(&["failure"], &[]).is_err());
    assert!(commands.git_data(&["transport"], &[]).is_err());
    assert_eq!(commands.git_data(&[], &[]).unwrap(), "diagnostic");
    assert!(
        commands
            .runner
            .edit_messages("git", &[], Path::new("/unit"), &BTreeMap::new(), &[])
            .is_err()
    );
    let repo = Repository {
        branchless: true,
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    assert!(git.reword_all(&[&commit(1), &commit(2)]).is_err());
}

#[test]
fn malformed_local_observations_and_missing_identities_fail_closed() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    w.fail("git show --no-show-signature -s --format=%H %T base");
    assert!(git.stack("base", "tip", false).is_err());
    w.reply("git show --no-show-signature -s --format=%H %T base", 0, "");
    let key = "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature base..tip";
    for value in [
        format!(
            "{}\0\0email\02025-01-01T00:00:00Z\0tree\0\0title\0",
            commit(1).hash
        ),
        format!(
            "{}\0name\0\02025-01-01T00:00:00Z\0tree\0\0title\0",
            commit(1).hash
        ),
        format!(
            "{}\0name\0email\0invalid-date\0tree\0\0title\0",
            commit(1).hash
        ),
    ] {
        w.reply(key, 0, &value);
        assert!(git.stack("base", "tip", false).is_err());
    }
    w.0.lock().unwrap().jj = true;
    w.fail("jj util snapshot");
    assert!(w.run(&["--stop-after=validate"]).is_err());
    jj::fill_change_ids(&commands, &mut []).unwrap();
    let bulk = format!(
        "jj --at-operation operation log -r {} --no-graph -T commit_id ++ \" \" ++ change_id ++ \" \" ++ conflict ++ \"\\n\"",
        commit(1).hash
    );
    w.fail(&bulk);
    assert!(jj::fill_change_ids(&commands, &mut [commit(1)]).is_err());
    w.reply(&bulk, 0, "");
    assert!(jj::fill_change_ids(&commands, &mut [commit(1)]).is_err());
    w.fail("jj --ignore-working-copy log -r missing --no-graph -T change_id");
    assert!(jj::change_id(&commands, "missing").is_err());
}

#[test]
fn publication_recovery_requires_all_push_urls_to_match() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository {
        remote: "origin".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let c = commit(1);
    w.fail(&format!(
        "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1",
        c.hash
    ));
    w.reply(
        "git remote get-url --push --all -- origin",
        0,
        "first\nsecond",
    );
    let expected = format!("{}\trefs/heads/topic/1", c.hash);
    w.reply(
        "git ls-remote --refs -- first refs/heads/topic/1",
        0,
        &expected,
    );
    w.reply("git ls-remote --refs -- second refs/heads/topic/1", 0, "");
    assert!(
        git.push_all(&[&c])
            .unwrap_err()
            .to_string()
            .contains("failed to push 1 branch")
    );
    w.reply(
        "git ls-remote --refs -- second refs/heads/topic/1",
        0,
        &expected,
    );
    git.push_all(&[&c]).unwrap();
}

#[test]
fn remote_verification_and_divergent_tips_do_not_guess_success() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository {
        remote: "origin".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let c = commit(1);
    w.fail(&format!(
        "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1",
        c.hash
    ));
    w.fail("git ls-remote --refs -- https://github.com/org/repo.git refs/heads/topic/1");
    assert!(git.push_all(&[&c]).is_err());
    git.sync_refs(&[]);
    w.reply(
        "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
        0,
        "",
    );
    git.sync_refs(&[c]);
    w.reply(
        "git for-each-ref --contains HEAD --format=%(objectname) refs/heads/",
        0,
        "one\ntwo",
    );
    w.reply("git merge-base --independent one two", 0, "one\ntwo");
    assert_eq!(git.stack_tip("HEAD").unwrap(), "HEAD");
    w.fail("git merge-base --independent one two");
    assert!(git.stack_tip("HEAD").is_err());
}

#[derive(Debug)]
struct InvalidBytes;
impl CommandRunner for InvalidBytes {
    fn execute(
        &self,
        _: &str,
        _: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
    ) -> anyhow::Result<CommandOutput> {
        Ok(CommandOutput::default())
    }
    fn execute_data(
        &self,
        _: &str,
        _: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
        _: &[u8],
    ) -> anyhow::Result<crate::commands::StructuredOutput> {
        Ok(crate::commands::StructuredOutput {
            stdout: vec![255],
            ..crate::commands::StructuredOutput::default()
        })
    }
    fn edit_messages(
        &self,
        _: &str,
        _: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
        _: &[(String, String)],
    ) -> anyhow::Result<CommandOutput> {
        Ok(CommandOutput {
            status: 1,
            text: "editor failed".into(),
        })
    }
}
#[test]
fn binary_output_and_failed_editor_are_never_partial_success() {
    let w = World::new(0);
    let mut commands = w.commands();
    commands.runner = std::sync::Arc::new(InvalidBytes);
    assert!(commands.git_data(&[], &[]).is_err());
    let repo = Repository {
        branchless: true,
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    assert!(git.reword_all(&[&commit(1), &commit(2)]).is_err());
}

#[test]
fn publication_batches_limit_argument_bytes_as_well_as_ref_count() {
    let w = World::new(2);
    let commands = w.commands();
    let repo = Repository {
        remote: "origin".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let mut first = commit(1);
    let mut second = commit(2);
    first.set_attr(crate::model::REMOTE_REF, &"a".repeat(40_000));
    second.set_attr(crate::model::REMOTE_REF, &"b".repeat(40_000));
    git.push_all(&[&first, &second]).unwrap();
    assert_eq!(
        w.trace().iter().filter(|v| v["args"][0] == "push").count(),
        2
    );
    first.set_attr(crate::model::REMOTE_REF, &"a".repeat(65_536));
    assert!(git.push_all(&[&first]).is_err());
}

#[test]
fn literal_branch_validation_covers_git_component_and_byte_rules() {
    for name in [
        "topic",
        "topic/nested",
        "@",
        "日本語/é",
        "foo./bar",
        "topic/+one",
        "foo.LOCK",
        "refs/heads/HEAD",
    ] {
        assert!(crate::git::policy::valid_branch_name(name), "{name:?}");
    }
    for name in [
        "", "-topic", "HEAD", "topic.", "a..b", "a@{b", "@{-1}", "a b", "a\u{7f}b", "a~b", "a^b",
        "a:b", "a?b", "a*b", "a[b", "a\\b", "/a", "a/", "a//b", ".a", "a/.b", "a.lock/b",
    ] {
        assert!(!crate::git::policy::valid_branch_name(name), "{name:?}");
    }
}

#[test]
fn large_descendant_discovery_distinguishes_divergence_from_git_failure() {
    let w = World::new(1);
    let commands = w.commands();
    let config = w.config();
    let git = Git {
        commands: &commands,
        repo: &config.repo,
    };
    let tips = (0..129)
        .map(|n| format!("tip{n}"))
        .collect::<Vec<_>>()
        .join("\n");
    w.reply(
        "git for-each-ref --contains HEAD --format=%(objectname) refs/heads/",
        0,
        &tips,
    );
    let first = format!(
        "git merge-base --independent {}",
        (0..128)
            .map(|n| format!("tip{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let verify = format!(
        "git merge-base --independent tip128 {}",
        (0..127)
            .map(|n| format!("tip{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    w.reply(&verify, 0, "tip128");
    w.reply(
        "git merge-base --independent tip128 tip127 tip128",
        0,
        "tip128",
    );
    w.0.lock().unwrap().trace.clear();
    assert_eq!(git.stack_tip("HEAD").unwrap(), "tip128");
    assert_eq!(
        w.trace()
            .iter()
            .filter(|e| e["args"][0] == "merge-base")
            .count(),
        4
    );
    w.reply(&verify, 0, "tip0\ntip128");
    assert_eq!(git.stack_tip("HEAD").unwrap(), "HEAD");
    w.reply(&verify, 128, "unreadable object");
    assert!(format!("{:#}", git.stack_tip("HEAD").unwrap_err()).contains("unreadable object"));
    w.fail(&first);
    assert!(git.stack_tip("HEAD").is_err());
    for output in ["", "unknown"] {
        w.reply(&first, 0, output);
        assert!(
            git.stack_tip("HEAD")
                .unwrap_err()
                .to_string()
                .contains("invalid independent")
        );
    }
}

#[test]
fn invalid_histories_stop_before_remote_observation_or_rewriting() {
    for merge in [false, true] {
        let w = World::new(2);
        if merge {
            let mut history = log(&[commit(1), commit(2)]);
            history = history.replace(
                &format!("\0{}\0", commit(1).hash),
                &format!("\0{} {}\0", commit(1).hash, commit(3).hash),
            );
            w.reply("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..HEAD", 0, &history);
        } else {
            let mut m = w.0.lock().unwrap();
            m.commits[1].title.clear();
            m.commits[1].message.clear();
            m.commits[1].attrs.clear();
        }
        let error = w.run(&[]).unwrap_err();
        assert!(format!("{error:#}").contains(if merge {
            "merge commit"
        } else {
            "validation failed"
        }));
        assert!(!w.trace().iter().any(|v| {
            v["kind"] == "http"
                || ["push", "reword", "describe"]
                    .iter()
                    .any(|arg| v["args"][0] == *arg)
        }));
    }
}

#[test]
fn batched_depth_resolution_is_bounded_and_rejects_incomplete_identities() {
    let w = World::new(0);
    let commands = w.commands();
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    assert_eq!(git.at_depths("HEAD", &[]).unwrap(), Vec::<String>::new());
    let depths: Vec<_> = (0..257).collect();
    for chunk in depths.chunks(128) {
        let refs = chunk
            .iter()
            .map(|depth| format!("HEAD~{depth}^{{commit}}"))
            .collect::<Vec<_>>()
            .join(" ");
        let values = chunk.iter().fold(String::new(), |mut output, depth| {
            use std::fmt::Write as _;
            writeln!(output, "{depth:040x}").unwrap();
            output
        });
        w.reply(
            &format!("git rev-parse --revs-only --end-of-options {refs}"),
            0,
            &values,
        );
    }
    assert_eq!(git.at_depths("HEAD", &depths).unwrap().len(), 257);
    assert_eq!(w.trace().len(), 3);
    let key = "git rev-parse --revs-only --end-of-options HEAD~0^{commit}";
    for value in [
        String::new(),
        "bad".into(),
        "z".repeat(40),
        format!("{}\n{}", "a".repeat(40), "b".repeat(40)),
    ] {
        w.reply(key, 0, &value);
        assert!(git.at_depths("HEAD", &[0]).is_err());
    }
    w.reply(key, 0, &"a".repeat(64));
    assert_eq!(git.at_depths("HEAD", &[0]).unwrap(), ["a".repeat(64)]);
    w.fail(key);
    assert!(git.at_depths("HEAD", &[0]).is_err());
}

#[test]
fn publication_preflights_the_trunk_even_in_a_later_batch() {
    let w = World::new(129);
    let mut commits: Vec<_> = (1..=129).map(commit).collect();
    commits[128].attrs[0].1 = "develop".into();
    let commands = w.commands();
    let repo = Repository {
        trunk: "develop".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    assert!(
        git.push_all(&commits.iter().collect::<Vec<_>>())
            .unwrap_err()
            .to_string()
            .contains("remote trunk")
    );
    assert!(!w.trace().iter().any(|e| e["args"][0] == "push"));
}

#[test]
fn batched_revision_resolution_obeys_the_argument_byte_budget() {
    let w = World::new(0);
    let commands = w.commands();
    assert!(crate::git::resolve_refs(&commands, &["x".repeat(64 * 1024)]).is_err());
    let references = vec!["x".repeat(33 * 1024), "y".repeat(33 * 1024)];
    for reference in &references {
        w.reply(
            &format!("git rev-parse --revs-only --end-of-options {reference}^{{commit}}"),
            0,
            &"a".repeat(40),
        );
    }
    w.0.lock().unwrap().trace.clear();
    assert_eq!(
        crate::git::resolve_refs(&commands, &references)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(w.trace().len(), 2);
}

#[test]
fn rewrite_prepares_each_message_once_and_preserves_literal_execution_order() {
    use super::support::MESSAGE_SERIALISATIONS;
    for jj in [false, true] {
        for (size, delimiter) in [
            (0, ""),
            (1, ""),
            (3, ""),
            (3, "++ reword"),
            (3, "JJ: literal"),
        ] {
            let w = World::new(size);
            let commands = w.commands();
            let repo = Repository {
                jj,
                branchless: !jj,
                ..Repository::default()
            };
            let git = Git {
                commands: &commands,
                repo: &repo,
            };
            let mut commits: Vec<_> = (1..=size).map(commit).collect();
            if let Some(first) = commits.first_mut() {
                first.message = format!("Human message β\n{delimiter}");
            }
            let messages: Vec<_> = commits
                .iter()
                .map(crate::model::Commit::full_message)
                .collect();
            MESSAGE_SERIALISATIONS.set(0);
            git.reword_all(&commits.iter().collect::<Vec<_>>()).unwrap();
            assert_eq!(MESSAGE_SERIALISATIONS.get(), usize::try_from(size).unwrap());
            let trace = w.trace();
            if size == 1 || !delimiter.is_empty() {
                assert_eq!(trace.len(), usize::try_from(size).unwrap());
                for ((event, c), message) in trace
                    .iter()
                    .zip(commits.iter().rev())
                    .zip(messages.iter().rev())
                {
                    let expected = if jj {
                        serde_json::json!(["describe", "-r", c.change_id, "-m", message])
                    } else {
                        serde_json::json!(["reword", c.hash, "-m", message])
                    };
                    assert_eq!(event["args"], expected);
                }
            } else if size == 0 {
                assert_eq!(trace, [] as [serde_json::Value; 0]);
            } else {
                assert_eq!(trace.len(), 1);
                assert_eq!(trace[0]["args"][0], if jj { "describe" } else { "reword" });
            }
        }
    }
}

#[test]
fn retained_literal_editor_failure_does_not_fall_through_to_a_bulk_rewrite() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository {
        branchless: true,
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let c = commit(1);
    w.fail(&format!("git reword {} -m {}", c.hash, c.full_message()));
    assert!(git.reword_all(&[&c]).is_err());
}

#[test]
fn non_linear_observations_do_not_supply_fabricated_selection_depths() {
    let w = World::new(2);
    let config = w.config();
    let commands = w.commands();
    let git = Git {
        commands: &commands,
        repo: &config.repo,
    };
    git.observe_stack("origin/main", "HEAD", false).unwrap();
    let trace = w.trace();
    let args = trace
        .iter()
        .find(|event| event["args"][0] == "log")
        .unwrap()["args"]
        .as_array()
        .unwrap();
    let key = format!(
        "git {}",
        args.iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let data = log(&[commit(1), commit(2)]).replace(
        &format!("tree2\0{}\0", commit(1).hash),
        &format!("tree2\0{:040x}\0", 0),
    );
    w.reply(&key, 0, &data);
    assert!(
        git.observe_stack("origin/main", "HEAD", false)
            .unwrap()
            .depths
            .is_empty()
    );
}
