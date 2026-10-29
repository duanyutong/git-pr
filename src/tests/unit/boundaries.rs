use super::support::{Response, World, commit};
use crate::{
    cli::Flags,
    config::{self, Loaded},
    credentials,
    git::Git,
    github::GitHub,
    jj,
    model::Repository,
};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

#[test]
fn trunk_discovery_requires_a_branch_in_the_selected_remote_namespace() {
    let key = "git symbolic-ref refs/remotes/origin/HEAD";
    for target in [
        "",
        "refs/remotes/origin/",
        "refs/heads/main",
        "refs/remotes/other/main",
        "origin/main",
    ] {
        let w = World::new(1);
        w.reply(key, 0, target);
        let error = format!("{:#}", w.run(&[]).unwrap_err());
        assert!(
            error.contains("failed to detect remote trunk branch"),
            "{error}"
        );
        assert!(!w.trace().iter().any(|event| event["args"][0] == "push"));
    }
    let w = World::new(1);
    w.reply(key, 0, "refs/remotes/origin/release/main");
    assert_eq!(w.config().repo.trunk, "release/main");
}

#[test]
fn command_results_and_verbose_diagnostics_preserve_status_and_arguments() {
    let w = World::new(0);
    let mut commands = w.commands();
    commands.verbose = true;
    w.reply("git diagnostic two words", 0, "  first\nsecond\nthird\n ");
    assert_eq!(
        commands.git(&["diagnostic", "two words"]).unwrap(),
        "first\nsecond\nthird"
    );
    w.reply("jj diagnostic", 3, "");
    assert_eq!(
        format!("{:#}", commands.jj(&["diagnostic"]).unwrap_err()),
        "command `jj diagnostic` failed in /unit/repo: exit code 3"
    );
    w.reply("gh diagnostic", 7, " detail ");
    assert_eq!(
        format!("{:#}", commands.run("gh", &["diagnostic"]).unwrap_err()),
        "command `gh diagnostic` failed in /unit/repo: exit code 7\ndetail"
    );
    w.fail("git unavailable");
    assert!(commands.git(&["unavailable"]).is_err());
    commands.debug("");
    commands.debug("single");
    let diagnostic = w.0.lock().unwrap().stderr.clone();
    assert!(diagnostic.contains("\"two words\""));
    assert!(diagnostic.contains(" ┌ first\n │ second\n └ third"));
    assert!(diagnostic.contains(" │ single"));
    assert_eq!(
        crate::commands::parallel(vec![1_i64, 2], |n| Ok(n + 1)).unwrap(),
        [2, 3]
    );
}

#[test]
fn configuration_resolution_failures_retain_the_failed_reference() {
    for (args, key, diagnostic) in [
        (
            vec!["base..tip"],
            "git rev-parse --verify base^{commit}",
            "base ref",
        ),
        (
            vec!["base..tip"],
            "git rev-parse --verify tip^{commit}",
            "tip ref",
        ),
        (vec!["tip"], "git rev-parse --verify tip^{commit}", "commit"),
        (
            vec!["tip"],
            "git rev-parse --verify tip^^{commit}",
            "parent",
        ),
        (
            vec!["first", "tip"],
            "git rev-parse --verify tip^{commit}",
            "commit",
        ),
    ] {
        let w = World::new(1);
        w.fail(key);
        assert!(format!("{:#}", w.run(&args).unwrap_err()).contains(diagnostic));
    }
    let w = World::new(1);
    w.reply("git config --get git-pr.tags", 2, "bad setting");
    assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains("failed to read Git configuration"));
    w.0.lock().unwrap().responses.clear();
    w.0.lock().unwrap().files.clear();
    assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains("hosts"));
    let w = World::new(1);
    w.0.lock().unwrap().jj = true;
    w.fail("jj --ignore-working-copy log -r topic/1 --no-graph -T commit_id ++ \"\\n\"");
    w.run(&["--stop-after=get-commits", "topic/1"]).unwrap();
}

#[test]
fn git_stack_validates_empty_commits_and_reports_available_branches() {
    let w = World::new(1);
    let commands = w.commands();
    let repo = Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    w.reply("git rev-list --count base..tip", 0, "invalid");
    assert!(
        git.depth("base", "tip")
            .unwrap_err()
            .to_string()
            .contains("commit count")
    );
    w.fail("git rev-list --count base..tip");
    assert!(git.depth("base", "tip").is_err());
    w.reply("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature invalid..tip", 0, "bad");
    assert!(git.stack("invalid", "tip", false).is_err());
    w.reply("git branch --format=%(refname:strip=2)|%(objectname)", 0, "main|other\nshort|abc\nother|aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbad|hash|extra\n(HEAD detached at x)|abc\ninvalid");
    assert_eq!(
        git.local_branch(
            &commit(1),
            &commands
                .git(&["branch", "--format=%(refname:strip=2)|%(objectname)"])
                .unwrap()
        ),
        ""
    );
    assert!(w.0.lock().unwrap().stdout.contains("short -> abc"));
    w.reply(
        "git diff-tree --no-commit-id --name-only -r 0000000000000000000000000000000000000001",
        0,
        "",
    );
    w.reply(
        "git show --no-show-signature -s --format=%H %T base",
        0,
        &format!("{:040x} tree1", 0),
    );
    assert_eq!(git.stack("base", "tip", false).unwrap().len(), 0);
    w.fail("git rev-parse target");
    w.reply(
        "git for-each-ref --contains target --format=%(objectname) refs/heads/",
        0,
        "",
    );
    assert_eq!(git.stack_tip("target").unwrap(), "target");
    w.fail("git for-each-ref --contains target --format=%(objectname) refs/heads/");
    assert!(git.stack_tip("target").is_err());
    w.reply(
        "git for-each-ref --contains target --format=%(objectname) refs/heads/",
        0,
        "target\nancestor\nleaf\nleaf",
    );
    w.fail("git merge-base --is-ancestor leaf ancestor");
    assert_eq!(git.stack_tip("target").unwrap(), "leaf");
}

#[test]
fn jujutsu_working_copies_and_change_recovery_cover_success_and_failure() {
    let w = World::new(1);
    let mut commands = w.commands();
    commands.verbose = true;
    let repo = Repository {
        jj: true,
        user: "Unit Author".into(),
        email: "unit@example.com".into(),
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let key = "jj --ignore-working-copy log -r @ --no-graph -T conflict ++ \"\\0\" ++ if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\") ++ \"\\0\" ++ author.name() ++ \"\\0\" ++ author.email() ++ \"\\0\" ++ change_id ++ \"|\" ++ commit_id ++ \"|\" ++ description";
    for (value, count) in [
        (
            "false\0EMPTY|NO-DESC\0Other Author\0other@example.com\0working|hash|",
            1,
        ),
        (
            "false\0NONEMPTY|HAS-DESC\0Other Author\0other@example.com\0working|hash|Working title\nBody",
            2,
        ),
    ] {
        w.reply(key, 0, value);
        assert_eq!(git.stack("base", "HEAD", true).unwrap().len(), count);
    }
    w.fail(key);
    assert!(jj::working_copy(&commands).is_err());
    assert!(git.stack("base", "HEAD", true).is_err());
    w.reply(key, 0, "malformed");
    assert!(jj::working_copy(&commands).is_err());
    w.reply(
        key,
        0,
        "false\0NONEMPTY|HAS-DESC\0Other Author\0other@example.com\0working|hash| \nbody",
    );
    w.reply("git diff-tree --no-commit-id --name-only -r hash", 0, "");
    assert_eq!(git.stack("base", "HEAD", true).unwrap().len(), 1);
    w.reply(
        "git diff-tree --no-commit-id --name-only -r hash",
        0,
        "file",
    );
    assert!(git.stack("base", "HEAD", true).is_err());
    w.fail("jj --ignore-working-copy op log --limit 1 --no-graph -T id");
    assert!(git.stack("base", "HEAD", false).is_err());
    w.fail("jj --ignore-working-copy log -r @ --no-graph -T if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\")");
    assert!(git.clean().is_err());
    let mut c = commit(1);
    c.change_id.clear();
    assert!(
        git.reword(&c)
            .unwrap_err()
            .to_string()
            .contains("no change ID")
    );
}

#[test]
fn github_creation_rejects_bad_outputs_and_recovers_completed_requests() {
    for body in ["invalid JSON", "{}"] {
        let w = World::new(1);
        let config = w.config();
        let commands = w.commands();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: w.as_ref(),
            template: "",
        };
        w.http("POST pulls", vec![Response::Output(200, body.into())]);
        assert!(github.create(&mut commit(1), "main").is_err());
    }
    let w = World::new(1);
    w.run(&["--no-stack"]).unwrap();
    let config = w.config();
    let commands = w.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    w.http(
        "POST pulls",
        vec![Response::Failure("response lost".into())],
    );
    let mut c = commit(1);
    github.create(&mut c, "main").unwrap();
    assert_eq!(c.pr_number, 101);
    assert!(!c.newly_created);
    assert_eq!(w.0.lock().unwrap().prs.len(), 1);
}

#[test]
fn configuration_errors_propagate_from_every_required_setting() {
    for key in [
        "git config --type=bool --get git-pr.githubStack",
        "git config --type=bool --get git-pr.skipDraft",
        "git config --type=bool --get git-pr.stackUp",
        "git config --type=bool --get git-pr.branchFromTitle",
        "git config --type=bool --get git-pr.defaultDraft",
        "git config --type=bool --get git-pr.stackEmoji",
        "git config --get git-pr.draftPattern",
        "git config --get git-pr.output",
        "git config --get git-pr.addLabel",
        "git config --get git-pr.addTipLabel",
    ] {
        let w = World::new(1);
        w.fail(key);
        assert!(w.run(&[]).is_err(), "{key}");
    }
    for args in [vec!["--stop-after=unknown"], vec!["--output={unknown}"]] {
        assert!(World::new(1).run(&args).is_err());
    }
    let w = World::new(1);
    w.reply("git remote -v", 0, "unsupported remote");
    assert!(w.run(&[]).is_err());
    w.reply(
        "git remote -v",
        0,
        "origin https://github.com/org/repo.git (push)",
    );
    w.run(&["--stop-after=get-commits", "topic/1", "topic/1"])
        .unwrap();
    assert!(crate::git::policy::parse_remote_patterns("remote", &["["]).is_err());
    assert!(crate::git::policy::parse_remote_patterns("unsupported", &["valid"]).is_err());
}

#[test]
fn malformed_pr_documents_and_command_failures_are_not_absence() {
    let w = World::new(1);
    let mut config = w.config();
    let commands = w.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    w.http(
        "POST graphql",
        vec![Response::Output(200, "not json".into())],
    );
    assert!(github.find_head("topic/1", "open").is_err());
    assert!(github.verify_base(&commit(1), "main").is_err());
    w.http(
        &format!("GET commits/{}/pulls", commit(1).hash),
        vec![Response::Output(200, "not json".into())],
    );
    assert!(github.lookup(&commit(1)).is_err());
    w.0.lock().unwrap().http_responses.clear();
    w.http(
        "POST pulls",
        vec![Response::Failure("creation failed".into())],
    );
    assert!(github.number(&mut commit(1), "main").is_err());
    let mut c = commit(1);
    c.attrs.clear();
    assert_eq!(github.find(&c).unwrap(), 0);
    config.repo.host = "]".into();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    assert!(github.find_head("topic/1", "open").is_err());
}

#[test]
fn configuration_assembly_obeys_precedence_and_reports_native_boundary_failures() {
    let w = World::new(1);
    {
        let mut m = w.0.lock().unwrap();
        m.config.extend([
            ("git-pr.defaultDraft".into(), "false".into()),
            ("git-pr.stackUp".into(), "false".into()),
            ("git-pr.skipDraft".into(), "true".into()),
            ("git-pr.branchFromTitle".into(), "true".into()),
            ("git-pr.tags".into(), "configured".into()),
            ("git-pr.draftPattern".into(), "custom:*".into()),
            ("git-pr.output".into(), "markdown".into()),
            ("git-pr.addLabel".into(), "common".into()),
            ("git-pr.addTipLabel".into(), "tip".into()),
        ]);
    }
    let c = w.config();
    assert!(!c.draft && !c.stack_up && c.skip_draft);
    assert!(c.draft_patterns.matches("CUSTOM: value"));
    assert!(!c.draft_patterns.matches("WIP: value"));
    assert_eq!(c.labels, ["common"]);
    let flags = Flags::parse(
        [
            "--draft=true",
            "--stack-up=true",
            "--skip-draft",
            "--branch-from-title",
            "-t=local",
            "--draft-pattern=wip:*",
            "--output=url",
            "--add-label=override",
        ]
        .map(str::to_owned)
        .to_vec(),
    )
    .unwrap();
    let Loaded::Ready(c) = config::load(
        &flags,
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
        &mut w.commands(),
        w.as_ref(),
        w.as_ref(),
        false,
    )
    .unwrap() else {
        panic!("configuration");
    };
    assert!(c.draft && c.stack_up);
    assert_eq!(c.tags, ["local"]);
    assert_eq!(c.output, "{url}");
    assert_eq!(c.labels, ["override"]);
    for key in [
        "git config --get git-pr.tags",
        "git remote -v",
        "git symbolic-ref refs/remotes/origin/HEAD",
        "git config --get user.name",
        "git config --get user.email",
    ] {
        let w = World::new(0);
        w.fail(key);
        assert!(w.run(&["--stop-after=validate"]).is_err());
    }
    for (key, value) in [
        ("git symbolic-ref refs/remotes/origin/HEAD", ""),
        ("git config --get user.name", ""),
        ("git config --get user.email", ""),
    ] {
        let w = World::new(0);
        w.reply(key, 0, value);
        assert!(w.run(&[]).is_err());
    }
    let w = World::new(0);
    w.fail("git rev-parse --show-toplevel");
    w.0.lock().unwrap().jj = true;
    w.run(&["--stop-after=validate"]).unwrap();
    w.fail("jj workspace root");
    assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains("workspace root"));
    w.fail("jj git root");
    assert!(
        w.run(&[])
            .unwrap_err()
            .to_string()
            .contains("git repository")
    );
    let w = World::new(0);
    w.fail("git config --replace-all git-pr.tags x");
    assert!(w.run(&["--default-tags=x"]).is_err());
    for args in [vec!["a...b"], vec!["..b"], vec!["a.."], vec!["a..b", "c"]] {
        let w = World::new(0);
        assert!(w.run(&args).is_err());
    }
}

#[test]
fn workspace_and_backing_roots_preserve_path_whitespace_in_configuration() {
    for suffix in [" ", "\n", "\t\n"] {
        let w = World::new(0);
        w.0.lock().unwrap().jj = true;
        w.fail("git rev-parse --show-toplevel");
        let workspace = format!("/unit/workspace{suffix}");
        let backing = format!("/unit/backing.git{suffix}");
        w.reply("jj git root", 0, &format!("{backing}\n"));
        w.reply("jj workspace root", 0, &format!("{workspace}\n"));
        let mut commands = w.commands();
        let Loaded::Ready(config) = config::load(
            &Flags::parse(vec!["--dry-run".into()]).unwrap(),
            &BTreeMap::new(),
            &mut commands,
            w.as_ref(),
            w.as_ref(),
            false,
        )
        .unwrap() else {
            panic!("expected configuration");
        };

        assert_eq!(config.repo.root, Path::new(&workspace));
        assert!(config.repo.jj);
        assert_eq!(
            commands.env.get(std::ffi::OsStr::new("GIT_DIR")),
            Some(&std::ffi::OsString::from(backing))
        );
        assert!(!w.trace().iter().any(|event| {
            event["program"] == "jj" && event["args"] == json!(["util", "snapshot"])
        }));
    }
}

#[test]
fn credential_documents_and_template_precedence_use_only_in_memory_files() {
    #[derive(Debug)]
    struct Unavailable;
    impl credentials::CredentialStore for Unavailable {
        fn get(&self, _: &str, _: &crate::commands::Commands) -> anyhow::Result<String> {
            anyhow::bail!("credential store unavailable");
        }
    }
    let w = World::new(0);
    let path = Path::new("/hosts");
    assert!(credentials::load(path, "github.com", w.as_ref(), &w.commands(), w.as_ref()).is_err());
    for document in [
        "[bad",
        "other: {}",
        "github.com:",
        "github.com: {}",
        "github.com:\n  oauth_token: token\n",
    ] {
        w.0.lock()
            .unwrap()
            .files
            .insert(path.into(), document.into());
        let result = credentials::load(path, "github.com", w.as_ref(), &w.commands(), w.as_ref());
        if document == "github.com: {}" {
            assert_eq!(result.unwrap().0, "unit-token");
        } else if document.contains("oauth_token") {
            assert_eq!(result.unwrap().0, "token");
        } else {
            assert!(result.is_err());
        }
    }
    w.0.lock()
        .unwrap()
        .files
        .insert(path.into(), "github.com: {}".into());
    w.0.lock().unwrap().token.clear();
    assert!(credentials::load(path, "github.com", w.as_ref(), &w.commands(), w.as_ref()).is_err());
    assert!(
        credentials::load(path, "github.com", &Unavailable, &w.commands(), w.as_ref()).is_err()
    );
    assert_eq!(
        format!("{:?}", credentials::Secret("never-print-me".into())),
        "[redacted]"
    );
    assert_eq!(
        config::load_template(Path::new("/unit/repo"), w.as_ref()).unwrap(),
        crate::stack::description::BODY_TEMPLATE
    );
    for template in [
        "pull_request_template.md",
        "PULL_REQUEST_TEMPLATE.md",
        "docs/pull_request_template.md",
        "docs/PULL_REQUEST_TEMPLATE.md",
        ".github/pull_request_template.md",
        ".github/PULL_REQUEST_TEMPLATE.md",
    ] {
        w.0.lock()
            .unwrap()
            .files
            .insert(Path::new("/unit/repo").join(template), template.into());
        assert_eq!(
            config::load_template(Path::new("/unit/repo"), w.as_ref()).unwrap(),
            template
        );
    }
}

#[test]
fn refs_are_guarded_and_push_errors_require_the_exact_remote_postcondition() {
    let w = World::new(1);
    let c = commit(1);
    let config = w.config();
    let commands = w.commands();
    let git = Git {
        commands: &commands,
        repo: &config.repo,
    };
    w.fail("git worktree list --porcelain");
    git.sync_refs(std::slice::from_ref(&c));
    assert!(!w.trace().iter().any(|v| v["args"][0] == "update-ref"));
    for worktree in ["branch refs/heads/topic/1", ""] {
        w.reply("git worktree list --porcelain", 0, worktree);
        w.reply(
            "git rev-parse --verify refs/heads/topic/1^{commit}",
            0,
            "old",
        );
        git.sync_refs(std::slice::from_ref(&c));
    }
    w.fail("git rev-parse --verify refs/heads/topic/1^{commit}");
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(
        "git rev-parse --verify refs/heads/topic/1^{commit}",
        0,
        &c.hash,
    );
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(
        "git rev-parse --verify refs/heads/topic/1^{commit}",
        0,
        "old",
    );
    w.fail(&format!("git merge-base --is-ancestor old {}", c.hash));
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(
        &format!("git merge-base --is-ancestor old {}", c.hash),
        0,
        "",
    );
    w.fail(&format!("git update-ref refs/heads/topic/1 {} old", c.hash));
    git.sync_refs(std::slice::from_ref(&c));
    let key = format!(
        "git push --porcelain --no-follow-tags -f -- origin {}:refs/heads/topic/1",
        c.hash
    );
    w.fail(&key);
    w.reply(
        "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/topic/1",
        0,
        &format!("{}\trefs/heads/topic/1", c.hash),
    );
    git.push_all(&[&c]).unwrap();
    w.reply(
        "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/topic/1",
        0,
        "other refs/heads/topic/1",
    );
    assert!(git.push_all(&[&c]).is_err());
    for name in [" topic", "topic ", "(HEAD detached at x)"] {
        assert!(Git::validate_remote_ref(name).is_err());
    }
    assert!(
        git.reword(&c)
            .unwrap_err()
            .to_string()
            .contains("neither jj nor git-branchless")
    );
}

#[test]
fn jujutsu_cardinality_recovery_and_bookmark_conflicts_are_explicit() {
    let w = World::new(1);
    let commands = w.commands();
    let query = "jj --ignore-working-copy log -r query --no-graph -T commit_id ++ \"\\n\"";
    for (value, count) in [("", 0), ("hash", 1), ("hash1\nhash2", 2)] {
        w.reply(query, 0, value);
        assert_eq!(jj::resolve_revision(&commands, "query").is_ok(), count == 1);
    }
    w.reply(
        "jj --ignore-working-copy log -r hash --no-graph -T change_id",
        0,
        "\nchange\n",
    );
    assert_eq!(jj::change_id(&commands, "hash").unwrap(), "change");
    w.reply(
        "jj --ignore-working-copy log -r hash --no-graph -T change_id",
        0,
        "",
    );
    assert!(jj::change_id(&commands, "hash").is_err());
    let repo = Repository {
        jj: true,
        ..Repository::default()
    };
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    let c = commit(1);
    git.sync_refs(std::slice::from_ref(&c));
    let key = "jj --ignore-working-copy log -r bookmarks(exact:\"topic/1\") --no-graph -T commit_id ++ \"\\n\"";
    let old = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    w.reply(key, 0, &format!("{old}\n{}", c.hash));
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(key, 0, &c.hash);
    git.sync_refs(std::slice::from_ref(&c));
    w.reply(key, 0, old);
    git.sync_refs(std::slice::from_ref(&c));
    w.reply("jj git export", 0, "Failed to export bookmarks");
    git.sync_refs(std::slice::from_ref(&c));
    w.fail("jj git export");
    git.sync_refs(std::slice::from_ref(&c));
    w.fail(&format!(
        "jj bookmark set topic/1 -r {} --allow-backwards",
        c.hash
    ));
    git.sync_refs(&[c]);
}

#[test]
fn github_lookup_absence_errors_creation_and_base_updates_are_distinct() {
    let w = World::new(1);
    let config = w.config();
    let commands = w.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "Template",
    };
    let mut c = commit(1);
    assert_eq!(github.find_head("", "all").unwrap(), 0);
    w.http(
        "POST graphql",
        vec![Response::Failure("lookup failed".into())],
    );
    assert!(github.number(&mut c, "main").is_err());
    assert!(w.0.lock().unwrap().prs.is_empty());
    w.0.lock().unwrap().http_responses.clear();
    c.message.clear();
    github.create(&mut c, "main").unwrap();
    assert_eq!(c.pr_number, 101);
    assert!(c.newly_created);
    assert_eq!(github.find(&c).unwrap(), 101);
    assert_eq!(github.lookup(&c).unwrap(), 101);
    c.pr_number = 0;
    assert_eq!(github.lookup(&c).unwrap(), 101);
    c.pr_number = 101;
    github.verify_base(&c, "main").unwrap();
    assert!(github.verify_base(&c, "other").is_err());
    assert!(!w.trace().iter().any(|event| event["method"] == "PATCH"));
    let mut unknown = commit(2);
    unknown.skip = true;
    assert_eq!(github.number(&mut unknown, "main").unwrap(), 0);
    unknown.attrs.clear();
    assert_eq!(github.lookup(&unknown).unwrap(), 0);
    for reply in [
        Response::Output(200, "not json".into()),
        Response::Failure("other failure".into()),
    ] {
        w.http(&format!("GET commits/{}/pulls", unknown.hash), vec![reply]);
        assert!(github.find(&unknown).is_err());
    }
    w.http(
        &format!("GET commits/{}/pulls", unknown.hash),
        vec![Response::Output(404, "No commit found".into())],
    );
    assert_eq!(github.find(&unknown).unwrap(), 0);
    unknown.hash = commit(3).hash;
    w.http(
        &format!("GET commits/{}/pulls", unknown.hash),
        vec![Response::Output(
            200,
            json!([{"number":1,"state":"open","head":{"ref":"topic/1","repo":{"full_name":"org/repo"}}},{"number":2,"state":"open","head":{"ref":"topic/2","repo":{"full_name":"org/repo"}}}]).to_string(),
        )],
    );
    assert!(
        github
            .find(&unknown)
            .unwrap_err()
            .to_string()
            .contains("multiple open PRs")
    );
    w.http(
        "GET pulls/102",
        vec![Response::Output(500, "API failed".into())],
    );
    assert!(format!("{:#}", github.get(102).unwrap_err()).contains("API failed"));
    assert!(!w.0.lock().unwrap().stdout.contains("API failed"));
    assert!(!w.trace().iter().any(|v| v["program"] == "gh"));
}

#[test]
fn native_stack_consent_rebuild_and_failures_preserve_requested_branch_order() {
    for answers in [vec![false], vec![true]] {
        let w = World::new(2);
        w.run(&["--github-stack", "--yes"]).unwrap();
        {
            let mut m = w.0.lock().unwrap();
            m.trace.clear();

            m.answers.extend(answers.iter().copied());
        }
        w.http("GET stacks/9", vec![Response::Output(200, json!({"number":9,"base":{"ref":"stale"},"pull_requests":[{"number":101},{"number":102}]}).to_string())]);
        let config = w.config();
        let commands = w.commands();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 102;
        github.manage_stack(&commits, "main").unwrap();
        assert_eq!(
            w.trace().iter().any(|v| v["path"] == "stacks/9/unstack"),
            answers == [true]
        );
    }
    for response in [
        Response::Failure("unstack failed".into()),
        Response::Output(200, "{}".into()),
    ] {
        let w = World::new(2);
        w.run(&["--github-stack", "--yes"]).unwrap();
        w.http("GET stacks/9", vec![Response::Output(200, json!({"number":9,"base":{"ref":"stale"},"pull_requests":[{"number":101},{"number":102}]}).to_string())]);
        w.http("POST stacks/9/unstack", vec![response]);
        let mut config = w.config();
        config.auto_accept = true;
        let commands = w.commands();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: w.as_ref(),
            template: "",
        };
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 102;
        assert!(github.manage_stack(&commits, "main").is_err());
        github.manage_stack(&commits[..1], "main").unwrap();
    }
}
