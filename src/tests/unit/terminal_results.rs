//! Terminal policies and complete results are tested without a terminal or repository.

use std::collections::BTreeMap;

use super::support::World;
use crate::{
    cli::{
        Flags,
        hyperlinks::{self, Mode},
        results::{self, Options},
    },
    config::{self, Loaded},
    model::{Commit, Repository},
};
use unicode_width::UnicodeWidthStr as _;

fn repo() -> Repository {
    Repository {
        host: "github.com".into(),
        name: "org/repo".into(),
        ..Repository::default()
    }
}

fn visible(text: &str) -> String {
    regex::Regex::new(r"\x1b\]8;;[^\x1b]*\x1b\\|\x1b\[[0-9;:]*m")
        .unwrap()
        .replace_all(text, "")
        .into_owned()
}

fn render(
    commits: &[Commit],
    repo: &Repository,
    columns: usize,
    colour: bool,
    hyperlinks: bool,
) -> String {
    results::render(
        &commits.iter().collect::<Vec<_>>(),
        repo,
        true,
        Options {
            summaries: &[],
            use_icon_font: false,
            columns,
            colour,
            hyperlinks,
        },
    )
}

#[test]
fn hyperlink_modes_are_explicit() {
    assert_eq!(Mode::default(), Mode::Auto);
    for (name, expected) in [
        ("auto", Mode::Auto),
        ("always", Mode::Always),
        ("never", Mode::Never),
    ] {
        assert_eq!(Mode::parse(name).unwrap(), expected);
        assert_eq!(expected.enabled(), name != "never");
        for args in [
            vec![format!("--hyperlinks={name}")],
            vec!["--hyperlinks".into(), name.into()],
            vec!["-hyperlinks".into(), name.into()],
        ] {
            assert_eq!(Flags::parse(args).unwrap().hyperlinks, Some(expected));
        }
    }
    assert!(Flags::default().hyperlinks.is_none());
    for value in ["", "true", "false", "AUTO", "invalid"] {
        assert!(
            Mode::parse(value)
                .unwrap_err()
                .contains("expected auto, always or never")
        );
        assert!(Flags::parse(vec![format!("--hyperlinks={value}")]).is_err());
    }
    assert!(Flags::parse(vec!["--hyperlinks".into()]).is_err());
    assert_eq!(
        Flags::parse(vec!["--hyperlinks=bad".into(), "--hyperlinks=never".into()])
            .unwrap()
            .hyperlinks,
        Some(Mode::Never)
    );
    assert!(
        Flags::parse(vec!["HEAD".into(), "--hyperlinks=bad".into()])
            .unwrap()
            .hyperlinks
            .is_none()
    );
}

#[test]
fn preview_limit_defaults_and_invalid_values_are_resolved_before_effects() {
    let world = World::new(0);
    assert_eq!(world.config().preview_limit, 20);
    for limit in [0, 1, 5, 20, 40, usize::MAX] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert("git-pr.previewLimit".into(), limit.to_string());
        assert_eq!(world.config().preview_limit, limit);
    }
    for value in ["", "-1", "true", "1.5", "twenty", "18446744073709551616"] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert("git-pr.previewLimit".into(), value.into());
        world.0.lock().unwrap().trace.clear();
        let error = config::load(
            &Flags::default(),
            &BTreeMap::new(),
            &mut world.commands(),
            world.as_ref(),
            world.as_ref(),
            false,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("git-pr.previewLimit"));
        assert!(world.trace().iter().all(|event| event["program"] == "git"));
    }
}

#[test]
fn hyperlink_configuration_resolves_only_the_effective_value_before_effects() {
    let world = World::new(0);
    assert_eq!(world.config().hyperlinks, Mode::Auto);
    for (configured, expected) in [
        ("auto", Mode::Auto),
        ("always", Mode::Always),
        ("never", Mode::Never),
    ] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert("git-pr.hyperlinks".into(), configured.into());
        assert_eq!(world.config().hyperlinks, expected);
    }
    for configured in ["invalid", "", "true"] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert("git-pr.hyperlinks".into(), configured.into());
        let load = |arguments: &[&str]| {
            config::load(
                &Flags::parse(arguments.iter().map(|s| (*s).into()).collect()).unwrap(),
                &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
                &mut world.commands(),
                world.as_ref(),
                world.as_ref(),
                false,
            )
        };
        world.0.lock().unwrap().trace.clear();
        let error = format!("{:#}", load(&[]).err().unwrap());
        assert!(
            error.contains("git-pr.hyperlinks") && error.contains("expected auto, always or never")
        );
        assert!(world.trace().iter().all(|event| event["program"] == "git"));
        let Loaded::Ready(config) = load(&["--hyperlinks=auto"]).unwrap() else {
            panic!("configuration expected")
        };
        assert_eq!(config.hyperlinks, Mode::Auto);
    }
}

#[test]
fn hyperlink_destinations_encode_components_and_reject_terminal_controls() {
    assert_eq!(
        hyperlinks::destination(&repo(), 142).unwrap(),
        "https://github.com/org/repo/pull/142"
    );
    let mut repository = repo();
    repository.host = "github.example.com:8443".into();
    repository.name = "café/a #?%".into();
    assert_eq!(
        hyperlinks::destination(&repository, 7).unwrap(),
        "https://github.example.com:8443/caf%C3%A9/a%20%23%3F%25/pull/7"
    );
    for host in [
        "",
        "[broken",
        "user:secret@github.com",
        "github.com/path",
        "github.com?x",
        "github.com#x",
        "github.com\\x",
        "github.com\n",
        "git hub.com",
        "github.com\x1b]8;;bad",
    ] {
        repository.host = host.into();
        assert!(
            hyperlinks::destination(&repository, 1).is_none(),
            "{host:?}"
        );
    }
    repository.host = "github.com".into();
    for name in [
        "repo",
        "/repo",
        "org/",
        "org/.",
        "../repo",
        "org/../repo",
        "org/repo\x07",
        "org/repo\n",
    ] {
        repository.name = name.into();
        assert!(
            hyperlinks::destination(&repository, 1).is_none(),
            "{name:?}"
        );
    }
    assert_eq!(
        hyperlinks::link("#142", "https://github.com/org/repo/pull/142"),
        "\x1b]8;;https://github.com/org/repo/pull/142\x1b\\#142\x1b]8;;\x1b\\"
    );
}

#[test]
fn uniform_results_have_one_body_row_per_pr_and_full_url_fallback() {
    for (status, glyph) in [("created", "+"), ("updated", "~"), ("unchanged", "=")] {
        let commits: Vec<_> = (1..=10)
            .map(|number| Commit {
                pr_number: number,
                title: format!("Feature {number} [TEST-{number}]"),
                newly_created: status == "created",
                metadata_updated: status != "unchanged",
                ..Commit::default()
            })
            .collect();
        let linked = render(&commits, &repo(), 120, false, true);
        let plain = visible(&linked);
        assert!(plain.starts_with(&format!(
            "\n  ✓ Pull requests · 10 {status}\n  org/repo · tip first\n\n"
        )));
        assert_eq!(plain.matches(status).count(), 1);
        assert_eq!(
            plain
                .lines()
                .filter(|line| line.trim_start().starts_with(glyph))
                .count(),
            10
        );
        assert_eq!(linked.matches("\x1b]8;;https://").count(), 10);
        assert!(!plain.contains("https://"));
        for number in 1..=10 {
            assert!(plain.contains(&format!(
                "  {glyph}  {:>3}  ?  Feature {number} [TEST-{number}]\n",
                format!("#{number}")
            )));
            assert!(linked.contains(&format!("https://github.com/org/repo/pull/{number}\x1b\\#{number}\x1b]8;;\x1b\\  ?  Feature {number}")));
        }
        let fallback = render(&commits, &repo(), 120, false, false);
        assert_eq!(fallback.lines().filter(|line| !line.is_empty()).count(), 22);
        assert!(!fallback.contains('\x1b'));
        assert!(fallback.contains(&format!(
            "[TEST-9]\n             https://github.com/org/repo/pull/9\n  {glyph}  #10"
        )));
    }
}

#[test]
fn mixed_results_align_columns_without_reordering_or_losing_titles() {
    let commits = [
        Commit {
            pr_number: 7,
            title: "Created [T-1]".into(),
            newly_created: true,
            metadata_updated: true,
            ..Commit::default()
        },
        Commit {
            pr_number: 1000,
            title: "Unchanged [T-2]".into(),
            ..Commit::default()
        },
        Commit {
            pr_number: 42,
            title: "Updated [T-3]".into(),
            base_updated: true,
            ..Commit::default()
        },
    ];
    for colour in [false, true] {
        let text = render(&commits, &repo(), 120, colour, true);
        let plain = visible(&text);
        assert!(
            plain
                .lines()
                .all(|line| line.is_empty() || line.starts_with("  "))
        );
        assert!(plain.contains("1 created · 1 updated · 1 unchanged"));
        assert!(plain.contains(
            "  +     #7  ?  Created [T-1]\n  =  #1000  ?  Unchanged [T-2]\n  ~    #42  ?  Updated [T-3]"
        ));
        assert_eq!(text.contains("\x1b[1m"), colour);
        for (glyph, code) in [("+", "32"), ("~", "33"), ("=", "2")] {
            assert_eq!(
                text.contains(&format!("\x1b[{code}m{glyph}\x1b[0m")),
                colour
            );
        }
    }
    let reverse = results::render(
        &commits.iter().rev().collect::<Vec<_>>(),
        &repo(),
        false,
        Options {
            summaries: &[],
            use_icon_font: false,
            columns: 32,
            colour: false,
            hyperlinks: true,
        },
    );
    assert!(reverse.contains("tip last"));
    let plain = visible(&reverse);
    assert!(plain.find("#42").unwrap() < plain.find("#1000").unwrap());
    assert!(plain.contains("  ~    #42  ?  Updated [T-3]"));
}

#[test]
fn narrow_results_keep_one_title_row_and_preserve_full_plain_output() {
    let title = "fix: café 日本語 👩‍💻 e\u{301} extraordinarilylongword [T-199]";
    let commits = [Commit {
        pr_number: 42,
        title: title.into(),
        ..Commit::default()
    }];
    for columns in [0, 1, 4, 12, 40, 80, 120, 160] {
        let plain = visible(&render(&commits, &repo(), columns, false, true));
        let row = plain.lines().find(|line| line.contains("#42")).unwrap();
        assert_eq!(plain.matches("#42").count(), 1);
        assert!(row.contains('?'));
        if columns >= 12 {
            assert!(row.width() <= columns, "{row}");
        }
        if columns == 40 {
            assert!(row.ends_with('…'));
        }
        if columns >= 80 {
            assert!(row.contains(title));
        }
        assert!(
            !plain
                .lines()
                .any(|line| line.trim_start().starts_with("[T-199]"))
        );
    }
    let full = render(&commits, &repo(), usize::MAX, false, false);
    assert!(full.lines().any(|line| line.contains(title)));
    assert!(full.contains("https://github.com/org/repo/pull/42"));
    for columns in [10, 80] {
        let empty = render(
            &[Commit {
                pr_number: 1,
                ..Commit::default()
            }],
            &repo(),
            columns,
            false,
            true,
        );
        assert!(visible(&empty).contains("#1"));
    }
}

#[test]
fn empty_and_untrusted_results_remain_inert_and_keep_repository_context() {
    for repository in [
        Repository::default(),
        repo(),
        Repository {
            host: "example.com".into(),
            name: "org/repo".into(),
            ..Repository::default()
        },
    ] {
        let empty = render(&[], &repository, 80, false, false);
        assert!(empty.starts_with("\n  Pull requests\n"));
        assert!(!empty.contains('✓') && !empty.contains("0 created"));
        assert!(empty.contains("Nothing selected for publication."));
        if repository.host == "example.com" {
            assert!(empty.contains("example.com/org/repo"));
        }
    }
    let commits = [Commit {
        pr_number: 1,
        title: "Hello\x1b]8;;bad\x07\nworld".into(),
        ..Commit::default()
    }];
    let repository = Repository {
        host: "github.com\x1b".into(),
        name: "org/repo\x07".into(),
        ..Repository::default()
    };
    let text = render(&commits, &repository, 120, false, true);
    assert!(!text.contains('\x1b') && !text.contains('\x07'));
    assert!(text.contains("https://github.com\\u{1b}/org/repo\\u{7}/pull/1"));
    assert!(text.contains("Hello\\u{1b}]8;;bad\\u{7} world"));
}

#[test]
fn plain_templates_and_submission_effects_are_independent_of_hyperlink_mode() {
    for template in ["url-title", "url", "markdown", "#{number} {title}"] {
        let mut baseline = None;
        for mode in ["auto", "always", "never"] {
            let world = World::new(2);
            world
                .run(&["--no-stack", "--output", template, "--hyperlinks", mode])
                .unwrap();
            let output = world.0.lock().unwrap().stdout.clone();
            assert!(!output.contains('\x1b'));
            let trace = world.trace();
            let effects: Vec<_> = trace
                .iter()
                .filter(|event| event["kind"] != "command" || event["args"][0] != "config")
                .cloned()
                .collect();
            if let Some((expected_output, expected_effects)) = &baseline {
                assert_eq!(&output, expected_output);
                assert_eq!(&effects, expected_effects);
            } else {
                baseline = Some((output, effects));
            }
        }
    }
}

#[test]
fn icon_font_boolean_defaults_on_and_cli_overrides_configuration() {
    let world = World::new(0);
    assert!(world.config().use_icon_font);
    for configured in ["true", "false"] {
        world
            .0
            .lock()
            .unwrap()
            .config
            .insert("git-pr.useIconFont".into(), configured.into());
        for flag in [None, Some(true), Some(false)] {
            let args = flag
                .map(|value| format!("--use-icon-font={value}"))
                .into_iter()
                .collect();
            let flags = Flags::parse(args).unwrap();
            let Loaded::Ready(config) = config::load(
                &flags,
                &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
                &mut world.commands(),
                world.as_ref(),
                world.as_ref(),
                false,
            )
            .unwrap() else {
                panic!("expected configuration")
            };
            assert_eq!(config.use_icon_font, flag.unwrap_or(configured == "true"));
        }
    }
    assert!(Flags::parse(vec!["--use-icon-font=invalid".into()]).is_err());
    assert!(
        Flags::parse(vec!["--use-icon-font".into()])
            .unwrap()
            .boolean("use-icon-font")
    );
}

#[test]
fn submission_uses_configured_icons_and_observed_open_or_draft_states() {
    for use_icon_font in [false, true] {
        for draft in [false, true] {
            let world = World::new(2);
            world
                .0
                .lock()
                .unwrap()
                .config
                .insert("git-pr.useIconFont".into(), use_icon_font.to_string());
            let draft_flag = format!("--draft={draft}");
            let icon = match (use_icon_font, draft) {
                (true, false) => "\u{f407}",
                (true, true) => "\u{f4dd}",
                (false, false) => "○",
                (false, true) => "◌",
            };
            for (change, outcome) in [("+", "created"), ("=", "unchanged")] {
                world.0.lock().unwrap().stdout.clear();
                world.run(&["--no-stack", &draft_flag]).unwrap();
                let memory = world.0.lock().unwrap();
                assert!(
                    memory
                        .stdout
                        .contains(&format!("Pull requests · 2 {outcome}"))
                );
                for number in [101, 102] {
                    assert!(
                        memory
                            .stdout
                            .contains(&format!("  {change}  #{number}  {icon}  Feature"))
                    );
                }
                drop(memory);
            }
        }
    }
}

#[test]
fn complete_stack_has_independent_change_and_lifecycle_icons_without_sections() {
    use crate::model::pull_request::{Lifecycle, PullRequestSummary};
    let commits = [
        Commit {
            pr_number: 20,
            title: "Created".into(),
            newly_created: true,
            ..Commit::default()
        },
        Commit {
            pr_number: 21,
            title: "Updated".into(),
            metadata_updated: true,
            ..Commit::default()
        },
        Commit {
            pr_number: 22,
            title: "Unchanged".into(),
            ..Commit::default()
        },
    ];
    let summaries = [
        PullRequestSummary {
            number: 1,
            title: Some("Closed title".into()),
            lifecycle: Lifecycle::Closed,
        },
        PullRequestSummary {
            number: 2,
            title: Some("Merged title".into()),
            lifecycle: Lifecycle::Merged,
        },
        PullRequestSummary {
            number: 3,
            title: Some("Draft title".into()),
            lifecycle: Lifecycle::Draft,
        },
        PullRequestSummary {
            number: 4,
            title: None,
            lifecycle: Lifecycle::Unknown,
        },
        PullRequestSummary {
            number: 20,
            title: None,
            lifecycle: Lifecycle::Open,
        },
        PullRequestSummary {
            number: 21,
            title: Some("Updated".into()),
            lifecycle: Lifecycle::Draft,
        },
        PullRequestSummary {
            number: 22,
            title: Some("Unchanged".into()),
            lifecycle: Lifecycle::Open,
        },
    ];
    for use_icon_font in [false, true] {
        for colour in [false, true] {
            for stack_up in [false, true] {
                let text = results::render(
                    &commits.iter().collect::<Vec<_>>(),
                    &repo(),
                    stack_up,
                    Options {
                        columns: 140,
                        colour,
                        hyperlinks: true,
                        summaries: &summaries,
                        use_icon_font,
                    },
                );
                let plain = visible(&text);
                assert!(plain.contains("1 created · 1 updated · 1 unchanged · 4 historical"));
                assert_eq!(
                    plain.find("#22").unwrap() < plain.find("#1 ").unwrap(),
                    stack_up
                );
                assert!(plain.contains("PR details unavailable"));
                assert!(plain.contains("Created"));
                assert!(!plain.contains("Current ·") && !plain.contains("History ·"));
                for (icon, fallback, code) in [
                    ("\u{f407}", "○", "32"),
                    ("\u{f4dd}", "◌", "90"),
                    ("\u{f419}", "✓", "35"),
                    ("\u{f4dc}", "×", "31"),
                ] {
                    let glyph = if use_icon_font { icon } else { fallback };
                    assert!(plain.contains(glyph));
                    assert_eq!(
                        text.contains(&format!("\x1b[{code}m{glyph}\x1b[0m")),
                        colour
                    );
                }
                assert_eq!(text.contains("\x1b[2mMerged title\x1b[0m"), colour);
                assert!(
                    plain
                        .lines()
                        .all(|line| line.is_empty() || line.starts_with("  "))
                );
            }
        }
    }
    let duplicates = [summaries[0].clone(), summaries[0].clone()];
    let text = results::render(
        &commits.iter().collect::<Vec<_>>(),
        &repo(),
        false,
        Options {
            columns: 120,
            colour: false,
            hyperlinks: false,
            summaries: &duplicates,
            use_icon_font: false,
        },
    );
    assert_eq!(text.matches("Closed title").count(), 1);
    assert!(text.contains("Created") && text.contains("1 historical"));
}

#[test]
fn narrow_summary_preserves_all_outcomes_and_repository_identity() {
    use crate::model::pull_request::{Lifecycle, PullRequestSummary};
    let commits = [
        Commit {
            pr_number: 1,
            newly_created: true,
            ..Commit::default()
        },
        Commit {
            pr_number: 2,
            metadata_updated: true,
            ..Commit::default()
        },
        Commit {
            pr_number: 3,
            ..Commit::default()
        },
    ];
    let summaries = [PullRequestSummary {
        number: 100,
        title: Some("Historical descriptive title ".repeat(20)),
        lifecycle: Lifecycle::Merged,
    }];
    for columns in [40, 80, 120, 160] {
        let output = results::render(
            &commits.iter().collect::<Vec<_>>(),
            &repo(),
            true,
            Options {
                columns,
                colour: false,
                hyperlinks: true,
                summaries: &summaries,
                use_icon_font: false,
            },
        );
        let plain = visible(&output);
        let joined = plain.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(joined.contains("1 created · 1 updated · 1 unchanged · 1 historical"));
        assert!(joined.contains("org/repo · tip first"));
        for number in [1, 2, 3, 100] {
            assert_eq!(plain.matches(&format!("#{number} ")).count(), 1);
        }
        assert!(plain.lines().all(|line| line.width() <= columns));
    }
}
