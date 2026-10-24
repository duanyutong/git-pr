//! Failed observations shall retain their cause and shall not authorise mutations.
use super::support::{Response, World, commit};
use crate::{
    commands::{CommandRunner, StructuredOutput, Terminal},
    config, credentials,
    git::Git,
};
use std::{collections::BTreeMap, ffi::OsString, path::Path, sync::Arc};

fn error(world: &Arc<World>, args: &[&str]) -> String {
    format!("{:#}", world.run(args).unwrap_err())
}

fn assert_no_mutations(world: &World) {
    assert!(!world.trace().iter().any(|operation| {
        operation["kind"] == "http"
            || matches!(
                operation["args"][0].as_str(),
                Some("push" | "reword" | "describe")
            )
    }));
}

#[test]
fn url_credentials_are_redacted_without_changing_other_diagnostic_text() {
    use std::borrow::Cow;
    for (text, expected) in [
        ("", ""),
        ("fixture@example.com", "fixture@example.com"),
        ("https://@host/repo", "https://[redacted]@host/repo"),
        (
            "fixture@example.com https://host:443/repo",
            "fixture@example.com https://host:443/repo",
        ),
        (
            "https://user:fixture/password@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:123/password@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:fi@xture/pass@word@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:fi://xture/password@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:fi/xture password@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:fi/xture\npassword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://host:443/path?email=fixture@example.com",
            "https://[redacted]@example.com",
        ),
        (
            "https://user:password@first/repo https://token@second/repo",
            "https://[redacted]@second/repo",
        ),
        ("https://github.com/org/repo", "https://github.com/org/repo"),
        (
            "https://github.com/org/repo?author=fixture@example.com",
            "https://github.com/org/repo?author=fixture@example.com",
        ),
        (
            "https://user:password@github.com/org/repo",
            "https://[redacted]@github.com/org/repo",
        ),
        (
            "'https://token@github.com'",
            "'https://[redacted]@github.com'",
        ),
        ("https://token@github.com", "https://[redacted]@github.com"),
        (
            "https://token@github.com?query",
            "https://[redacted]@github.com?query",
        ),
        (
            "https://token@github.com#fragment",
            "https://[redacted]@github.com#fragment",
        ),
        (
            "http://fixture%40user:fixture%2Fpassword@host/repo",
            "http://[redacted]@host/repo",
        ),
        (
            "https://user:pässword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:p@ssword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:p?ssword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:p#ssword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:p ssword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:p\tssword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://user:p\nssword@host/repo",
            "https://[redacted]@host/repo",
        ),
        (
            "https://host Contact user@example.com",
            "https://[redacted]@example.com",
        ),
        (
            "https://host?email=user@example.com",
            "https://[redacted]@example.com",
        ),
        (
            "https://first@host/a\nssh://second@host/b\thttps://host/c",
            "https://[redacted]@host/a\nssh://[redacted]@host/b\thttps://host/c",
        ),
    ] {
        let redacted = crate::diagnostics::errors::redact_url_credentials(text);
        assert_eq!(redacted, expected);
        assert_eq!(matches!(redacted, Cow::Borrowed(_)), text == expected);
    }
}

#[test]
fn credential_bearing_remotes_use_the_host_and_keep_diagnostics_confidential() {
    let remote = "origin https://fixture-user:fixture-password@github.com/org/repo.git (push)";
    let w = World::new(1);
    w.reply("git remote -v", 0, remote);
    w.run(&["--no-stack", "-v"]).unwrap();
    assert!(w.trace().iter().any(|event| event["kind"] == "http"));
    assert!(!w.0.lock().unwrap().stderr.contains("fixture-password"));

    let mut commands = w.commands();
    commands.verbose = true;
    assert_eq!(commands.git(&["remote", "-v"]).unwrap(), remote);
    commands.warn(remote);
    assert!(
        w.0.lock()
            .unwrap()
            .stderr
            .contains("https://[redacted]@github.com")
    );
    assert!(!w.0.lock().unwrap().stderr.contains("fixture-password"));

    w.reply("git remote -v", 128, remote);
    for diagnostic in [
        format!("{:#}", commands.git(&["remote", "-v"]).unwrap_err()),
        format!(
            "{:#}",
            commands.git_data(&["remote", "-v"], &[]).unwrap_err()
        ),
    ] {
        assert!(diagnostic.contains("exit code 128"));
        assert!(diagnostic.contains("https://[redacted]@github.com"));
        assert!(!diagnostic.contains("fixture-password"));
    }
    assert!(!w.0.lock().unwrap().stderr.contains("fixture-password"));

    for rejected in [
        "origin http://fixture-user:fixture-password@github.com/org/repo.git (push)",
        "origin https://fixture-user:fixture-password@github.com/org/repo/extra (push)",
    ] {
        let w = World::new(1);
        w.reply("git remote -v", 0, rejected);
        let diagnostic = error(&w, &["--no-stack"]);
        assert!(diagnostic.contains("failed to parse remote url"));
        assert!(diagnostic.contains("[redacted]@github.com"));
        assert!(!diagnostic.contains("fixture-password"));
        assert_no_mutations(&w);
    }
}

#[test]
fn malformed_url_user_information_is_rejected_without_exposing_credentials() {
    for separator in ['/', '?', '#', ' ', '\t', '\n', '\r', '\u{a0}'] {
        let password = format!("fixture{separator}password");
        let remote =
            format!("origin https://fixture-user:{password}@github.com/org/repo.git (push)");
        let w = World::new(1);
        w.reply("git remote -v", 0, &remote);
        let diagnostic = error(&w, &["--no-stack", "-v"]);
        assert!(diagnostic.contains("failed to parse remote url"));
        assert!(diagnostic.contains("percent-encode"));
        assert!(diagnostic.contains("https://[redacted]@github.com/org/repo.git"));
        assert!(!diagnostic.contains(&password));
        assert_no_mutations(&w);

        let mut commands = w.commands();
        commands.verbose = true;
        assert_eq!(commands.git(&["remote", "-v"]).unwrap(), remote);
        commands.warn(&remote);
        w.reply("git remote -v", 128, &remote);
        for diagnostic in [
            format!("{:#}", commands.git(&["remote", "-v"]).unwrap_err()),
            format!(
                "{:#}",
                commands.git_data(&["remote", "-v"], &[]).unwrap_err()
            ),
        ] {
            assert!(diagnostic.contains("[redacted]@github.com"));
            assert!(!diagnostic.contains(&password));
        }
        assert!(!w.0.lock().unwrap().stderr.contains(&password));

        let encoded = remote.replace(
            &password,
            &percent_encoding::utf8_percent_encode(&password, percent_encoding::NON_ALPHANUMERIC)
                .to_string(),
        );
        let parsed = crate::git::policy::parse_remote_patterns(
            &encoded,
            &crate::git::policy::REMOTE_PATTERNS,
        )
        .unwrap();
        assert_eq!(parsed.host, "github.com");
        assert_eq!(parsed.name, "org/repo");
    }
}

#[test]
fn ambiguous_path_separators_in_passwords_remain_confidential() {
    for password in [
        "fi/xture/password",
        "fi@xture/pass@word",
        "fi://xture/password",
        "fi/xture password",
        "fi/xture\npassword",
        "fi/xture?password",
        "fi/xture#password",
    ] {
        let remote =
            format!("origin https://fixture-user:{password}@github.com/org/repo.git (push)");
        let w = World::new(1);
        w.reply("git remote -v", 0, &remote);
        let diagnostic = error(&w, &["--no-stack"]);
        assert!(diagnostic.contains("failed to parse remote url"));
        assert!(diagnostic.contains("https://[redacted]@github.com/org/repo.git"));
        assert!(!diagnostic.contains(password));
        assert_no_mutations(&w);
        let mut commands = w.commands();
        commands.verbose = true;
        assert_eq!(commands.git(&["remote", "-v"]).unwrap(), remote);
        commands.warn(&remote);
        w.reply("git remote -v", 128, &remote);
        for diagnostic in [
            format!("{:#}", commands.git(&["remote", "-v"]).unwrap_err()),
            format!(
                "{:#}",
                commands.git_data(&["remote", "-v"], &[]).unwrap_err()
            ),
        ] {
            assert!(!diagnostic.contains(password));
            assert!(diagnostic.contains("[redacted]@github.com"));
        }
        assert!(!w.0.lock().unwrap().stderr.contains(password));
    }
}

#[test]
fn discovery_and_cleanliness_failures_are_actionable_and_preserve_causes() {
    let w = World::new(1);
    w.reply(
        "git symbolic-ref refs/remotes/origin/HEAD",
        128,
        "fatal: ref is not a symbolic ref",
    );
    let diagnostic = error(&w, &[]);
    for expected in [
        "refs/remotes/origin/HEAD",
        "git remote set-head origin --auto",
        "fatal: ref is not a symbolic ref",
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
    assert_no_mutations(&w);

    let w = World::new(1);
    w.reply(
        "git status --porcelain=v1 --untracked-files=no",
        128,
        "index file corrupt",
    );
    let diagnostic = error(&w, &[]);
    assert!(diagnostic.contains("working-tree") && diagnostic.contains("index file corrupt"));
    assert!(!diagnostic.contains("reports uncommitted changes"));
    assert_no_mutations(&w);

    let w = World::new(1);
    w.0.lock().unwrap().jj = true;
    w.fail("jj version");
    let diagnostic = error(&w, &[]);
    assert!(
        diagnostic.contains("Jujutsu workspace detected") && diagnostic.contains("unavailable")
    );
    assert_no_mutations(&w);
}

#[test]
fn invalid_working_copy_markers_and_failed_diff_reads_stop_submission() {
    let w = World::new(1);
    w.0.lock().unwrap().jj = true;
    w.reply("jj --ignore-working-copy log -r @ --no-graph -T if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\")", 0, "unexpected status");
    assert!(error(&w, &[]).contains("invalid Jujutsu working-copy status"));
    assert_no_mutations(&w);

    let w = World::new(1);
    w.reply(
        "git show --no-show-signature -s --format=%H %T refs/remotes/origin/main",
        0,
        &format!("{} tree99", commit(99).hash),
    );
    w.reply(
        &format!(
            "git diff-tree --no-commit-id --name-only -r {}",
            commit(1).hash
        ),
        128,
        "missing tree object",
    );
    let diagnostic = error(&w, &[]);
    assert!(
        diagnostic.contains("file changes in commit") && diagnostic.contains("missing tree object")
    );
    assert_no_mutations(&w);
}

#[test]
fn template_access_and_encoding_errors_prevent_default_template_fallback() {
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::InvalidData,
    ] {
        let w = World::new(1);
        let path = Path::new("/unit/repo/.github/PULL_REQUEST_TEMPLATE.md");
        w.0.lock().unwrap().file_errors.insert(path.into(), kind);
        let diagnostic = error(&w, &[]);
        assert!(
            diagnostic.contains("PR template") && diagnostic.contains("PULL_REQUEST_TEMPLATE.md")
        );
        assert_no_mutations(&w);
        assert!(config::load_template(Path::new("/unit/repo"), w.as_ref()).is_err());
    }
}

#[test]
fn credential_diagnostics_identify_the_source_and_never_echo_yaml_contents() {
    #[derive(Debug)]
    struct Locked;
    impl credentials::CredentialStore for Locked {
        fn get(&self, _: &str, _: &crate::commands::Commands) -> anyhow::Result<String> {
            anyhow::bail!("credential collection is locked");
        }
    }
    let w = World::new(0);
    let path = Path::new("/unit/hosts.yml");
    w.0.lock().unwrap().files.insert(
        path.into(),
        "github.com:\n  oauth_token: [secret-token-fixture\n".into(),
    );
    let diagnostic = format!(
        "{:#}",
        credentials::load(path, "github.com", w.as_ref(), &w.commands(), w.as_ref()).unwrap_err()
    );
    assert!(diagnostic.contains("/unit/hosts.yml") && diagnostic.contains("YAML"));
    assert!(!diagnostic.contains("secret-token-fixture"));

    w.0.lock()
        .unwrap()
        .files
        .insert(path.into(), "github.com: {}".into());
    let diagnostic = format!(
        "{:#}",
        credentials::load(path, "github.com", &Locked, &w.commands(), w.as_ref()).unwrap_err()
    );
    assert!(
        diagnostic.contains("credential collection is locked")
            && diagnostic.contains("gh auth login")
    );
    assert!(!diagnostic.contains("no GitHub token found"));
}

#[test]
fn failed_publication_verification_retains_both_failures_and_uncertainty() {
    let w = World::new(1);
    w.reply(
        &format!(
            "git push --porcelain --no-follow-tags --force-with-lease=refs/heads/topic/1: -- origin {}:refs/heads/topic/1",
            commit(1).hash
        ),
        128,
        "connection lost during push",
    );
    w.0.lock().unwrap().responses.insert(
        "git ls-remote --refs -- https://github.com/org/repo.git refs/heads/topic/1".into(),
        vec![
            super::support::Response::Output(0, String::new()),
            super::support::Response::Output(128, "remote verification unavailable".into()),
        ]
        .into(),
    );
    let diagnostic = error(&w, &[]);
    for expected in [
        "connection lost during push",
        "remote verification unavailable",
        "some refs may already have changed",
        "completed",
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
    assert!(w.0.lock().unwrap().prs.is_empty());
}

#[test]
fn failed_publication_requires_resolvable_push_destinations() {
    for (status, output) in [(128, "push destination unavailable"), (0, "")] {
        let w = World::new(1);
        w.reply(
            &format!(
                "git push --porcelain --no-follow-tags --force-with-lease=refs/heads/topic/1: -- origin {}:refs/heads/topic/1",
                commit(1).hash
            ),
            128,
            "connection lost during push",
        );
        w.0.lock().unwrap().responses.insert(
            "git remote get-url --push --all -- origin".into(),
            vec![
                super::support::Response::Output(0, "https://github.com/org/repo.git".into()),
                super::support::Response::Output(0, "https://github.com/org/repo.git".into()),
                super::support::Response::Output(status, output.into()),
            ]
            .into(),
        );
        let diagnostic = error(&w, &[]);
        for expected in [
            "connection lost during push",
            if status == 0 {
                "no push destinations"
            } else {
                output
            },
        ] {
            assert!(diagnostic.contains(expected), "{diagnostic}");
        }
        assert!(w.0.lock().unwrap().prs.is_empty());
    }
}

#[test]
fn ancestry_failure_is_distinct_from_unrelated_history() {
    let w = World::new(1);
    let commands = w.commands();
    let config = w.config();
    let git = Git {
        commands: &commands,
        repo: &config.repo,
    };
    for (status, output, expected) in [
        (0, "", Some(true)),
        (1, "", Some(false)),
        (128, "bad object", None),
        (1, "unexpected failure", None),
    ] {
        w.reply("git merge-base --is-ancestor old new", status, output);
        let result = git.is_ancestor("old", "new");
        assert_eq!(result.ok(), expected);
    }
    w.fail("git merge-base --is-ancestor old new");
    assert!(git.is_ancestor("old", "new").is_err());

    w.reply(
        "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
        0,
        "topic/1|old",
    );
    let key = format!("git merge-base --is-ancestor old {}", commit(1).hash);
    w.reply(&key, 128, "bad object");
    git.sync_refs(&[commit(1)]);
    let diagnostic = w.0.lock().unwrap().stderr.clone();
    assert!(diagnostic.contains("bad object") && diagnostic.contains("push succeeded"));
    assert!(!diagnostic.contains("not an ancestor"));
    assert!(
        !w.trace()
            .iter()
            .any(|operation| operation["args"][0] == "update-ref")
    );
}

#[test]
fn structured_command_failures_retain_output_and_encoding_context() {
    #[derive(Debug)]
    struct Output {
        status: i32,
        bytes: Vec<u8>,
    }
    impl CommandRunner for Output {
        fn execute(
            &self,
            _: &str,
            _: &[String],
            _: &Path,
            _: &BTreeMap<OsString, OsString>,
        ) -> anyhow::Result<crate::commands::CommandOutput> {
            anyhow::bail!("unused adapter method");
        }
        fn execute_data(
            &self,
            _: &str,
            _: &[String],
            _: &Path,
            _: &BTreeMap<OsString, OsString>,
            _: &[u8],
        ) -> anyhow::Result<StructuredOutput> {
            Ok(StructuredOutput {
                status: self.status,
                stdout: self.bytes.clone(),
                stderr: b"stderr diagnostic".to_vec(),
            })
        }
    }
    let w = World::new(0);
    let mut commands = w.commands();
    commands.runner = Arc::new(Output {
        status: 7,
        bytes: b"stdout diagnostic".to_vec(),
    });
    let diagnostic = format!("{:#}", commands.git_data(&["log"], &[]).unwrap_err());
    for expected in [
        "git log",
        "exit code 7",
        "stdout diagnostic",
        "stderr diagnostic",
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
    commands.runner = Arc::new(Output {
        status: 0,
        bytes: vec![0xff],
    });
    assert!(
        format!("{:#}", commands.git_data(&["log"], &[]).unwrap_err()).contains("not valid UTF-8")
    );
    assert!(w.failure().is_none());
}

#[test]
fn github_errors_are_reported_once_with_method_url_and_completed_effects() {
    let w = World::new(2);
    w.http(
        "POST stacks",
        vec![Response::Output(403, "organisation SSO required".into())],
    );
    let diagnostic = error(&w, &["--github-stack", "--yes"]);
    for expected in [
        "native stack reconciliation failed",
        "GitHub POST",
        "/stacks",
        "organisation SSO required",
        "completed PR changes are retained",
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
    assert!(
        !w.0.lock()
            .unwrap()
            .stdout
            .contains("organisation SSO required")
    );
    assert_eq!(w.0.lock().unwrap().prs.len(), 2);
}

#[test]
fn http_status_diagnostics_bound_server_output_and_offer_recovery_actions() {
    for (status, hint) in [
        (401, "authentication"),
        (403, "SSO"),
        (404, "availability"),
        (409, "PR state"),
        (422, "PR state"),
        (429, "rate limit"),
        (503, "mutation completed"),
        (400, "requested operation"),
    ] {
        let error = crate::github::http::HttpStatusError {
            url: "https://api.github.com/repos/org/repo/pulls/1".into(),
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            body: "service diagnostic".into(),
            quota: crate::github::limits::Quota::default(),
        };
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains("/pulls/1")
                && diagnostic.contains("service diagnostic")
                && diagnostic.contains(hint),
            "{diagnostic}"
        );
    }
    let error = crate::github::http::HttpStatusError {
        url: "https://api.github.com/graphql".into(),
        status: reqwest::StatusCode::BAD_GATEWAY,
        body: format!("{}undisplayed-suffix", "é".repeat(3000)),
        quota: crate::github::limits::Quota::default(),
    };
    let diagnostic = error.to_string();
    assert!(diagnostic.contains("truncated") && !diagnostic.contains("undisplayed-suffix"));
    assert!(diagnostic.len() < 4500);
}

#[test]
fn actual_uncommitted_changes_have_backend_specific_recovery_guidance() {
    for jj in [false, true] {
        let w = World::new(1);
        w.0.lock().unwrap().jj = jj;
        if jj {
            w.reply("jj --ignore-working-copy log -r @ --no-graph -T if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\")", 0, "NONEMPTY|NO-DESC");
        } else {
            w.reply(
                "git status --porcelain=v1 --untracked-files=no",
                0,
                " M tracked-file",
            );
        }
        let diagnostic = error(&w, &[]);
        assert!(diagnostic.contains("uncommitted changes"));
        assert!(diagnostic.contains(if jj { "jj describe" } else { "commit or stash" }));
        assert_no_mutations(&w);
    }
}

#[test]
fn unrelated_ref_histories_remain_distinct_from_successful_ancestry() {
    for jj in [false, true] {
        for status in [0, 1] {
            let w = World::new(1);
            w.0.lock().unwrap().jj = jj;
            let config = w.config();
            let commands = w.commands();
            let git = Git {
                commands: &commands,
                repo: &config.repo,
            };
            w.reply(
                "git for-each-ref --format=%(refname:strip=2)|%(objectname) refs/heads/",
                0,
                "topic/1|old",
            );
            if jj {
                w.reply("jj --ignore-working-copy bookmark list -T name ++ \"\\0\" ++ if(conflict, \"1\", \"0\") ++ \"\\0\" ++ added_targets.map(|c| c.commit_id() ++ \" \" ++ c.change_id()).join(\",\") ++ \"\\n\"", 0, "topic/1\x000\0old different-change");
            }
            w.reply(
                &format!("git merge-base --is-ancestor old {}", commit(1).hash),
                status,
                "",
            );
            git.sync_refs(&[commit(1)]);
            assert_eq!(
                w.trace().iter().any(|operation| matches!(
                    operation["args"][0].as_str(),
                    Some("update-ref" | "bookmark")
                ) && operation["args"][1] != "list"),
                status == 0
            );
            let diagnostic = w.0.lock().unwrap().stderr.clone();
            assert_eq!(
                diagnostic.contains(if jj {
                    "different change"
                } else {
                    "not an ancestor"
                }),
                status == 1
            );
        }
    }
}

#[test]
fn failed_creation_recovery_retains_the_original_failure_and_unknown_result() {
    let w = World::new(1);
    let config = w.config();
    let commands = w.commands();
    let github = crate::github::GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    w.http(
        "POST pulls",
        vec![Response::Failure("creation response was lost".into())],
    );
    w.http(
        "POST graphql",
        vec![Response::Failure("recovery observation unavailable".into())],
    );
    let diagnostic = format!("{:#}", github.create(&mut commit(1), "main").unwrap_err());
    for expected in [
        "creation response was lost",
        "recovery observation unavailable",
        "a PR may already exist",
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
}

#[test]
fn commit_filtering_preserves_order_after_skipping_an_empty_predecessor() {
    let w = World::new(3);
    let commands = w.commands();
    let repo = crate::model::Repository::default();
    let git = Git {
        commands: &commands,
        repo: &repo,
    };
    w.reply(
        "git show --no-show-signature -s --format=%H %T base",
        0,
        &format!("{:040x} tree1", 0),
    );
    let commits = git.stack("base", "tip", false).unwrap();
    assert_eq!(
        commits
            .iter()
            .map(|commit| &commit.hash)
            .collect::<Vec<_>>(),
        [&commit(2).hash, &commit(3).hash]
    );
}
