use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use crate::{
    cli::output,
    cli::{Flags, ParseError},
    config::settings,
    model::trailers,
    model::{self, Commit},
    stack::description::{self, HistoryEntry, Style},
    stack::selection,
};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}
fn flags(values: &[&str]) -> Flags {
    Flags::parse(strings(values)).unwrap()
}
fn commit(number: u64) -> Commit {
    Commit {
        hash: format!("{number:040x}"),
        title: format!("Feature {number}"),
        pr_number: number,
        author_name: "Test Author".into(),
        author_email: "test@example.com".into(),
        attrs: vec![("remote-ref".into(), format!("test/{number}"))],
        ..Commit::default()
    }
}

#[test]
fn cli_preserves_go_boundaries_and_boolean_values() {
    for value in ["1", "t", "T", "TRUE", "true", "True"] {
        assert!(flags(&[&format!("--dry-run={value}")]).boolean("dry-run"));
    }
    for value in ["0", "f", "F", "FALSE", "false", "False"] {
        assert!(!flags(&[&format!("-draft={value}")]).boolean("draft"));
    }
    assert_eq!(
        flags(&["a", "--draft=false"]).args,
        strings(&["a", "--draft=false"])
    );
    assert_eq!(flags(&["--", "-draft"]).args, strings(&["-draft"]));
    assert_eq!(flags(&["-"]).args, strings(&["-"]));
    assert!(flags(&[]).boolean("stack-up"));
    assert!(flags(&[]).boolean("draft"));
    assert!(!flags(&[]).boolean("github-stack"));
    assert!(!flags(&["-y", "-yes=false"]).boolean("yes"));
    let parsed = flags(&[
        "--add-label",
        "a,b",
        "-add-label=two words",
        "--add-tip-label=tip",
    ]);
    assert_eq!(parsed.labels, strings(&["a,b", "two words"]));
    assert_eq!(parsed.tip_labels, strings(&["tip"]));
    assert!(matches!(
        Flags::parse(strings(&["--help"])),
        Err(ParseError::Help)
    ));
    for args in [
        &["---draft"][..],
        &["-=x"],
        &["--unknown"],
        &["-draft=YES"],
        &["--timeout"],
        &["--timeout=invalid"],
    ] {
        assert!(
            matches!(Flags::parse(strings(args)), Err(ParseError::Invalid(_))),
            "{args:?}"
        );
    }
    for (value, want) in [
        ("20", 20),
        ("-1", -1),
        ("0x10", 16),
        ("0X10", 16),
        ("0b11", 3),
        ("0B11", 3),
        ("0o10", 8),
        ("0O10", 8),
        ("010", 8),
        ("+20", 20),
        ("1_000", 1000),
    ] {
        assert_eq!(crate::cli::parse_integer(value).ok(), Some(want));
    }
    assert_eq!(crate::cli::parse_integer("9223372036854775808").ok(), None);
}

#[test]
fn configuration_precedence_and_present_empty_lists() {
    for no_stack in [false, true] {
        for cli in [None, Some(false), Some(true)] {
            for env in [
                None,
                Some("1"),
                Some("true"),
                Some("TRUE"),
                Some("0"),
                Some("invalid"),
                Some(""),
            ] {
                for git in [None, Some(false), Some(true)] {
                    let expected = !no_stack
                        && cli.unwrap_or_else(|| {
                            env.map_or_else(
                                || git.unwrap_or(false),
                                |v| ["1", "true", "TRUE"].contains(&v),
                            )
                        });
                    assert_eq!(settings::github_stack(no_stack, cli, env, git), expected);
                }
            }
        }
    }
    let env = BTreeMap::from([
        ("GIT_PR_DRY_RUN".into(), "1".into()),
        ("GIT_PR_OUTPUT".into(), "markdown".into()),
    ]);
    assert!(settings::enabled(
        &flags(&["--dry-run=false"]),
        &env,
        "dry-run",
        "GIT_PR_DRY_RUN"
    ));
    assert!(!settings::enabled(&flags(&[]), &env, "yes", "GIT_PR_YES"));
    assert_eq!(settings::fallback("", &env, "GIT_PR_OUTPUT"), "markdown");
    assert_eq!(settings::fallback("url", &env, "GIT_PR_OUTPUT"), "url");
    assert_eq!(settings::fallback("", &env, "ABSENT"), "");
    assert_eq!(
        settings::labels(&strings(&["cli"]), "git"),
        strings(&["cli"])
    );
    assert_eq!(
        settings::labels(&[], " one, two "),
        strings(&["one", "two"])
    );
    assert_eq!(settings::labels(&[], ""), [] as [String; 0]);
    assert_eq!(
        model::dedup(["a", "", "b", "a", "c", "b"]),
        strings(&["a", "b", "c"])
    );
    assert_eq!(model::comma_list(" a, , b,,a "), strings(&["a", "b", "a"]));
}

#[test]
fn trailers_round_trip_and_body_separation() {
    for (body, message, count) in [
        ("", "", 0),
        ("  \n \n", "", 0),
        ("Body\nKey: value", "Body\nKey: value", 0),
        ("Body\n\nKey: value", "Body", 1),
        ("Key: value", "", 1),
        ("Key: x", "", 1),
        ("Tags: x", "", 1),
        ("Tags: é", "", 1),
        ("Tags: \t", "Tags:", 0),
        ("Remote-Ref: a", "", 1),
        ("remote-ref: é", "", 1),
        ("Remote-Ref:", "Remote-Ref:", 0),
        ("Body\n\nBad_Key: value", "Body\n\nBad_Key: value", 0),
        ("\n\nTags: one, two\nRemote-Ref: user/topic\n\n", "", 2),
    ] {
        let (actual, attrs) = trailers::parse(&body.split('\n').collect::<Vec<_>>());
        assert_eq!(actual, message, "{body:?}");
        assert_eq!(attrs.len(), count, "{body:?}");
    }
    let attrs = vec![
        ("tags".into(), "one, two".into()),
        ("remote-ref".into(), "user/topic".into()),
        ("custom-key".into(), "value".into()),
    ];
    let message = trailers::serialise("Title", "A body", &attrs);
    assert_eq!(
        message,
        "Title\n\nA body\n\nCustom-Key: value\n      Tags: one, two\nRemote-Ref: user/topic"
    );
    let (body, parsed) = trailers::parse(&message.split('\n').skip(1).collect::<Vec<_>>());
    assert_eq!(body, "A body");
    assert_eq!(parsed[0].0, "remote-ref");
    assert_eq!(trailers::format_key("--CUSTOM--Key-"), "--Custom--Key-");
    assert_eq!(trailers::serialise("Title", "", &[]), "Title");
    for reference in ["a", "é"] {
        let attrs = vec![("remote-ref".into(), reference.into())];
        let text = trailers::serialise("Title", "Body", &attrs);
        assert_eq!(
            trailers::parse(&text.lines().skip(1).collect::<Vec<_>>()),
            ("Body".into(), attrs)
        );
    }
    for key in ["tags", "custom-key"] {
        for value in ["x", "é"] {
            for footer in [
                format!("Remote-Ref: published-topic\n{key}: {value}"),
                format!("{key}: {value}\nRemote-Ref: published-topic"),
            ] {
                let (body, attrs) = trailers::parse(&footer.lines().collect::<Vec<_>>());
                assert_eq!(body, "");
                let commit = Commit {
                    attrs,
                    ..Commit::default()
                };
                assert_eq!(commit.remote_ref(), "published-topic");
                assert_eq!(commit.attr(key), value);
                let text = trailers::serialise("Title", "Body", &commit.attrs);
                let (body, mut parsed) = trailers::parse(&text.lines().skip(1).collect::<Vec<_>>());
                let mut expected = commit.attrs;
                parsed.sort();
                expected.sort();
                assert_eq!(body, "Body");
                assert_eq!(parsed, expected);
            }
        }
    }
    let mut cm = commit(1);
    cm.set_attr("tags", "one, two");
    cm.set_attr("tags", "two,three");
    assert_eq!(
        cm.tags(&strings(&[" one ", "two", ""])),
        strings(&["one", "two", "three"])
    );
    assert!(cm.to_string().contains("(test/1)"));
    assert_eq!(
        Commit {
            title: "Title".into(),
            hash: "short".into(),
            ..Commit::default()
        }
        .to_string(),
        "short  Title"
    );
}

#[test]
fn log_parsing_dates_headers_and_multiline_messages() {
    let hash = "abc123def456789012345678901234567890abcd";
    for date in [
        "Mon Jan 1 00:00:00 2024 +0000",
        "Fri Nov 30 18:30:16 2025 -0300",
        "2024-01-01 12:34:56 +0000",
    ] {
        let log = format!(
            "commit {hash}\nAuthor: Test User <test@example.com>\nDate:   {date}\n\n    Title 🛠️\n\n    Body\n    continued\n\n    Remote-Ref: test/topic\n"
        );
        let parsed = crate::tests::legacy::parse_logs(&log).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].hash, hash);
        assert_eq!(parsed[0].title, "Title 🛠️");
        assert_eq!(parsed[0].message, "Body\ncontinued");
        assert_eq!(parsed[0].remote_ref(), "test/topic");
        assert!(parsed[0].date.is_some());
        assert_eq!(
            crate::tests::legacy::parse_logs(&format!("{log}\n{log}"))
                .unwrap()
                .len(),
            2
        );
    }
    assert_eq!(
        crate::tests::legacy::parse_logs(" \n \n").unwrap(),
        [] as [model::Commit; 0]
    );
    for log in [
        "bad",
        "commit 123\nAuthor: X <a@b>\n",
        "Author: X <a@b>\n",
        "commit abc123def456789012345678901234567890abcd\nAuthor: X <a@b>\nDate: invalid",
    ] {
        assert!(crate::tests::legacy::parse_logs(log).is_err());
    }
    let empty = crate::tests::legacy::parse_logs(&format!(
        "commit {hash}\nAuthor: Test <test@example.com>\n"
    ))
    .unwrap();
    assert_eq!(empty[0].title, "");
}

#[test]
fn output_presets_custom_templates_and_validation() {
    let cm = Commit {
        hash: "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".into(),
        title: "Fix the parser".into(),
        author_name: "Oliver N".into(),
        author_email: "oliver@example.com".into(),
        pr_number: 1234,
        attrs: vec![("remote-ref".into(), "oliver/a1b2c3d4".into())],
        ..Commit::default()
    };
    for (spec, want) in [
        ("", "https://github.com/org/repo/pull/1234 Fix the parser"),
        ("url", "https://github.com/org/repo/pull/1234"),
        (
            "url-title",
            "https://github.com/org/repo/pull/1234 Fix the parser",
        ),
        (
            "  markdown  ",
            "[#1234](https://github.com/org/repo/pull/1234) Fix the parser",
        ),
        (
            "{shorthash} #{number} {title}",
            "a1b2c3d4 #1234 Fix the parser",
        ),
        ("{repo}#{number} on {host}", "org/repo#1234 on github.com"),
        (
            "{branch} by {author} <{email}>",
            "oliver/a1b2c3d4 by Oliver N <oliver@example.com>",
        ),
        ("{hash}", "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"),
        ("{number} {number}", "1234 1234"),
        ("λ {title} λ", "λ Fix the parser λ"),
        ("{}", "{}"),
        ("{ } {title} {", "{ } Fix the parser {"),
        ("- [ ] {url}", "- [ ] https://github.com/org/repo/pull/1234"),
    ] {
        assert_eq!(
            crate::tests::legacy::format_pr(
                &output::resolve_format(spec).unwrap(),
                &cm,
                "github.com",
                "org/repo"
            ),
            want
        );
    }
    let error = output::resolve_format("{url} {nope}")
        .unwrap_err()
        .to_string();
    assert!(error.contains("{nope}"));
    assert!(error.contains("{title}"));
    let error = output::resolve_format("bogus").unwrap_err().to_string();
    for expected in ["bogus", "markdown", "url-title"] {
        assert!(error.contains(expected));
    }
    let cm = Commit {
        title: "{number}".into(),
        ..cm
    };
    assert_eq!(
        crate::tests::legacy::format_pr("{title}", &cm, "github.com", "org/repo"),
        "{number}"
    );
}

#[test]
fn selection_ranges_skips_and_reference_policy() {
    let mut stack: Vec<_> = (1..=5).map(commit).collect();
    for (numbers, want) in [
        (vec![2, 3, 4], (1, 3)),
        (vec![5, 1, 3], (0, 4)),
        (vec![3], (2, 2)),
        (vec![2, 2], (1, 1)),
    ] {
        assert_eq!(
            selection::order_selection(
                &stack,
                &numbers
                    .into_iter()
                    .map(|n| format!("{n:040x}"))
                    .collect::<Vec<_>>()
            )
            .unwrap(),
            want
        );
    }
    assert!(selection::order_selection(&stack, &[]).is_err());
    assert!(selection::order_selection(&stack, &strings(&["missing"])).is_err());
    assert_eq!(
        selection::slice_start(&stack, "trunk", "trunk", &HashMap::new()),
        0
    );
    assert_eq!(
        selection::slice_start(&stack, &stack[1].hash, "trunk", &HashMap::new()),
        2
    );
    assert_eq!(
        selection::slice_start(&stack, "missing", "trunk", &HashMap::new()),
        5
    );
    assert_eq!(
        crate::tests::legacy::resolve_base(&stack, 0, 0, "main"),
        "main"
    );
    assert_eq!(
        crate::tests::legacy::resolve_base(&stack, 1, 1, "main"),
        "test/1"
    );
    stack[0].attrs.clear();
    assert_eq!(
        crate::tests::legacy::resolve_base(&stack, 1, 1, "main"),
        "main"
    );
    stack[2].skip = true;
    assert_eq!(
        crate::tests::legacy::resolve_base(&stack, 0, 3, "main"),
        "test/2"
    );
    let selected = vec![stack[0].hash.clone(), stack[4].hash.clone()];
    assert_eq!(selection::mark_unselected(&mut stack, &selected).len(), 2);
    assert_eq!(
        stack.iter().map(|c| c.skip).collect::<Vec<_>>(),
        vec![false, true, true, true, false]
    );
    for (local, pushed, ff, same, want) in [
        ("a", "a", false, false, selection::RefSync::Nothing),
        ("a", "b", true, false, selection::RefSync::Move),
        ("a", "b", false, true, selection::RefSync::Move),
        ("a", "b", false, false, selection::RefSync::Unrelated),
    ] {
        assert_eq!(selection::decide_ref_sync(local, pushed, ff, same), want);
    }
    assert!(selection::remote_ref_matches(
        "abc\trefs/heads/topic\n",
        "refs/heads/topic",
        "abc"
    ));
    for out in [
        "abc refs/heads/topic extra",
        "abcd refs/heads/topic",
        "abc refs/heads/topic-other",
        "",
    ] {
        assert!(!selection::remote_ref_matches(
            out,
            "refs/heads/topic",
            "abc"
        ));
    }
    let tips = strings(&["a", "b", "c"]);
    assert_eq!(
        selection::pick_stack_leaf(&tips, &|batch| Ok(vec![
            batch.iter().max().unwrap().clone()
        ]))
        .unwrap(),
        Some("c".into())
    );
    assert_eq!(
        selection::pick_stack_leaf(&tips, &|batch| Ok(batch.to_vec())).unwrap(),
        None
    );
    assert_eq!(
        selection::pick_stack_leaf(&[], &|_| unreachable!()).unwrap(),
        None
    );
    assert_eq!(
        selection::local_branch(
            "(HEAD detached at abc)|hash\nmain|hash\nmaster|hash\nbad\nbranch|hash|more\nfirst|hash\nsecond|hash",
            "hash"
        ),
        Some("first".into())
    );
    assert_eq!(selection::local_branch("x|else", "hash"), None);
    assert_eq!(
        selection::local_branch("topic|one|hash", "hash"),
        Some("topic|one".into())
    );
}

#[test]
fn stack_history_rendering_order_markers_and_duplicates() {
    let stack: Vec<_> = (101..=103).map(commit).collect();
    let history = [
        HistoryEntry {
            number: 99,
            merged: true,
        },
        HistoryEntry {
            number: 100,
            merged: false,
        },
        HistoryEntry {
            number: 101,
            merged: true,
        },
        HistoryEntry {
            number: 99,
            merged: false,
        },
    ];
    for stack_up in [false, true] {
        for emoji in [false, true] {
            let style = Style {
                stack_up,
                emoji,
                host: "github.com",
                repo: "org/repo",
            };
            let info = crate::tests::legacy::generate_stack(
                &stack,
                &stack[1],
                &history,
                &HashSet::from([101]),
                style,
            );
            assert!(info.contains("PR **4 of 5**"));
            assert!(info.contains("#102 👈 This PR"));
            assert_eq!(info.matches("#99").count(), 1);
            assert_eq!(
                description::extract_history(&format!(
                    "{}\n{info}\n{}",
                    description::START,
                    description::END
                ))
                .iter()
                .map(|e| e.number)
                .collect::<Vec<_>>(),
                vec![99, 100, 101, 102, 103]
            );
            assert_eq!(info.contains("✔️"), emoji);
        }
    }
    assert_eq!(
        description::extract_history(""),
        [] as [description::HistoryEntry; 0]
    );
    assert_eq!(
        description::extract_history("No history"),
        [] as [description::HistoryEntry; 0]
    );
    let entries = description::extract_history(
        "Description\n---\n<!-- git-pr-stack-start -->\n* ✔️ #1\n* #2\n* ⬛ #2\n* invalid\n* #3 title\n<!-- git-pr-stack-end -->",
    );
    assert_eq!(
        entries,
        vec![
            HistoryEntry {
                number: 1,
                merged: true
            },
            HistoryEntry {
                number: 2,
                merged: false
            },
            HistoryEntry {
                number: 3,
                merged: false
            }
        ]
    );
    let style = Style {
        stack_up: false,
        emoji: true,
        host: "github.com",
        repo: "org/repo",
    };
    let without_pr = Commit {
        pr_number: 0,
        ..commit(9)
    };
    assert!(
        description::render_commit(&without_pr, &stack[0], false, style)
            .contains("test&#x200B;@example.com")
    );
    let without_email = Commit {
        author_email: "unknown".into(),
        ..without_pr.clone()
    };
    assert!(
        description::render_commit(&without_email, &without_email, false, style)
            .contains("unknown&#x200B;")
    );
    assert!(
        !crate::tests::legacy::generate_stack(
            &[without_pr],
            &stack[0],
            &[],
            &HashSet::new(),
            style
        )
        .contains("PR **")
    );
    assert_eq!(
        crate::tests::legacy::generate_stack(
            &[stack[0].clone(), stack[0].clone()],
            &stack[0],
            &[],
            &HashSet::new(),
            style
        )
        .lines()
        .count(),
        1
    );
}

#[test]
fn description_preserves_sentinels_bots_crlf_and_legacy_sections() {
    let template = "# Template\n";
    let stack = "* ⬛ #101\n* ⬛ #102";
    for existing in [
        "# Description",
        "",
        "# Template\n",
        description::BODY_TEMPLATE,
        "# Description\n---\n* ⬛ #999\n---\nBot",
        "# Description\n---\nOther section",
    ] {
        let body = description::generate_body("", existing, stack, template);
        assert_eq!(body.matches(description::START).count(), 1);
        assert_eq!(body.matches(description::END).count(), 1);
        assert!(body.contains(stack));
        assert!(!body.contains("#999"));
        if existing.contains("Bot") {
            assert!(body.contains("Bot"));
        }
    }
    for placeholder in ["", description::BODY_TEMPLATE, template] {
        let seeded =
            description::generate_body("Initial commit content", placeholder, stack, template);
        assert!(seeded.starts_with("Initial commit content\n\n---\n"));
    }
    let mut body = description::generate_body("new message", "old description", stack, template);
    assert!(body.starts_with("old description\n\n---\n"));
    assert!(!body.contains("new message"));
    for n in 1..=3 {
        write!(body, "\n\n---\nBot {n}").unwrap();
        body = description::generate_body("stale message", &body, &format!("Stack v{n}"), template);
        assert!(body.contains("old description"));
        assert!(!body.contains("stale message"));
        assert!(body.contains(&format!("Stack v{n}")));
        for b in 1..=n {
            assert!(body.contains(&format!("Bot {b}")));
        }
        assert_eq!(body.matches(description::START).count(), 1);
    }
    let existing = format!(
        "Human\r\n\r\n{}\r\nOld\r\n{}\r\n",
        description::START,
        description::END
    );
    let body = description::generate_body("stale", &existing, stack, template);
    assert!(body.starts_with("Human\n\n"));
    assert!(!body.contains('\r'));
    assert!(!body.contains("Old"));
    assert_eq!(
        description::generate_body("", "", "X", template),
        format!(
            "{template}\n---\n{}\nX\n{}",
            description::START,
            description::END
        )
    );
}

#[test]
fn legacy_footer_recognition_preserves_ambiguous_human_sections() {
    for section in [
        "## Verification\n* Fixes #123\nManual verification details",
        "* Fixes #123",
        "* #123",
        "* 📝 #123",
        "* ⬛ 123",
        "* ⬛ #123\nHuman notes",
        "    * ⬛ #123",
        "* ⬛ #0",
        "* ⬛ #18446744073709551616",
        "* ⬛ #",
        "* ⬛ #123attached",
        "* ⬛ #123 title",
        "* #123 👈 This PR ()",
        "* #123 👈 This PR (xyz)",
        "* #123 👈 This PR (abc",
        "This is PR nonsense\n* #123",
        "This is PR 0 of 2\n* #123",
        "This is PR 3 of 2\n* #123",
        "This is PR x of 2\n* #123",
        "This is PR **1 of 2\n* #123",
        "This is PR 1 of 2",
        "This is PR 1 of 2\nThis is PR 1 of 2\n* #123",
    ] {
        let existing = format!("Human summary\n---\n{section}\n---\nBot report");
        assert!(
            description::extract_history(&existing).is_empty(),
            "{section}"
        );
        let body = description::generate_body("", &existing, "* #101", "");
        assert!(body.starts_with(&existing), "{section}");
    }
}

#[test]
fn legacy_footers_migrate_once_with_or_without_emoji() {
    for header in [
        "This is PR 1 of 2",
        "This is PR **1 of 2** in a stack",
        "This is PR **1 of 2** in a stack (oldest at the top)",
        "This is PR **1 of 2** in a stack (newest at the top)",
    ] {
        for ending in ["\n", "\r\n"] {
            let existing =
                format!("Human summary\n---\n{header}\n\n* #101\n* #102\n---\nBot report")
                    .replace('\n', ending);
            let history = description::extract_history(&existing);
            assert_eq!(
                history.iter().map(|e| e.number).collect::<Vec<_>>(),
                if header.contains("newest") {
                    vec![102, 101]
                } else {
                    vec![101, 102]
                }
            );
            let body = description::generate_body("", &existing, "* #101\n* #102", "");
            assert_eq!(body.matches("* #101").count(), 1);
            assert_eq!(body.matches("* #102").count(), 1);
            assert!(body.starts_with("Human summary\n---\n"));
            assert!(body.ends_with("\n---\nBot report"));
            assert_eq!(
                description::generate_body("", &body, "* #101\n* #102", ""),
                body
            );
        }
    }
    for row in [
        "* #101 👈 This PR",
        "* #101 👈 This PR (Ab12)",
        "* ◻️ #101",
        "* 👉 #101",
        "* 🐹 #101",
        "* ✔️ #101",
    ] {
        assert_eq!(
            description::extract_history(row),
            vec![description::HistoryEntry {
                number: 101,
                merged: row.contains("✔️"),
            }]
        );
        let body = description::generate_body("", row, "* #101", "");
        assert_eq!(
            body,
            format!("{}\n* #101\n{}", description::START, description::END)
        );
    }
}

#[test]
fn malformed_history_markers_and_literal_output_braces() {
    for body in [
        description::START.to_owned(),
        description::END.to_owned(),
        format!("{}{}", description::END, description::START),
    ] {
        assert!(description::Body::parse(&body).is_err());
    }
    assert_eq!(
        description::extract_history("* \t #1\n* marker #x\n* marker\n* marker invalid\n* #"),
        [] as [description::HistoryEntry; 0]
    );
    for output in ["{", "{ }", "{}", "{{title}}", "{!}"] {
        assert!(crate::cli::output::resolve_format(output).is_ok());
    }
    let invalid_hash = "Z".repeat(40);
    assert!(
        crate::tests::legacy::parse_logs(&format!(
            "commit {invalid_hash}\nAuthor: Test <test@example.com>\n"
        ))
        .is_err()
    );
}

#[test]
fn indexed_branch_policy_and_repeated_identity_rendering_are_deterministic() {
    use crate::{
        stack::description::{StackRenderer, Style},
        stack::selection,
    };
    let names = selection::local_branches("main|a\nmalformed\nx|a|b\nfirst|a\nsecond|a");
    assert_eq!(names.get("a"), Some(&"first"));
    assert_eq!(names.get("b"), Some(&"x|a"));
    let w = crate::tests::unit::support::World::new(1);
    let commands = w.commands();
    let repo = crate::model::Repository::default();
    let git = crate::git::Git {
        commands: &commands,
        repo: &repo,
    };
    let c = crate::tests::unit::support::commit(1);
    assert_eq!(git.local_branch(&c, &format!("first|{}", c.hash)), "first");
    let stack: Vec<_> = (1..=3)
        .map(|n| crate::model::Commit {
            hash: "same".into(),
            pr_number: n,
            ..crate::model::Commit::default()
        })
        .collect();
    for stack_up in [false, true] {
        let renderer = StackRenderer::new(
            stack.iter(),
            &[],
            &std::collections::HashSet::new(),
            Style {
                stack_up,
                emoji: false,
                host: "github.com",
                repo: "org/repo",
            },
        );
        assert!(renderer.render(&stack[0]).contains("PR **1 of 3**"));
    }
}

#[test]
fn compact_footer_has_bounded_rows_at_scale_and_deduplicates_identities() {
    use crate::tests::unit::support::commit;
    for size in [1, 5, 100, 1000] {
        let mut stack: Vec<_> = (1..=size)
            .map(|n| {
                let mut c = commit(n);
                c.pr_number = n;
                c
            })
            .collect();
        stack.push(stack[0].clone());
        let renderer = crate::stack::description::CompactRenderer::new(&stack);
        for c in &stack {
            let footer = renderer.render(c);
            assert!(footer.lines().filter(|line| line.starts_with("* ")).count() <= 3);
            assert!(footer.len() < 200);
        }
        assert_eq!(renderer.render(&commit(size + 1)), "");
    }
    let empty = crate::stack::description::CompactRenderer::new(&[]);
    assert_eq!(empty.render(&commit(1)), "");
    let unresolved = [commit(1)];
    assert_eq!(
        crate::stack::description::CompactRenderer::new(&unresolved).render(&unresolved[0]),
        ""
    );
}

#[test]
fn filtered_range_boundaries_use_only_verified_ancestry() {
    use super::support::commit;
    let stack = vec![commit(1), commit(3)];
    let mut depths = std::collections::HashMap::from([
        (commit(1).hash, 2),
        (commit(2).hash, 1),
        (commit(3).hash, 0),
    ]);
    assert_eq!(
        selection::slice_start(&stack, &commit(2).hash, "trunk", &depths),
        1
    );
    depths.remove(&commit(3).hash);
    assert_eq!(
        selection::slice_start(&stack, &commit(2).hash, "trunk", &depths),
        stack.len()
    );
}

#[test]
fn cached_body_analysis_retains_legacy_boundaries_and_invalidates_changed_text() {
    use super::support::BODY_ANALYSES;
    for newline in ["\n", "\r\n"] {
        for section in [
            "This is PR **1 of 2** in a stack\n\n* #101 👈 This PR\n* #102",
            "<!-- git-pr-stack-start -->\n* #101\n* #102\n<!-- git-pr-stack-end -->",
        ] {
            let source =
                format!("Human text\n---\n{section}\n---\nBot text").replace('\n', newline);
            let mut cache = description::BodyCache::default();
            BODY_ANALYSES.set(0);
            assert_eq!(cache.get(101, &source).unwrap().history().len(), 2);
            let expected = cache
                .get(101, &source)
                .unwrap()
                .render("ignored", "* #101", "")
                .unwrap();
            assert!(expected.starts_with("Human text\n---\n"));
            assert!(expected.ends_with("---\nBot text"));
            assert!(!expected.contains("#102"));
            assert_eq!(BODY_ANALYSES.get(), 1);
            assert_eq!(cache.get(101, &source).unwrap().history().len(), 2);
            assert_eq!(BODY_ANALYSES.get(), 1);
            let edited = source.replace("Human text", "Fresh human text");
            let updated = cache
                .get(101, &edited)
                .unwrap()
                .render("ignored", "* #101", "")
                .unwrap();
            assert!(updated.starts_with("Fresh human text"));
            assert_eq!(BODY_ANALYSES.get(), 2);
            assert_eq!(
                cache
                    .get(101, &edited)
                    .unwrap()
                    .render("", "* #101", "")
                    .unwrap(),
                updated
            );
            assert_eq!(BODY_ANALYSES.get(), 2);
        }
    }
}

#[test]
fn malformed_footer_boundaries_cannot_be_adopted_or_replace_cached_human_text() {
    let start = description::START;
    let end = description::END;
    for malformed in [
        format!("Summary\n{start}\nHuman notes"),
        format!("Summary\n{end}\nHuman notes"),
        format!("{end}\nHuman notes\n{start}"),
        format!("{start}\nHuman notes\n{start}\n* #101\n{end}"),
        format!("{start}\n* #101\n{end}\nHuman notes\n{end}"),
        format!("{start}\n* #101\n{end}\nHuman notes\n{start}\n* #102\n{end}"),
    ] {
        for newline in ["\n", "\r\n"] {
            let malformed = malformed.replace('\n', newline);
            let valid = format!("Summary\n{start}\n* #101\n{end}\nHuman notes");
            let mut cache = description::BodyCache::default();
            for source in [malformed.as_str(), valid.as_str(), malformed.as_str()] {
                if source == valid {
                    assert_eq!(cache.get(101, source).unwrap().history()[0].number, 101);
                } else {
                    let diagnostic = format!("{:#}", cache.get(101, source).unwrap_err());
                    assert!(diagnostic.contains("PR #101 description"));
                    assert!(diagnostic.contains("malformed managed stack footer"));
                    assert!(diagnostic.contains("retain exactly one"));
                    assert!(diagnostic.contains("this description was not updated"));
                    assert!(!diagnostic.contains("Human notes"));
                }
            }
            let mut rendered = cache
                .get(101, &valid)
                .unwrap()
                .render("", "* #101", "")
                .unwrap();
            for _ in 0..4 {
                let repeated = cache
                    .get(101, &rendered)
                    .unwrap()
                    .render("", "* #101", "")
                    .unwrap();
                assert_eq!(repeated, rendered);
                assert!(repeated.contains("Human notes"));
                assert_eq!(repeated.matches(start).count(), 1);
                assert_eq!(repeated.matches(end).count(), 1);
                rendered = repeated;
            }
        }
    }
}

#[test]
fn initial_description_sources_cannot_introduce_reserved_footer_delimiters() {
    let start = description::START;
    let end = description::END;
    for seed in [
        format!("Human notes\n{start}"),
        format!("Human notes\n{end}"),
        format!("Human notes\n{start}\nExample content\n{end}"),
        format!("Human notes\n```html\n{start}\n```"),
    ] {
        for template_source in [false, true] {
            let (message, template, source) = if template_source {
                ("", seed.as_str(), "PR template")
            } else {
                (seed.as_str(), "Unused template", "commit message")
            };
            for existing in ["", description::BODY_TEMPLATE, template] {
                // A template already observed with delimiters is subject to the
                // separate authoritative managed-section contract.
                if existing == seed {
                    continue;
                }
                let body = description::Body::parse(existing).unwrap();
                let diagnostic = body
                    .render(message, "* #101", template)
                    .unwrap_err()
                    .to_string();
                assert!(diagnostic.contains(source));
                assert!(diagnostic.contains("reserved managed-footer delimiters"));
                assert!(diagnostic.contains("&lt;"));
                assert!(!diagnostic.contains("Human notes"));
            }
            let escaped = seed.replace('<', "&lt;");
            let (message, template) = if template_source {
                ("", escaped.as_str())
            } else {
                (escaped.as_str(), "Unused template")
            };
            let rendered = description::Body::parse("")
                .unwrap()
                .render(message, "* #101", template)
                .unwrap();
            assert!(rendered.contains(&escaped));
            assert_eq!(rendered.matches(start).count(), 1);
            assert_eq!(rendered.matches(end).count(), 1);
            let repeated = description::Body::parse(&rendered)
                .unwrap()
                .render(message, "* #101", template)
                .unwrap();
            assert_eq!(repeated, rendered);
        }
        let adopted = description::Body::parse("Existing human notes")
            .unwrap()
            .render(&seed, "* #101", &seed)
            .unwrap();
        assert!(adopted.starts_with("Existing human notes"));
        assert!(!adopted.contains("Example content"));
        assert_eq!(
            description::initial_body("Used message", &seed).unwrap(),
            "Used message"
        );
    }
}

#[test]
fn full_stack_retains_multiple_unnumbered_commits_and_escapes_context_text() {
    let stack = [
        commit(101),
        Commit {
            pr_number: 0,
            title: format!(
                "[Pending] \\ `feature` * _ & < > {}\r\n",
                description::START
            ),
            ..commit(102)
        },
        Commit {
            pr_number: 0,
            title: "Other pending feature".into(),
            author_email: "unknown".into(),
            ..commit(103)
        },
        commit(101),
    ];
    for stack_up in [false, true] {
        for emoji in [false, true] {
            let text = description::StackRenderer::new(
                stack.iter(),
                &[HistoryEntry {
                    number: 99,
                    merged: true,
                }],
                &HashSet::new(),
                Style {
                    stack_up,
                    emoji,
                    host: "github.com",
                    repo: "org/repo",
                },
            )
            .render(&stack[0]);
            assert!(text.contains("This PR is entry **2 of 4**"));
            assert_eq!(
                text.lines().filter(|line| line.starts_with("* ")).count(),
                4
            );
            assert_eq!(text.matches("#101").count(), 1);
            assert!(!text.contains("#0"));
            assert!(text.contains("\\[Pending\\] \\\\ \\`feature\\` \\* \\_ &amp; &lt; &gt;"));
            assert!(text.contains("Other pending feature"));
            assert!(text.contains(&format!("/commit/{}", stack[1].hash)));
            assert!(text.contains(&format!("/commit/{}", stack[2].hash)));
            assert_eq!(
                text.find("Other pending feature").unwrap() < text.find("Pending").unwrap(),
                stack_up
            );
            let body = description::generate_body("", "Summary", &text, "");
            assert_eq!(body.matches(description::START).count(), 1);
            assert_eq!(description::Body::parse(&body).unwrap().history().len(), 2);
        }
    }
    let compact = description::CompactRenderer::new(&stack).render(&stack[0]);
    assert!(compact.contains("PR 1 of 1"));
    assert!(!compact.contains("Pending"));
}

#[test]
fn stack_direction_cli_aliases_reject_only_conflicting_effective_values() {
    for (current, legacy) in [("true", "true"), ("false", "false")] {
        let args = [
            format!("--stack-up={current}"),
            format!("--reverse={legacy}"),
        ];
        let parsed = Flags::parse(args.to_vec()).unwrap();
        assert_eq!(parsed.boolean("stack-up"), current == "true");
        assert_eq!(parsed.boolean("reverse"), legacy == "true");
    }
    for args in [
        ["--stack-up", "--reverse=false"],
        ["--reverse", "--stack-up=false"],
    ] {
        assert_eq!(
            Flags::parse(strings(&args)).unwrap_err(),
            ParseError::Invalid(
                "conflicting --stack-up and deprecated --reverse values; use --stack-up alone"
                    .into()
            )
        );
    }
    assert!(!flags(&["--stack-up", "--stack-up=false"]).boolean("stack-up"));
    assert!(!flags(&["--reverse=false"]).boolean("reverse"));
    assert!(flags(&[]).boolean("reverse"));
}
