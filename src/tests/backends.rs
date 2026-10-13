use super::support::Fixture;

#[test]
fn invalid_inputs_do_not_snapshot_unsaved_jujutsu_working_copy() {
    let mut fixture = Fixture::new(1, true);
    enable_jj(&mut fixture);
    let operation = || {
        fixture
            .app
            .commands
            .jj(&[
                "--ignore-working-copy",
                "op",
                "log",
                "--limit",
                "1",
                "--no-graph",
                "-T",
                "id",
            ])
            .unwrap()
    };
    let initial_operation = operation();
    fixture.write("not-yet-snapshotted", "working-copy change\n");
    for (args, diagnostic) in [
        (vec!["--output=invalid"], "invalid --output"),
        (vec!["main...HEAD"], "symmetric-difference range"),
        (vec!["..HEAD"], "both endpoints non-empty"),
        (vec!["main..HEAD", "HEAD"], "cannot combine a range"),
    ] {
        let error = fixture.run(&args).unwrap_err();
        assert!(format!("{error:#}").contains(diagnostic));
        assert_eq!(operation(), initial_operation);
    }
    assert!(fixture.service.prs.lock().unwrap().is_empty());
}

fn enable_jj(fixture: &mut Fixture) {
    let config = fixture.home.join("jj.toml");
    std::fs::write(
        &config,
        "[user]\nname = 'Fixture Author'\nemail = 'fixture@example.com'\n[ui]\ncolor = 'never'\n",
    )
    .unwrap();
    fixture
        .app
        .commands
        .env
        .insert("JJ_CONFIG".into(), config.into_os_string());
    fixture
        .app
        .commands
        .jj(&["version"])
        .expect("Jujutsu is required; run setup.sh or install the declared backend version");
    fixture
        .app
        .commands
        .jj(&["git", "init", "--colocate"])
        .unwrap();
}

#[test]
fn jj_colocated_rewrite_exports_bookmarks_and_preserves_checkout() {
    let mut f = Fixture::new(3, false);
    enable_jj(&mut f);
    let before = f
        .app
        .commands
        .jj(&["log", "-r", "@", "--no-graph", "-T", "change_id"])
        .unwrap();
    f.run(&[]).unwrap();
    let after = f
        .app
        .commands
        .jj(&["log", "-r", "@", "--no-graph", "-T", "change_id"])
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(f.service.prs.lock().unwrap().len(), 3);
    for i in 1..=3 {
        let branch = format!("topic/{i}");
        let hash = f.git(&["rev-parse", &branch]);
        assert!(
            f.git(&["show", "-s", "--format=%B", &branch])
                .contains(&format!("Remote-Ref: {branch}"))
        );
        assert!(
            f.git(&["ls-remote", "origin", &format!("refs/heads/{branch}")])
                .starts_with(&hash)
        );
    }
    assert!(
        !f.trace()
            .iter()
            .any(|v| v["program"] == "git" && v["args"][0] == "checkout")
    );
}

#[test]
fn jj_separate_workspace_discovers_backing_git_and_resolves_selected_tip() {
    let mut f = Fixture::new(3, false);
    enable_jj(&mut f);
    f.git(&["config", "git-pr.addLabel", "workspace-label"]);
    f.git(&["config", "git-pr.defaultDraft", "off"]);
    f.git(&["config", "git-pr.output", "url"]);
    let workspace = f.temp.path().join("workspace");
    f.app
        .commands
        .jj(&[
            "workspace",
            "add",
            "--no-colocate",
            workspace.to_str().unwrap(),
        ])
        .unwrap();
    f.app.commands.cwd = workspace.clone();
    f.cwd = workspace;
    assert!(!f.cwd.join(".git").exists());
    let before = f
        .app
        .commands
        .jj(&["log", "-r", "@", "--no-graph", "-T", "change_id"])
        .unwrap();
    f.run(&["topic/1..topic/2"]).unwrap();
    let after = f
        .app
        .commands
        .jj(&["log", "-r", "@", "--no-graph", "-T", "change_id"])
        .unwrap();
    assert_eq!(before, after);
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[&101]["head"]["ref"], "topic/2");
    assert_eq!(prs[&101]["draft"], false);
    assert_eq!(prs[&101]["labels"], serde_json::json!(["workspace-label"]));
}

#[test]
fn jj_nonempty_described_working_copy_is_included() {
    let mut f = Fixture::new(1, true);
    enable_jj(&mut f);
    f.write("working.txt", "working copy changes\n");
    f.app
        .commands
        .jj(&[
            "describe",
            "-m",
            "Working | feature\n\nRemote-Ref: topic/working",
        ])
        .unwrap();
    f.run(&[]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[&102]["title"], "Working | feature");
    assert_eq!(prs[&102]["base"]["ref"], "topic/1");
}

#[test]
fn jj_undescribed_working_copy_is_rejected() {
    let mut f = Fixture::new(1, true);
    enable_jj(&mut f);
    f.write("working.txt", "unspecified\n");
    assert!(
        f.run(&[])
            .unwrap_err()
            .to_string()
            .contains("uncommitted changes")
    );
}

#[test]
fn jj_unordered_multi_selection_recovers_stable_change_ids() {
    let mut f = Fixture::new(3, false);
    enable_jj(&mut f);
    f.run(&["topic/3", "topic/2"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[&101]["head"]["ref"], "topic/2");
    assert_eq!(prs[&102]["base"]["ref"], "topic/2");
}

#[test]
fn dry_run_preserves_git_and_jujutsu_repository_state() {
    for backend in ["git", "branchless", "jj", "jj-workspace"] {
        let mut f = Fixture::new(3, false);
        if backend == "branchless" {
            f.git(&["branchless", "init", "--main-branch", "main"]);
        }
        if backend.starts_with("jj") {
            enable_jj(&mut f);
            if backend == "jj-workspace" {
                let workspace = f.temp.path().join("preview-workspace");
                f.app
                    .commands
                    .jj(&[
                        "workspace",
                        "add",
                        "--no-colocate",
                        workspace.to_str().unwrap(),
                    ])
                    .unwrap();
                f.app.commands.cwd = workspace.clone();
                f.cwd = workspace;
                let root = f.app.commands.jj(&["git", "root"]).unwrap();
                f.app.commands.env.insert("GIT_DIR".into(), root.into());
            }
        }
        let refs = f.git(&["show-ref"]);
        let history = f.git(&["log", "--all", "--format=%H %B"]);
        let operation = if backend.starts_with("jj") {
            f.app
                .commands
                .jj(&[
                    "--ignore-working-copy",
                    "op",
                    "log",
                    "--limit",
                    "1",
                    "--no-graph",
                    "-T",
                    "id",
                ])
                .unwrap()
        } else {
            String::new()
        };
        // An untracked file must not be snapshotted by a preview.
        if backend.starts_with("jj") {
            std::fs::write(f.cwd.join("preview-only"), "unsnapshotted").unwrap();
        }
        f.run(&["--dry-run"]).unwrap();
        assert_eq!(refs, f.git(&["show-ref"]), "{backend}");
        assert_eq!(
            history,
            f.git(&["log", "--all", "--format=%H %B"]),
            "{backend}"
        );
        assert!(f.service.prs.lock().unwrap().is_empty());
        if backend.starts_with("jj") {
            assert_eq!(
                operation,
                f.app
                    .commands
                    .jj(&[
                        "--ignore-working-copy",
                        "op",
                        "log",
                        "--limit",
                        "1",
                        "--no-graph",
                        "-T",
                        "id"
                    ])
                    .unwrap(),
                "{backend}"
            );
        }
    }
}

#[test]
fn jj_working_copy_retains_its_actual_author_and_respects_filtering() {
    let mut f = Fixture::new(1, true);
    enable_jj(&mut f);
    f.write("working.txt", "someone else's work\n");
    f.app
        .commands
        .jj(&[
            "metaedit",
            "--author",
            "Other Author <other@example.com>",
            "-m",
            "Other feature\n\nRemote-Ref: topic/other",
        ])
        .unwrap();
    let author = f
        .app
        .commands
        .jj(&["log", "-r", "@", "--no-graph", "-T", "author.email()"])
        .unwrap();
    assert_eq!(author, "other@example.com");
    f.run(&["--no-stack", "--output={email} {title}"]).unwrap();
    assert!(
        !f.service
            .prs
            .lock()
            .unwrap()
            .values()
            .any(|pr| pr["head"]["ref"] == "topic/other")
    );
    f.run(&[
        "--no-stack",
        "--include-other-authors",
        "--output={email} {title}",
    ])
    .unwrap();
    assert!(
        f.service
            .prs
            .lock()
            .unwrap()
            .values()
            .any(|pr| pr["head"]["ref"] == "topic/other")
    );
    assert!(f.output().contains("other@example.com Other feature"));
}

#[test]
fn jj_explicit_list_can_include_the_described_working_copy() {
    let mut f = Fixture::new(1, true);
    enable_jj(&mut f);
    f.write("working.txt", "selected working-copy change\n");
    f.app
        .commands
        .jj(&[
            "describe",
            "-m",
            "Working feature\n\nRemote-Ref: topic/working",
        ])
        .unwrap();
    f.run(&["--no-stack", "@", "topic/1"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[&102]["head"]["ref"], "topic/working");
}

#[test]
fn jj_merge_working_copy_stops_before_publication() {
    let mut f = Fixture::new(1, true);
    enable_jj(&mut f);
    f.app.commands.jj(&["new", "main"]).unwrap();
    f.write("side.txt", "independent work\n");
    f.app.commands.jj(&["describe", "-m", "Side work"]).unwrap();
    f.app
        .commands
        .jj(&[
            "new",
            "@",
            "topic/1",
            "-m",
            "Merge feature\n\nRemote-Ref: topic/merge",
        ])
        .unwrap();
    f.service.trace.lock().unwrap().clear();
    assert!(f.run(&["--no-stack"]).is_err());
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(!f.trace().iter().any(|e| e["args"][0] == "push"));
}

fn conflicted_linear_history() -> Fixture {
    let mut f = Fixture::new(0, false);
    enable_jj(&mut f);
    f.app.commands.jj(&["new", "main"]).unwrap();
    f.write("base.txt", "first change\n");
    f.app
        .commands
        .jj(&["describe", "-m", "First change\n\nRemote-Ref: topic/first"])
        .unwrap();
    f.app
        .commands
        .jj(&["bookmark", "create", "topic/first"])
        .unwrap();
    f.app.commands.jj(&["new", "main"]).unwrap();
    f.write("base.txt", "second change\n");
    f.app
        .commands
        .jj(&[
            "describe",
            "-m",
            "Second change\n\nRemote-Ref: topic/second",
        ])
        .unwrap();
    f.app
        .commands
        .jj(&["bookmark", "create", "topic/second"])
        .unwrap();
    f.app
        .commands
        .jj(&["rebase", "-s", "@", "-d", "topic/first"])
        .unwrap();
    assert_eq!(
        f.app
            .commands
            .jj(&["log", "-r", "@", "--no-graph", "-T", "conflict"])
            .unwrap(),
        "true"
    );
    f
}

#[test]
fn jj_conflicted_linear_history_stops_before_rewrite_or_publication() {
    let f = conflicted_linear_history();
    let rejected = f
        .app
        .commands
        .jj(&["git", "push", "--bookmark", "topic/second"])
        .unwrap_err();
    assert!(format!("{rejected:#}").contains("since it has conflicts"));
    let history = f.git(&["log", "--all", "--format=%H %B"]);
    f.service.trace.lock().unwrap().clear();
    let error = f.run(&["--no-stack"]).unwrap_err();
    assert!(format!("{error:#}").contains("conflict"), "{error:#}");
    assert_eq!(history, f.git(&["log", "--all", "--format=%H %B"]));
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(!f.trace().iter().any(|entry| matches!(
        entry["args"][0].as_str(),
        Some("push" | "describe" | "reword")
    )));
}

#[test]
fn jj_conflicted_skipped_ancestors_block_resolved_descendants() {
    let f = conflicted_linear_history();
    f.app
        .commands
        .jj(&[
            "describe",
            "-m",
            "wip: conflicted ancestor\n\nRemote-Ref: topic/second",
        ])
        .unwrap();
    f.app
        .commands
        .jj(&[
            "new",
            "@",
            "-m",
            "Resolved descendant\n\nRemote-Ref: topic/resolved",
        ])
        .unwrap();
    f.write("base.txt", "resolved content\n");
    f.app.commands.jj(&["util", "snapshot"]).unwrap();
    assert_eq!(
        f.app
            .commands
            .jj(&["log", "-r", "@", "--no-graph", "-T", "conflict"])
            .unwrap(),
        "false"
    );
    let history = f.git(&["log", "--all", "--format=%H %B"]);
    f.service.trace.lock().unwrap().clear();
    let error = f.run(&["--no-stack", "--skip-draft"]).unwrap_err();
    assert!(format!("{error:#}").contains("unresolved Jujutsu conflicts"));
    assert_eq!(history, f.git(&["log", "--all", "--format=%H %B"]));
    assert!(f.service.prs.lock().unwrap().is_empty());
    assert!(!f.trace().iter().any(|entry| matches!(
        entry["args"][0].as_str(),
        Some("push" | "describe" | "reword")
    )));
}

#[test]
fn jj_conflicted_excluded_descendants_do_not_block_selected_ancestors() {
    let f = conflicted_linear_history();
    f.run(&["--no-stack", "topic/first"]).unwrap();
    let prs = f.service.prs.lock().unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[&101]["head"]["ref"], "topic/first");
    assert_eq!(
        f.git(&["ls-remote", "origin", "refs/heads/topic/second"]),
        ""
    );
}

#[test]
fn jj_batched_revision_resolution_preserves_aliases_and_individual_cardinality() {
    let mut f = Fixture::new(3, false);
    enable_jj(&mut f);
    let escaped = "quote\" slash\\ newline\n return\r tab\t nul\0 β\u{1}\u{85} end";
    let literal = crate::jj::policy::template_literal(escaped);
    assert_eq!(
        f.app
            .commands
            .jj_data(&[
                "--ignore-working-copy",
                "log",
                "-r",
                "root()",
                "--no-graph",
                "-T",
                &literal
            ])
            .unwrap(),
        escaped
    );
    let first = f.git(&["rev-parse", "topic/1"]);
    let third = f.git(&["rev-parse", "topic/3"]);
    let arguments: Vec<_> = (0..100)
        .map(|n| {
            if n % 2 == 0 {
                "topic/3".to_owned()
            } else {
                "topic/1".to_owned()
            }
        })
        .collect();
    f.service.trace.lock().unwrap().clear();
    let resolved = crate::jj::resolve_revisions(&f.app.commands, &arguments).unwrap();
    assert_eq!(
        resolved,
        (0..100)
            .map(|n| if n % 2 == 0 {
                third.clone()
            } else {
                first.clone()
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(f.trace().iter().filter(|v| v["program"] == "jj").count(), 2);
    for arguments in [
        vec!["none()".into(), "topic/1 | topic/3".into()],
        vec!["topic/1 | topic/3".into(), "none()".into()],
        vec!["topic/1".into(), "invalid!(".into()],
    ] {
        assert!(crate::jj::resolve_revisions(&f.app.commands, &arguments).is_err());
    }
    f.git(&["branch", "topic/quoted\"β", "topic/1"]);
    f.app.commands.jj(&["git", "import"]).unwrap();
    let arguments = vec!["\"topic/quoted\\\"β\"".into(), "topic/3".into()];
    assert_eq!(
        crate::jj::resolve_revisions(&f.app.commands, &arguments).unwrap(),
        vec![first, third]
    );
}
