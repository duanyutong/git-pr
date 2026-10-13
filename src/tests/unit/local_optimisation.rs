//! Cache and preparation budgets, with byte-oriented wildcard equivalence.
use super::support::{DRAFT_EVALUATIONS, DRAFT_PREPARATIONS, WORKER_STARTS, World, commit, log};
use crate::stack::selection::{DraftPatterns, match_wildcard};

#[test]
fn cached_submission_needs_no_workers_in_either_stack_mode() {
    for mode in [vec!["--no-stack"], vec!["--github-stack", "--yes"]] {
        for size in [1, 5, 100] {
            let w = World::new(size);
            WORKER_STARTS.set(0);
            w.run(&mode).unwrap();
            assert_eq!(WORKER_STARTS.get(), 0);
            let before = w.0.lock().unwrap().prs.clone();
            w.0.lock().unwrap().trace.clear();
            WORKER_STARTS.set(0);
            w.run(&mode).unwrap();
            assert_eq!(WORKER_STARTS.get(), 0);
            assert_eq!(w.0.lock().unwrap().prs, before);
            assert!(!w.trace().iter().any(|v| {
                v["method"] == "PATCH"
                    || v["method"] == "DELETE"
                    || (v["method"] == "POST" && v["path"] != "graphql")
                    || v["body"]["query"]
                        .as_str()
                        .is_some_and(|q| q.starts_with("mutation"))
            }));
        }
    }
}

#[test]
fn cached_workers_keep_order_and_join_pending_operations_before_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let completed = AtomicUsize::new(0);
    WORKER_STARTS.set(0);
    let values = crate::commands::parallel_cached(
        (0..20).collect(),
        |&n| (n % 2 == 0).then_some(n),
        |n| {
            completed.fetch_add(1, Ordering::SeqCst);
            Ok(n)
        },
    )
    .unwrap();
    assert_eq!(values, (0..20).collect::<Vec<_>>());
    assert_eq!(WORKER_STARTS.get(), 10);
    assert_eq!(completed.load(Ordering::SeqCst), 10);
    completed.store(0, Ordering::SeqCst);
    let result = crate::commands::parallel_cached(
        (0..20).collect(),
        |&n| (n % 2 == 0).then_some(n),
        |n| {
            completed.fetch_add(1, Ordering::SeqCst);
            if n == 1 {
                anyhow::bail!("failed");
            }
            Ok(n)
        },
    );
    assert_eq!(result.unwrap_err().to_string(), "failed");
    assert_eq!(completed.load(Ordering::SeqCst), 4);
}

#[test]
fn descendant_observation_reads_only_beyond_the_frozen_tip() {
    for empty_tip in [false, true] {
        let w = World::new(3);
        let selected = w.0.lock().unwrap().commits.clone();
        if empty_tip {
            w.reply("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature refs/remotes/origin/main..HEAD",0,&log(&selected).replace("\0tree3\0","\0tree2\0"));
        }
        let anchor = commit(3).hash;
        let extra = commit(4);
        w.reply(
            &format!("git for-each-ref --contains {anchor} --format=%(objectname) refs/heads/"),
            0,
            &extra.hash,
        );
        let range = format!("{anchor}..{}", extra.hash);
        w.reply(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature {range}"),0,&log(std::slice::from_ref(&extra)));
        // Ensure a live HEAD change cannot change the descendant-discovery anchor.
        w.reply(&format!("git rev-parse {anchor}"), 0, &anchor);
        w.run(&["--no-stack"]).unwrap();
        let logs = w
            .trace()
            .into_iter()
            .filter(|v| v["program"] == "git" && v["args"][0] == "log")
            .collect::<Vec<_>>();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[1]["args"].as_array().unwrap().last().unwrap(), &range);
        assert!(
            w.0.lock().unwrap().prs[&101]["body"]
                .as_str()
                .unwrap()
                .contains("Feature 4")
        );
        if empty_tip {
            assert!(!w.0.lock().unwrap().prs.contains_key(&103));
        }
    }
}

#[test]
fn draft_preparation_and_title_classification_are_shared_across_rewriting_and_creation() {
    for rewrite in [false, true] {
        let w = World::new(5);
        if rewrite {
            let mut memory = w.0.lock().unwrap();
            memory.branchless = true;
            for c in &mut memory.commits {
                c.attrs.clear();
            }
        }
        DRAFT_PREPARATIONS.set(0);
        DRAFT_EVALUATIONS.set(0);
        w.run(&["--no-stack", "--skip-draft", "--draft=false"])
            .unwrap();
        assert_eq!(DRAFT_PREPARATIONS.get(), 4);
        assert_eq!(DRAFT_EVALUATIONS.get(), 5);
        assert!(
            w.0.lock()
                .unwrap()
                .prs
                .values()
                .all(|p| p["draft"] == false)
        );
    }
}

#[test]
fn compiled_globs_preserve_case_folding_literals_newlines_and_byte_questions() {
    let alphabet = ["a", "b", "*", "?", ".", "β", "\\"];
    let mut patterns = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..3 {
        layer = layer
            .iter()
            .flat_map(|p| alphabet.iter().map(move |c| format!("{p}{c}")))
            .collect();
        patterns.extend(layer.clone());
    }
    patterns.extend(["*[wip]*", "*[draft]*", "WIP:*", "a\nb", "*🦀*", "*İ*"].map(str::to_owned));
    let titles = [
        "",
        "a",
        "ab",
        concat!("b", "a"),
        "AAA",
        "a.b",
        "β",
        "βa",
        "a\nb",
        "🦀",
        "İ",
        "[WiP] work",
        "draft: x",
        "\\a",
        "a\0b",
    ];
    for pattern in &patterns {
        let matcher = DraftPatterns::new(std::slice::from_ref(pattern)).unwrap();
        for title in titles {
            assert_eq!(
                matcher.matches(title),
                match_wildcard(pattern, title),
                "{pattern:?}, {title:?}"
            );
            assert_eq!(
                matcher.clone().matches(title),
                match_wildcard(pattern, title)
            );
        }
    }
    let combined = DraftPatterns::new(&patterns).unwrap();
    for title in titles {
        assert_eq!(
            combined.matches(title),
            patterns.iter().any(|p| match_wildcard(p, title))
        );
    }
    let empty = DraftPatterns::new(&[]).unwrap();
    assert!(!empty.matches("anything"));
    let title = "a".repeat(32768);
    let matcher = DraftPatterns::new(&[format!("*{}b*", "a".repeat(512))]).unwrap();
    super::support::LITERAL_STEPS.set(0);
    assert!(!matcher.matches(&title));
    assert!((title.len()..=2 * title.len()).contains(&super::support::LITERAL_STEPS.get()));
    super::support::LITERAL_STEPS.set(0);
    assert!(!matcher.matches(&title));
    assert_eq!(super::support::LITERAL_STEPS.get(), 0);
}

#[test]
fn excessive_draft_patterns_fail_with_clear_guidance_before_effects() {
    let w = World::new(1);
    w.0.lock()
        .unwrap()
        .config
        .insert("git-pr.draftPattern".into(), "?".repeat(1_000_000));
    let error = w.run(&["--stop-after=validate"]).unwrap_err();
    assert!(format!("{error:#}").contains("draft patterns exceed matcher resource limits"));
    assert!(format!("{error:#}").contains("git-pr.draftPattern"));
    assert!(
        !w.trace()
            .iter()
            .any(|v| v["kind"] == "http" || v["args"][0] == "push")
    );
}

#[test]
fn literal_sections_do_not_overlap_and_backtracking_preserves_borders() {
    for pattern in [
        "aaa*aaa",
        "*aaab*aaac*",
        "*ababaca*",
        "a**b***c",
        "*a*a*",
        "*🦀*β*",
        "*aaaaab*",
    ] {
        let matcher = DraftPatterns::new(&[pattern.into()]).unwrap();
        for title in [
            "aaaaa",
            "aaaaaa",
            "aaaaaaabaaaaaaac",
            "ababababacax",
            "ac",
            "abc",
            "aa",
            "🦀β",
            "β🦀",
            "aaaaaaaaac",
        ] {
            assert_eq!(
                matcher.matches(title),
                match_wildcard(pattern, title),
                "{pattern:?}, {title:?}"
            );
        }
    }
    let error = DraftPatterns::new(&["a".repeat(1024 * 1024 + 1)]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("draft patterns exceed matcher resource limits")
    );
}

#[test]
fn described_jujutsu_working_copy_is_the_observed_tip_without_a_second_history_read() {
    let w = World::new(2);
    w.0.lock().unwrap().jj = true;
    let working = commit(3);
    w.reply(r#"jj --ignore-working-copy log -r @ --no-graph -T conflict ++ "\0" ++ if(empty, "EMPTY", "NONEMPTY") ++ "|" ++ if(description, "HAS-DESC", "NO-DESC") ++ "\0" ++ author.name() ++ "\0" ++ author.email() ++ "\0" ++ change_id ++ "|" ++ commit_id ++ "|" ++ description"#,0,&format!("false\0NONEMPTY|HAS-DESC\0Unit Author\0unit@example.com\0{}|{}|{}",working.change_id,working.hash,working.full_message()));
    for hash in [commit(2).hash, working.hash.clone()] {
        w.reply(
            &format!("git for-each-ref --contains {hash} --format=%(objectname) refs/heads/"),
            0,
            &working.hash,
        );
    }
    w.run(&["--no-stack"]).unwrap();
    assert_eq!(w.0.lock().unwrap().prs.len(), 3);
    assert_eq!(
        w.trace()
            .iter()
            .filter(|v| v["program"] == "git" && v["args"][0] == "log")
            .count(),
        1
    );
}

#[test]
fn discovery_reuses_base_and_tip_identities_in_both_native_modes() {
    for mode in [vec!["--no-stack"], vec!["--github-stack", "--yes"]] {
        let w = World::new(5);
        w.fail("git rev-parse --verify refs/remotes/origin/main^{commit}");
        w.fail(&format!("git rev-parse {}", commit(5).hash));
        w.run(&mode).unwrap();
        let base_reads = w
            .trace()
            .iter()
            .filter(|v| {
                v["program"] == "git"
                    && v["args"]
                        == serde_json::json!([
                            "show",
                            "--no-show-signature",
                            "-s",
                            "--format=%H %T",
                            "refs/remotes/origin/main"
                        ])
            })
            .count();
        assert_eq!(base_reads, 1);
        assert_eq!(w.0.lock().unwrap().prs.len(), 5);
    }
}

#[test]
fn descendant_description_reads_require_no_jujutsu_recovery_queries() {
    for compact in [false, true] {
        let w = World::new(3);
        w.0.lock().unwrap().jj = true;
        let extra = commit(4);
        let anchor = commit(3).hash;
        w.reply(
            &format!("git for-each-ref --contains {anchor} --format=%(objectname) refs/heads/"),
            0,
            &extra.hash,
        );
        let range = format!("{anchor}..{}", extra.hash);
        w.reply(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature {range}"),0,&log(std::slice::from_ref(&extra)));
        w.fail(&format!("jj --at-operation operation log -r {} --no-graph -T commit_id ++ \" \" ++ change_id ++ \" \" ++ conflict ++ \"\\n\"", extra.hash));
        w.run(&[
            "--no-stack",
            if compact {
                "--compact-stack"
            } else {
                "--compact-stack=false"
            },
        ])
        .unwrap();
        let trace = w.trace();
        assert_eq!(
            trace
                .iter()
                .filter(|v| v["program"] == "jj" && v["args"][1] == "op")
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|v| v["program"] == "jj" && v["args"][0] == "--at-operation")
                .count(),
            1
        );
        if !compact {
            assert!(
                w.0.lock().unwrap().prs[&101]["body"]
                    .as_str()
                    .unwrap()
                    .contains("Feature 4")
            );
        }
    }
}

#[test]
fn malformed_base_observations_stop_before_publication_with_clear_guidance() {
    for value in ["", "missing-tree", " tree", "hash ", "hash tree extra"] {
        let w = World::new(1);
        w.reply(
            "git show --no-show-signature -s --format=%H %T refs/remotes/origin/main",
            0,
            value,
        );
        let error = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
        assert!(
            error.contains("invalid base commit/tree observation for refs/remotes/origin/main")
        );
        assert!(
            !w.trace()
                .iter()
                .any(|v| v["kind"] == "http" || v["args"][0] == "push")
        );
    }
}

#[test]
fn invalid_descendant_content_stops_description_updates() {
    let w = World::new(3);
    let anchor = commit(3).hash;
    let mut extra = commit(4);
    extra.title.clear();
    extra.message.clear();
    extra.attrs.clear();
    w.reply(
        &format!("git for-each-ref --contains {anchor} --format=%(objectname) refs/heads/"),
        0,
        &extra.hash,
    );
    let range = format!("{anchor}..{}", extra.hash);
    w.reply(&format!("git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature {range}"),0,&log(std::slice::from_ref(&extra)));
    let diagnostic = format!("{:#}", w.run(&["--no-stack"]).unwrap_err());
    assert!(diagnostic.contains("validation failed, please fix the commits above"));
    let memory = w.0.lock().unwrap();
    assert!(
        memory
            .stdout
            .contains("empty title but contains file changes")
    );
    assert!(
        memory
            .prs
            .values()
            .all(|pr| pr["body"] == "Body 1" || pr["body"] == "Body 2" || pr["body"] == "Body 3")
    );
    assert!(!memory.trace.iter().any(|v| {
        v["body"]["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("mutation"))
    }));
    drop(memory);
}

#[test]
fn publication_remote_selection_respects_git_precedence_without_extra_config_queries() {
    let remotes = "a-backup https://github.com/org/backup (push)\norigin https://github.com/org/repo (push)\nupstream https://github.com/org/upstream (push)";
    for (branch, settings, expected) in [
        ("Topic", vec![], "origin"),
        ("Topic", vec![("remote.pushDefault", "origin")], "origin"),
        (
            "Topic",
            vec![("branch.Topic.remote", "upstream")],
            "upstream",
        ),
        (
            "Topic",
            vec![
                ("branch.Topic.remote", "upstream"),
                ("remote.pushDefault", "origin"),
            ],
            "origin",
        ),
        (
            "Topic",
            vec![
                ("branch.Topic.remote", "upstream"),
                ("remote.pushDefault", "origin"),
                ("branch.Topic.pushRemote", "a-backup"),
            ],
            "a-backup",
        ),
        (
            "Topic",
            vec![("branch.topic.pushRemote", "a-backup")],
            "origin",
        ),
        (
            "",
            vec![
                ("branch.Topic.pushRemote", "a-backup"),
                ("remote.pushDefault", "origin"),
            ],
            "origin",
        ),
    ] {
        let w = World::new(1);
        w.reply("git remote -v", 0, remotes);
        w.0.lock().unwrap().branch = branch.into();
        for (key, value) in settings {
            w.0.lock().unwrap().config.insert(key.into(), value.into());
        }
        let config = w.config();
        assert_eq!(config.repo.remote, expected);
        assert_eq!(config.repo.trunk, "main");
        assert_eq!(
            config.repo.name,
            format!(
                "org/{}",
                if expected == "origin" {
                    "repo"
                } else if expected == "a-backup" {
                    "backup"
                } else {
                    "upstream"
                }
            )
        );
        assert_eq!(
            w.trace()
                .iter()
                .filter(|v| v["program"] == "git" && v["args"][0] == "config")
                .count(),
            3
        );
    }
}

#[test]
fn unsafe_remote_destinations_stop_before_publication() {
    for (remotes, setting, expected) in [
        (
            "backup https://github.com/org/backup (push)\nupstream https://github.com/org/upstream (push)",
            None,
            "no unambiguous publication remote",
        ),
        ("", None, "no unambiguous publication remote"),
        (
            "origin https://github.com/org/repo (push)",
            Some("missing"),
            "not a named remote",
        ),
        (
            "origin https://github.com/org/repo (push)",
            Some("."),
            "not a named remote",
        ),
        (
            "origin https://github.com/org/repo (push)\norigin https://github.com/org/backup (push)",
            None,
            "multiple push URLs",
        ),
        (
            "origin https://github.com/org/repo (push)\norigin https://github.com/org/repo (push)",
            None,
            "multiple push URLs",
        ),
    ] {
        let w = World::new(1);
        w.reply("git remote -v", 0, remotes);
        if let Some(value) = setting {
            w.0.lock()
                .unwrap()
                .config
                .insert("remote.pushDefault".into(), value.into());
        }
        assert!(format!("{:#}", w.run(&[]).unwrap_err()).contains(expected));
        assert!(
            !w.trace()
                .iter()
                .any(|v| v["args"][0] == "push" || v["method"].is_string())
        );
    }
    let only = "publishing https://github.com/org/repo (fetch)";
    assert_eq!(crate::git::policy::select_remote(only, None).unwrap(), only);
}

#[test]
fn explicit_jujutsu_arguments_are_batched_with_order_and_aliases_preserved() {
    use crate::jj;
    assert_eq!(
        jj::resolve_revisions(&World::new(0).commands(), &[]).unwrap(),
        Vec::<String>::new()
    );
    for size in [2, 100, 1000] {
        let w = World::new(size);
        let revisions: Vec<_> = (1..=size).rev().map(|n| format!("topic/{n}")).collect();
        let hashes = jj::resolve_revisions(&w.commands(), &revisions).unwrap();
        assert_eq!(
            hashes,
            (1..=size).rev().map(|n| commit(n).hash).collect::<Vec<_>>()
        );
        assert_eq!(
            w.trace().len(),
            1 + usize::try_from(size).unwrap().div_ceil(128)
        );
    }
    let w = World::new(2);
    let revisions = vec![commit(2).hash, "change1".into(), "topic/2".into()];
    assert_eq!(
        jj::resolve_revisions(&w.commands(), &revisions).unwrap(),
        vec![commit(2).hash, commit(1).hash, commit(2).hash]
    );
    w.0.lock().unwrap().jj = true;
    w.run(&["--no-stack", "topic/2", "change1", "topic/2"])
        .unwrap();
    assert_eq!(w.0.lock().unwrap().prs.len(), 2);
}

#[test]
fn jujutsu_batch_rejects_incomplete_or_ambiguous_argument_membership() {
    use crate::jj;
    let w = World::new(2);
    let revisions = vec!["topic/1".into(), "topic/2".into()];
    jj::resolve_revisions(&w.commands(), &revisions).unwrap();
    let trace = w.trace();
    let args = trace.last().unwrap()["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    let command = format!("jj {args}");
    for output in [
        "invalid".into(),
        format!("{}\0\x30", commit(1).hash),
        format!("{}\0,\n", commit(1).hash),
        "hash\0\x30,\n".into(),
        format!("{}\0invalid,\n", commit(1).hash),
        format!("{}\0\x32,\n", commit(1).hash),
        format!("{}\0\x30,\n{}\0\x30,\n", commit(1).hash, commit(2).hash),
        format!("{}\0\x30,\n", commit(1).hash),
    ] {
        w.reply(&command, 0, &output);
        assert!(
            jj::resolve_revisions(&w.commands(), &revisions).is_err(),
            "{output:?}"
        );
    }
    assert!(jj::resolve_revisions(&w.commands(), &["x".repeat(65_536)]).is_err());
    w.fail("jj --ignore-working-copy op log --limit 1 --no-graph -T id");
    assert!(jj::resolve_revisions(&w.commands(), &revisions).is_err());
}

#[test]
fn unchanged_large_stack_footers_borrow_rows_and_write_no_description_bytes() {
    use super::support::DESCRIPTION_BYTES_WRITTEN;
    use crate::stack::description::{Body, StackRenderer, Style};
    use std::collections::HashSet;
    let stack: Vec<_> = (1..=1000)
        .map(|n| {
            let mut c = commit(n);
            c.pr_number = n;
            c
        })
        .collect();
    for stack_up in [false, true] {
        let renderer = StackRenderer::new(
            stack.iter(),
            &[],
            &HashSet::new(),
            Style {
                stack_up,
                emoji: true,
                host: "github.com",
                repo: "org/repo",
            },
        );
        for current in &stack {
            let footer = renderer.footer(current);
            let parts = footer.parts();
            assert!(parts[0].len() + parts[2].len() < 256);
            assert!(parts[1].len() + parts[3].len() > 7000);
            let body = format!(
                "Human text β\n\n{}\n{}\n{}",
                crate::stack::description::START,
                parts.concat(),
                crate::stack::description::END
            );
            DESCRIPTION_BYTES_WRITTEN.set(0);
            assert!(
                Body::parse(&body)
                    .unwrap()
                    .render_footer("", &footer, "")
                    .unwrap()
                    .is_none()
            );
            assert_eq!(DESCRIPTION_BYTES_WRITTEN.get(), 0);
        }
    }
}

#[test]
fn repeated_submission_materialises_no_unchanged_descriptions_in_either_stack_mode() {
    use super::support::{BODY_ANALYSES, CONTENT_ENCODINGS, DESCRIPTION_BYTES_WRITTEN};
    for mode in [vec!["--no-stack"], vec!["--github-stack", "--yes"]] {
        let w = World::new(100);
        w.run(&mode).unwrap();
        let bodies: Vec<_> = {
            let mut memory = w.0.lock().unwrap();
            for pr in memory.prs.values_mut() {
                pr["body"] = serde_json::Value::String(format!(
                    "{}\n\n{}",
                    "Human β text. ".repeat(5000),
                    pr["body"].as_str().unwrap()
                ));
            }
            memory.prs.values().map(|pr| pr["body"].clone()).collect()
        };
        BODY_ANALYSES.set(0);
        CONTENT_ENCODINGS.set(0);
        DESCRIPTION_BYTES_WRITTEN.set(0);
        w.run(&mode).unwrap();
        assert_eq!(BODY_ANALYSES.get(), 100);
        assert_eq!(CONTENT_ENCODINGS.get(), 0);
        assert_eq!(DESCRIPTION_BYTES_WRITTEN.get(), 0);
        assert_eq!(
            w.0.lock()
                .unwrap()
                .prs
                .values()
                .map(|pr| pr["body"].clone())
                .collect::<Vec<_>>(),
            bodies
        );
    }
}

#[test]
fn jujutsu_resolution_byte_budget_and_hash_widths_are_enforced() {
    use crate::jj::policy;
    let arguments = vec!["x".repeat(20_000), "y".repeat(20_000)];
    let (revset, template, count) = policy::revision_query(&arguments).unwrap();
    assert_eq!(count, 1);
    assert!(revset.len() + template.len() < 65_536);
    let hash = "a".repeat(64);
    assert_eq!(
        policy::parse_resolutions(&format!("{hash}\0\x30,\n"), &["arg".into()]).unwrap(),
        vec![hash]
    );
    assert!(
        policy::parse_resolutions(&format!("{}\0\x30,\n", "z".repeat(40)), &["arg".into()])
            .is_err()
    );
}

#[test]
fn branch_destination_discovery_and_batch_execution_failures_propagate() {
    let w = World::new(1);
    w.0.lock()
        .unwrap()
        .config
        .insert("branch.topic/1.remote".into(), "origin".into());
    w.fail("git branch --show-current");
    assert!(
        format!("{:#}", w.run(&[]).unwrap_err())
            .contains("current Git branch for publication remote selection")
    );
    let w = World::new(2);
    let arguments = vec!["topic/1".into(), "topic/2".into()];
    crate::jj::resolve_revisions(&w.commands(), &arguments).unwrap();
    let trace = w.trace();
    let args = trace.last().unwrap()["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    w.fail(&format!("jj {args}"));
    assert!(crate::jj::resolve_revisions(&w.commands(), &arguments).is_err());
}

#[test]
fn jujutsu_template_literals_preserve_its_escape_dialect_and_control_characters() {
    let text = "quote\" slash\\ newline\n return\r tab\t nul\0 β\u{1}\u{85}";
    assert_eq!(
        crate::jj::policy::template_literal(text),
        "\"quote\\\" slash\\\\ newline\\n return\\r tab\\t nul\\0 β\u{1}\u{85}\""
    );
}

#[test]
fn shared_history_preserves_duplicate_precedence_and_each_body_order() {
    use crate::stack::description::{Body, END, HistoryEntry, START};
    use std::collections::HashSet;
    for managed in [false, true] {
        for stack_up in [false, true] {
            let order = if stack_up { "newest" } else { "oldest" };
            let footer = format!(
                "This is PR **1 of 5** in a stack ({order} at the top)\n\n* #3\n* ✔️ #1\n* ✔️ #3\n* #2\n* #4"
            );
            let text = if managed {
                format!("Human text\n{START}\n{footer}\n{END}")
            } else {
                format!("Human text\n---\n{footer}")
            };
            let mut history = vec![HistoryEntry {
                number: 2,
                merged: true,
            }];
            let mut seen = HashSet::from([2]);
            Body::parse(&text)
                .unwrap()
                .append_history(&mut history, &mut seen);
            let numbers = if stack_up {
                vec![2, 4, 1, 3]
            } else {
                vec![2, 3, 1, 4]
            };
            assert_eq!(
                history.iter().map(|entry| entry.number).collect::<Vec<_>>(),
                numbers
            );
            assert!(
                history
                    .iter()
                    .find(|entry| entry.number == 2)
                    .unwrap()
                    .merged
            );
            assert!(
                !history
                    .iter()
                    .find(|entry| entry.number == 3)
                    .unwrap()
                    .merged
            );
            assert!(
                history
                    .iter()
                    .find(|entry| entry.number == 1)
                    .unwrap()
                    .merged
            );
            let second = Body::parse(&format!("{START}\n* #1\n* ✔️ #5\n{END}")).unwrap();
            second.append_history(&mut history, &mut seen);
            assert_eq!(
                history.last().unwrap(),
                &HistoryEntry {
                    number: 5,
                    merged: true
                }
            );
            second.append_history(&mut history, &mut seen);
            assert_eq!(history.len(), 5);
        }
    }
}

#[test]
fn a_thousand_complete_histories_retain_only_one_list_of_unique_entries() {
    use crate::stack::description::{BodyCache, END, START};
    use std::fmt::Write as _;
    use std::{collections::HashSet, sync::Arc};
    let mut rows = String::new();
    for number in 1..=1000 {
        writeln!(rows, "* #{number}").unwrap();
    }
    let source: Arc<str> = format!("{START}\n{rows}{END}").into();
    let mut bodies = BodyCache::default();
    let mut history = Vec::new();
    let mut seen = HashSet::new();
    for number in 1..=1000 {
        bodies
            .get_shared(number, &source)
            .unwrap()
            .append_history(&mut history, &mut seen);
    }
    assert_eq!(history.len(), 1000);
    assert_eq!(seen.len(), 1000);
    assert!(history.capacity() <= 2048);
    assert_eq!(
        history.iter().map(|entry| entry.number).collect::<Vec<_>>(),
        (1..=1000).collect::<Vec<_>>()
    );
}

#[test]
fn renderers_accept_borrowed_commit_context_across_separate_owned_stacks() {
    use crate::stack::description::{CompactRenderer, StackRenderer, Style};
    use std::collections::HashSet;
    let mut submitted = [commit(1), commit(2)];
    let mut descendants = [commit(3)];
    for (index, c) in submitted.iter_mut().chain(&mut descendants).enumerate() {
        c.pr_number = 101 + u64::try_from(index).unwrap();
        c.message = "long commit message ".repeat(3000);
    }
    let context = submitted.iter().chain(&descendants);
    let renderer = CompactRenderer::new(context.clone());
    assert!(renderer.render(&submitted[1]).contains("#103 — Next PR"));
    let renderer = StackRenderer::new(
        context,
        &[],
        &HashSet::new(),
        Style {
            stack_up: false,
            emoji: false,
            host: "github.com",
            repo: "org/repo",
        },
    );
    assert!(
        renderer
            .footer(&submitted[1])
            .parts()
            .concat()
            .contains("* #103")
    );
    assert!(
        submitted
            .iter()
            .chain(&descendants)
            .all(|c| c.message.len() == 60_000)
    );
}
