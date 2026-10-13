use crate::{
    cli::Flags,
    config::paths,
    credentials::keychain,
    diagnostics::errors::{self, Fatal},
    git::policy as git_policy,
    github::policy as github_policy,
    jj::policy as jj_policy,
};

#[test]
fn remote_metadata_and_host_paths_are_parsed_without_external_discovery() {
    let patterns = git_policy::REMOTE_PATTERNS.map(|pattern| regex::Regex::new(pattern).unwrap());
    for remote in [
        "origin\tgit@github.com:org/repo.git (push)",
        "origin\thttps://github.com/org/repo.git (push)",
        "origin\thttps://fixture-user:fixture-password@github.com/org/repo.git (push)",
        "origin\thttps://fixture-token@github.com/org/repo.git (push)",
        "origin\thttps://fixture%40user:fixture%2Fpassword@github.com/org/repo.git (push)",
        "origin\thttps://github.com/org/repo (fetch)\norigin\thttps://enterprise.example/org/repo (push)",
    ] {
        let parsed = git_policy::parse_remote(remote, &patterns).unwrap();
        assert_eq!(parsed.remote, "origin");
        assert_eq!(parsed.name, "org/repo");
        assert!(matches!(
            parsed.host.as_str(),
            "github.com" | "enterprise.example"
        ));
    }
    for (remote, expected_remote, expected_name) in [
        (
            "origin https://github.com/org/tool.v2.git (push)",
            "origin",
            "org/tool.v2",
        ),
        (
            "my-origin git@github.com:org/repo.git (push)",
            "my-origin",
            "org/repo",
        ),
        (
            "origin git@github.com:org/.github.git (push)",
            "origin",
            "org/.github",
        ),
        (
            "up.stream https://github.com/org/tool.git.git (push)",
            "up.stream",
            "org/tool.git",
        ),
        (
            " origin https://github.com/org/tool (fetch) \nbackup https://github.com/other/repo (fetch)",
            "origin",
            "org/tool",
        ),
    ] {
        let parsed = git_policy::parse_remote(remote, &patterns).unwrap();
        assert_eq!(
            (parsed.remote.as_str(), parsed.name.as_str()),
            (expected_remote, expected_name)
        );
    }
    let parsed = git_policy::parse_remote("origin https://github.com/org/repo", &patterns).unwrap();
    assert_eq!(parsed.name, "org/repo");
    assert!(git_policy::parse_remote("invalid", &patterns).is_err());
    for invalid in [
        "",
        "origin https://github.com/org/repo/extra (push)",
        "origin https://github.com/org/ (push)",
        "origin https://github.com/org/repo unknown",
        "origin https://github.com/org/repo?query (push)",
        "origin https://github.com/org/repo#fragment (push)",
        "origin https://github.com/org?query/repo (push)",
        "origin https://github.com#fragment/org/repo (push)",
    ] {
        assert!(
            git_policy::parse_remote(invalid, &patterns).is_err(),
            "{invalid}"
        );
    }
    let invalid = [regex::Regex::new(r"origin\s+").unwrap()];
    assert_eq!(
        git_policy::parse_remote("origin https://github.com/org/repo", &invalid)
            .unwrap_err()
            .to_string(),
        "remote pattern must capture remote, host, owner and repository"
    );
    for (path, expected) in [
        ("~/hosts.yml", "/home/fixture/hosts.yml"),
        ("/hosts.yml", "/hosts.yml"),
    ] {
        assert_eq!(
            paths::expand_path(path, "/home/fixture"),
            std::path::PathBuf::from(expected)
        );
    }
    assert_eq!(git_policy::short_hash("short"), "short");
    assert_eq!(git_policy::short_hash("0123456789abcd"), "0123456789ab");
}

#[test]
fn pr_selection_preserves_order_and_stops_at_the_first_matching_open_pr() {
    use crate::model::{PullRequest, pull_request::Reference};
    let prs: Vec<_> = [
        (1, "other", "open"),
        (0, "topic", "closed"),
        (2, "topic", "closed"),
        (3, "topic", "open"),
        (4, "topic", "open"),
    ]
    .into_iter()
    .map(|(number, head, state)| PullRequest {
        number,
        head: Reference { name: head.into() },
        state: state.into(),
        ..PullRequest::default()
    })
    .collect();
    let inspected = std::cell::Cell::new(0);
    assert_eq!(
        github_policy::select_pr(
            prs.iter().inspect(|_| inspected.set(inspected.get() + 1)),
            "topic"
        ),
        3
    );
    assert_eq!(inspected.get(), 4);
    assert_eq!(github_policy::select_pr(&prs[..3], "topic"), 0);
    assert_eq!(github_policy::select_pr(&prs[2..3], "topic"), 2);
    assert_eq!(github_policy::select_pr(&prs, "absent"), 0);
}

#[test]
fn common_and_commit_labels_survive_tip_label_reconciliation() {
    let mut commit = crate::model::Commit::default();
    commit.set_attr("tags", "commit-tag");
    let defaults = vec!["default".into()];
    let common = vec!["shared".into(), "SHARED".into()];
    let tip = vec![
        "Shared".into(),
        "COMMIT-tag".into(),
        "Default".into(),
        "tip-only".into(),
        "TIP-ONLY".into(),
    ];
    let (add, remove) = github_policy::label_changes(&commit, &defaults, &common, &tip, false);
    assert_eq!(add, ["default", "commit-tag", "shared"]);
    assert_eq!(remove, ["tip-only"]);
    let (add, remove) = github_policy::label_changes(&commit, &defaults, &common, &tip, true);
    assert_eq!(add, ["default", "commit-tag", "shared", "tip-only"]);
    assert_eq!(remove, Vec::<String>::new());
    assert_eq!(
        github_policy::label_changes(&crate::model::Commit::default(), &[], &[], &[], false),
        (Vec::<String>::new(), Vec::<String>::new())
    );
    let (add, remove) = github_policy::label_changes(
        &crate::model::Commit::default(),
        &[],
        &["éBug".into(), "ébug".into(), "Ébug".into()],
        &[],
        false,
    );
    assert_eq!(add, ["éBug", "Ébug"]);
    assert_eq!(remove, Vec::<String>::new());
}

#[test]
fn draft_exclusions_are_repeatable_and_do_not_restore_other_exclusions() {
    use crate::{model::Commit, stack::selection};
    let source = vec![
        Commit {
            title: "wip: feature".into(),
            ..Commit::default()
        },
        Commit {
            title: "Ready".into(),
            ..Commit::default()
        },
        Commit {
            title: "wip: excluded".into(),
            skip: true,
            ..Commit::default()
        },
    ];
    let patterns = selection::DraftPatterns::new(&["wip:*".into()]).unwrap();
    for (enabled, include) in [(false, false), (true, true)] {
        let mut commits = source.clone();
        assert_eq!(
            selection::mark_drafts(&mut commits, enabled, include, &patterns),
            Vec::<usize>::new()
        );
        assert!(!commits[0].skip && commits[2].skip);
    }
    let mut rewritten = source;
    assert_eq!(
        selection::mark_drafts(&mut rewritten, true, false, &patterns),
        [0]
    );
    assert!(rewritten[0].skip && !rewritten[1].skip && rewritten[2].skip);
    assert_eq!(
        selection::mark_drafts(&mut rewritten, true, false, &patterns),
        Vec::<usize>::new()
    );
    rewritten[0].hash = "new identity".into();
    rewritten[0].skip = false;
    assert_eq!(
        selection::mark_drafts(&mut rewritten, true, false, &patterns),
        [0]
    );
}

#[test]
fn native_environment_values_have_explicit_unicode_boundaries() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let invalid = OsString::from_vec(vec![0xff]);
    let values = vec![
        (OsString::from("HOME"), OsString::from("/fixture")),
        (OsString::from("GIT_PR_OUTPUT"), OsString::from("url")),
        (OsString::from("OTHER"), OsString::from("value")),
        (OsString::from("UNRELATED_BINARY"), invalid.clone()),
        (invalid.clone(), OsString::from("unused")),
    ];
    let env = crate::config::settings::unicode_environment(values).unwrap();
    assert_eq!(env.len(), 3);
    assert_eq!(env["HOME"], "/fixture");
    assert_eq!(env["GIT_PR_OUTPUT"], "url");
    assert_eq!(env["OTHER"], "value");
    for key in ["HOME", "GIT_PR_OUTPUT"] {
        let error = crate::config::settings::unicode_environment(vec![(
            OsString::from(key),
            invalid.clone(),
        )])
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("ERROR: environment variable {key} must be valid UTF-8")
        );
    }
    assert_eq!(
        crate::config::settings::unicode_environment(Vec::new()).unwrap(),
        std::collections::BTreeMap::new()
    );
}

#[test]
fn worker_errors_and_panics_are_joined_before_returning() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let completed = AtomicBool::new(false);
    let error = crate::commands::parallel(vec![0, 1, 2], |item| -> anyhow::Result<usize> {
        match item {
            0 => anyhow::bail!("first failure"),
            1 => panic!("later worker panic"),
            _ => {
                completed.store(true, Ordering::SeqCst);
                Ok(item)
            }
        }
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "first failure");
    assert!(completed.load(Ordering::SeqCst));
    let error = crate::commands::parallel(vec![0], |_| -> anyhow::Result<()> {
        panic!("worker panic")
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "worker terminated unexpectedly");
    assert_eq!(crate::commands::parallel(vec![1, 2], Ok).unwrap(), [1, 2]);
    assert_eq!(
        crate::commands::parallel(Vec::<usize>::new(), Ok).unwrap(),
        Vec::<usize>::new()
    );
}

#[test]
fn version_output_remains_available_without_git_metadata() {
    let version = crate::cli::output::version(None, None);
    assert!(version.starts_with(concat!(
        "git-pr version ",
        env!("CARGO_PKG_VERSION"),
        " (git unknown, dirty=unknown, target="
    )));
    for field in [", rustc=", ", built="] {
        assert!(version.contains(field));
    }
    let version = crate::cli::output::version(Some("test-sha"), Some("false"));
    assert!(version.contains("(git test-sha, dirty=false, target="));
}

#[test]
fn credential_encodings_accept_supported_forms_and_reject_invalid_data() {
    for (encoded, expected) in [
        (" token ", "token"),
        ("go-keyring-base64:dG9rZW4=", "token"),
        ("go-keyring-encoded:746f6b656e", "token"),
    ] {
        assert_eq!(keychain::decode_keychain(encoded).unwrap(), expected);
    }
    for encoded in [
        "go-keyring-base64:!",
        "go-keyring-base64:/w==",
        "go-keyring-encoded:f",
        "go-keyring-encoded:zz",
        "go-keyring-encoded:ff",
        "go-keyring-encoded:é",
        "go-keyring-encoded:aéa",
    ] {
        assert!(keychain::decode_keychain(encoded).is_err(), "{encoded:?}");
    }
}

#[test]
fn jujutsu_status_and_pr_creation_output_handle_valid_and_malformed_values() {
    for (output, expected) in [
        ("EMPTY|NO-DESC", true),
        ("EMPTY|HAS-DESC", true),
        ("INVALID|HAS-DESC", false),
        ("NONEMPTY|HAS-DESC", true),
        ("NONEMPTY|NO-DESC", false),
        ("", false),
        ("NONEMPTY|Feature|extra", false),
    ] {
        assert_eq!(jj_policy::clean_status(output), expected);
    }
    assert_eq!(
        crate::tests::legacy::parse_pr_url("https://github.com/org/repo/pull/123\n").unwrap(),
        123
    );
    assert_eq!(
        crate::tests::legacy::parse_pr_url("/pull/invalid\nhttps://github.com/org/repo/pull/456")
            .unwrap(),
        456
    );
    assert!(crate::tests::legacy::parse_pr_url("No URL").is_err());
    assert!(crate::tests::legacy::parse_pr_url("/pull/18446744073709551616").is_err());
}

#[test]
fn cli_defaults_and_explicit_values_are_distinct() {
    let defaults = Flags::default();
    assert_eq!(defaults.timeout(), 20);
    for (name, expected) in [
        ("gh-hosts", "~/.config/gh/hosts.yml"),
        ("timeout", "20"),
        ("output", ""),
    ] {
        assert_eq!(defaults.string(name), expected);
        assert!(!defaults.supplied.contains(name));
    }
    let arguments = [
        "--gh-hosts=/fixture/hosts.yml",
        "--timeout=0x10",
        "--output={url}",
    ];
    let flags = Flags::parse(arguments.map(str::to_owned).into()).unwrap();
    assert_eq!(flags.timeout(), 16);
    let repeated = Flags::parse(vec!["--timeout=1".into(), "--timeout=-2".into()]).unwrap();
    assert_eq!(repeated.timeout(), -2);
    for (name, expected) in [
        ("gh-hosts", "/fixture/hosts.yml"),
        ("timeout", "0x10"),
        ("output", "{url}"),
    ] {
        assert_eq!(flags.string(name), expected);
        assert!(flags.supplied.contains(name));
    }
}

#[test]
fn cli_integer_syntax_and_overflow_are_reported() {
    for (value, expected) in [
        ("0", Some(0)),
        ("-9223372036854775808", Some(i64::MIN)),
        ("9223372036854775807", Some(i64::MAX)),
        ("0x_10", Some(16)),
        ("", None),
        ("-", None),
        ("0x", None),
        ("1_", None),
        ("_1", None),
        ("1__0", None),
        ("08", None),
        ("١", None),
        ("-9223372036854775809", None),
        ("999999999999999999999999999999999999999999", None),
    ] {
        assert_eq!(crate::cli::parse_integer(value).ok(), expected, "{value:?}");
    }
}

#[test]
fn fatal_context_determines_exit_status_without_losing_the_cause() {
    let ordinary = anyhow::anyhow!("transport failed");
    assert_eq!(errors::status(&ordinary), 1);
    let fatal = ordinary.context(Fatal).context("description update failed");
    assert_eq!(errors::status(&fatal), 2);
    assert_eq!(
        format!("{fatal:#}"),
        "description update failed: ERROR: transport failed"
    );
    assert_eq!(Fatal.to_string(), "ERROR");
}
