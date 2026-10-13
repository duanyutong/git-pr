//! Independent examples and bounded generators for the behavioural contracts.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use proptest::{
    prelude::*,
    test_runner::{Config, RngSeed},
};

use super::support::{World, commit};
use crate::{
    git::Git,
    model::trailers,
    model::{Commit, REMOTE_REF, Repository},
    stack::description::{self, Body},
    stack::selection::{self, RefSync},
};

// Config::default reads PROPTEST_CASES and PROPTEST_RNG_SEED. A fixed ordinary
// seed makes PR results reproducible; scheduled campaigns supply other seeds.
fn config() -> Config {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 128;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        config.rng_seed = RngSeed::Fixed(0x4750_5241_5353_5552);
    }
    config.max_shrink_iters = 4096;
    config
}

fn value() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("x".to_owned()),
        Just("é".to_owned()),
        Just("🦀".to_owned()),
        Just("a|b:c, d".to_owned()),
        "[a-zA-Z0-9éλ中🦀|:,_./ -]{1,48}".prop_map(|s| format!("v{s}")),
    ]
}

fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("Remote-Ref".to_owned()),
        Just("Tags".to_owned()),
        Just("X-Unknown".to_owned()),
        "[a-zA-Z0-9-]{1,16}",
    ]
}

fn attributes() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec((key(), value()), 1..12)
}

fn human_text() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("Unknown: x\n\n---\nHuman notes 🦀".to_owned()),
        Just("<!-- other-tool -->\n* Fixes #123\n```rust\nlet x = 1;\n```".to_owned()),
        "[a-zA-Z0-9éλ中🦀 <>!\\[\\]*#:`|,._/\\n\\t-]{0,100}",
    ]
}

fn parsed(text: &str) -> (String, Vec<(String, String)>) {
    trailers::parse(&text.lines().collect::<Vec<_>>())
}

fn winners(attrs: &[(String, String)]) -> BTreeMap<String, String> {
    let mut expected = BTreeMap::new();
    for (key, value) in attrs {
        expected.entry(key.clone()).or_insert_with(|| value.clone());
    }
    expected
}

fn git_log_key() -> String {
    "git log --format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00 --no-decorate --no-color --encoding=UTF-8 --no-show-signature trunk..HEAD".into()
}

#[test]
fn duplicate_trailer_winners_survive_rewriting() {
    // On disk the final occurrence wins; internal attributes are newest first.
    let (message, attrs) = parsed(
        "Human notes\n\nRemote-Ref: first\nTags: a\nRemote-Ref: second\nTags: b\nX-Unknown: z",
    );
    let before = Commit {
        title: "Feature".into(),
        message,
        attrs,
        ..Commit::default()
    };
    assert_eq!(before.remote_ref(), "second");
    assert_eq!(before.tags(&[]), ["b"]);
    let output = before.full_message();
    let (_, body) = output.split_once('\n').unwrap();
    let (message, attrs) = parsed(body);
    let after = Commit {
        message,
        attrs,
        ..Commit::default()
    };
    assert_eq!(after.remote_ref(), "second");
    assert_eq!(after.tags(&[]), ["b"]);
    assert_eq!(after.attr("x-unknown"), "z");
    assert_eq!(after.message, "Human notes");
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn footer_semantics_follow_the_independent_source_order(
        mut attrs in attributes(),
        reverse in any::<bool>(),
        indent in prop::sample::select(vec!["", " ", "\t", " \t"]),
        separator in prop::sample::select(vec![":", ": ", " \t: \t"]),
        ending in prop::sample::select(vec!["\n", "\r\n"]),
    ) {
        if reverse {
            attrs.reverse();
        }
        let mut text = "Human body\n\n".to_owned();
        let mut expected = Vec::new();
        let mut effective = BTreeMap::new();
        for (key, value) in &attrs {
            writeln!(text, "{indent}{key}{separator}{value} \t").unwrap();
            let pair = (key.to_ascii_lowercase(), value.trim().to_owned());
            effective.insert(pair.0.clone(), pair.1.clone());
            expected.push(pair);
        }
        expected.reverse();
        let (message, actual) = parsed(&text.replace('\n', ending));
        prop_assert_eq!(message, "Human body");
        prop_assert_eq!(&actual, &expected);
        prop_assert_eq!(winners(&actual), effective);
    }

    #[test]
    fn rewriting_preserves_all_effective_trailers(attrs in attributes()) {
        let text = format!("Human body\n\n{}", attrs.iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>().join("\n"));
        let (message, attrs) = parsed(&text);
        let expected = winners(&attrs);
        let output = trailers::serialise("Feature", &message, &attrs);
        let (title, body) = output.split_once('\n').unwrap();
        let (actual_message, actual) = parsed(body);
        prop_assert_eq!(title, "Feature");
        prop_assert_eq!(&actual_message, "Human body");
        prop_assert_eq!(winners(&actual), expected);
        // The normal form is stable even when multiple keys occur repeatedly.
        prop_assert_eq!(trailers::serialise(title, &actual_message, &actual), output);
    }

    #[test]
    fn empty_or_invalid_footer_lines_preserve_the_entire_human_body(
        preceding in attributes(),
        invalid in prop::sample::select(vec!["Tags:", "Tags: \t", "Bad_Key: x", ": x", "é: x", "Human prose"]),
    ) {
        let footer = preceding.iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>().join("\n");
        let text = format!("Human body\n\n{footer}\n{invalid}\nRemote-Ref: topic");
        let (message, attrs) = parsed(&text);
        prop_assert_eq!(message, text.trim());
        prop_assert!(attrs.is_empty());
    }

    #[test]
    fn a_missing_blank_boundary_cannot_consume_human_prose(attrs in attributes()) {
        let footer = attrs.iter().map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>().join("\n");
        let text = format!("Human prose without a colon\n{footer}");
        let (message, actual) = parsed(&text);
        prop_assert_eq!(message, text.trim());
        prop_assert!(actual.is_empty());
    }

    #[test]
    fn branch_inventories_preserve_literal_names_and_the_first_alias(
        suffix in "[a-zA-Z0-9éλ中|_-]{1,32}",
        target in 1_u64..1000,
    ) {
        let name = format!("topic/{suffix}");
        let hash = format!("{target:040x}");
        let inventory = format!("main|{hash}\nmaster|{hash}\n{name}|{hash}\nalias|{hash}\nother|{:040x}", target + 1);
        prop_assert_eq!(selection::local_branch(&inventory, &hash), Some(name.clone()));
        let branches = selection::local_branches(&inventory);
        prop_assert_eq!(branches.get(hash.as_str()), Some(&name.as_str()));
        prop_assert_eq!(selection::local_branch(&inventory, &format!("{target:040x}0")), None);
    }

    #[test]
    fn observed_publication_refs_require_complete_literal_identity(
        suffix in "[a-zA-Z0-9éλ中|_-]{1,24}",
        number in 1_u64..1000,
    ) {
        let reference = format!("refs/heads/topic/{suffix}");
        let hash = format!("{number:040x}");
        let similar = format!("{hash}\t{reference}-other\n{hash}0\t{reference}\n{hash}\t{reference}\textra");
        prop_assert!(!selection::remote_ref_matches(&similar, &reference, &hash));
        let exact = format!("{similar}\n{hash}\t{reference}\n");
        prop_assert!(selection::remote_ref_matches(&exact, &reference, &hash));
    }

    #[test]
    fn selected_bounds_follow_stack_order_and_ignore_argument_order(
        count in 1_usize..32,
        arguments in prop::collection::vec(any::<usize>(), 1..48),
    ) {
        let stack = (1..=count).map(|n| commit(n as u64)).collect::<Vec<_>>();
        let indices = arguments.iter().map(|n| n % count).collect::<Vec<_>>();
        let selected = indices.iter().map(|&i| stack[i].hash.clone()).collect::<Vec<_>>();
        let expected = (*indices.iter().min().unwrap(), *indices.iter().max().unwrap());
        prop_assert_eq!(selection::order_selection(&stack, &selected).unwrap(), expected);
        let mut unknown = selected;
        unknown.push(format!("{:040x}", count + 1));
        prop_assert!(selection::order_selection(&stack, &unknown).is_err());
        prop_assert!(selection::order_selection(&stack, &[]).is_err());
    }

    #[test]
    fn selection_never_clears_existing_exclusions(
        decisions in prop::collection::vec((any::<bool>(), any::<bool>()), 0..32),
    ) {
        let mut stack = decisions.iter().enumerate().map(|(i, &(skip, _))| {
            Commit { skip, ..commit(i as u64 + 1) }
        }).collect::<Vec<_>>();
        let selected = stack.iter().zip(&decisions).filter(|(_, (_, included))| *included)
            .map(|(c, _)| c.hash.clone()).collect::<Vec<_>>();
        let announcements = selection::mark_unselected(&mut stack, &selected);
        prop_assert_eq!(stack.iter().map(|c| c.skip).collect::<Vec<_>>(),
            decisions.iter().map(|&(skip, included)| skip || !included).collect::<Vec<_>>());
        prop_assert_eq!(announcements.len(), decisions.iter().filter(|&&(skip, included)| !skip && !included).count());
    }

    #[test]
    fn filtered_range_boundaries_obey_verified_linear_ancestry(
        visible in prop::collection::vec(any::<bool>(), 1..32),
        base_input in any::<usize>(),
    ) {
        let base_index = base_input % visible.len();
        let all = (0..visible.len()).map(|i| commit(i as u64 + 1)).collect::<Vec<_>>();
        let stack = all.iter().zip(&visible).filter(|(_, keep)| **keep).map(|(c, _)| c.clone()).collect::<Vec<_>>();
        let depths = all.iter().enumerate().map(|(i, c)| (c.hash.clone(), all.len() - i - 1)).collect::<HashMap<_, _>>();
        let expected = visible.iter().take(base_index + 1).filter(|&&keep| keep).count();
        prop_assert_eq!(selection::slice_start(&stack, &all[base_index].hash, "trunk", &depths), expected);
        prop_assert_eq!(selection::slice_start(&stack, "trunk", "trunk", &depths), 0);
        prop_assert_eq!(selection::slice_start(&stack, "unknown", "trunk", &depths), stack.len());
    }

    #[test]
    fn bases_use_live_selected_predecessors_and_preserve_context(
        skips in prop::collection::vec(any::<bool>(), 1..32),
        start_input in any::<usize>(),
    ) {
        let start = start_input % (skips.len() + 1);
        let stack = skips.iter().enumerate().map(|(i, &skip)| Commit {
            skip, ..commit(i as u64 + 1)
        }).collect::<Vec<_>>();
        let expected = (0..stack.len()).map(|i| {
            (start..i).rev().find(|&j| !stack[j].skip)
                .or_else(|| i.checked_sub(1))
                .map_or("main", |j| stack[j].remote_ref()).to_owned()
        }).collect::<Vec<_>>();
        prop_assert_eq!(selection::resolve_bases(&stack, start, "main"), expected);
    }

    #[test]
    fn ref_recovery_requires_identity_or_authorised_ancestry(
        same in any::<bool>(), fast_forward in any::<bool>(), same_change in any::<bool>(),
        local in value(),
    ) {
        let pushed = if same { local.clone() } else { format!("different-{local}") };
        let expected = match (same, fast_forward, same_change) {
            (true, _, _) => RefSync::Nothing,
            (false, false, false) => RefSync::Unrelated,
            (false, _, _) => RefSync::Move,
        };
        prop_assert_eq!(selection::decide_ref_sync(&local, &pushed, fast_forward, same_change), expected);
    }

    #[test]
    fn structured_metadata_preserves_independently_framed_records(
        rows in prop::collection::vec((value(), value(), value()), 1..10),
        sha256 in any::<bool>(),
    ) {
        let width = if sha256 { 64 } else { 40 };
        let base = format!("{:0width$x}", 0);
        let mut frames = String::new();
        for (i, (title, author, tag)) in rows.iter().enumerate().rev() {
            let hash = format!("{:0width$x}", i + 1);
            let parent = format!("{i:0width$x}");
            writeln!(frames, "{hash}\0{author}\0author{i}@example.com\02024-06-01T12:30:00+02:00\0tree{}\0{parent}\0{title}\n\nBody {i}: commit fake\n\nTags: {tag}\nRemote-Ref: topic/{i}\n\0", i + 1).unwrap();
        }
        let w = World::new(0);
        w.reply(&git_log_key(), 0, &frames);
        w.reply("git show --no-show-signature -s --format=%H %T trunk", 0, &format!("{base} tree0"));
        let commands = w.commands();
        let repo = Repository::default();
        let observed = Git { commands: &commands, repo: &repo }.observe_stack("trunk", "HEAD", false).unwrap();
        prop_assert_eq!(observed.commits.len(), rows.len());
        prop_assert_eq!(&observed.base_hash, &base);
        prop_assert_eq!(observed.depths.get(&base), Some(&rows.len()));
        for (i, (actual, (title, author, tag))) in observed.commits.iter().zip(&rows).enumerate() {
            prop_assert_eq!(&actual.hash, &format!("{:0width$x}", i + 1));
            prop_assert_eq!(&actual.title, title.trim());
            prop_assert_eq!(&actual.author_name, author);
            prop_assert_eq!(&actual.author_email, &format!("author{i}@example.com"));
            prop_assert_eq!(&actual.message, &format!("Body {i}: commit fake"));
            prop_assert_eq!(actual.attr("tags"), tag.trim());
            prop_assert_eq!(actual.attr(REMOTE_REF), format!("topic/{i}"));
            prop_assert_eq!(actual.date.unwrap().to_rfc3339(), "2024-06-01T10:30:00+00:00");
            prop_assert_eq!(observed.depths.get(&actual.hash), Some(&(rows.len() - i - 1)));
        }
    }

    #[test]
    fn truncated_structured_records_fail_before_any_effect(
        present in 1_usize..7,
        text in value(),
    ) {
        let fields = [format!("{:040x}", 1), text, "author@example.com".into(),
            "2024-01-01T00:00:00Z".into(), "tree1".into(), format!("{:040x}", 0), "Feature".into()];
        let output = format!("{}\0", fields[..present].join("\0"));
        let w = World::new(0);
        w.reply(&git_log_key(), 0, &output);
        let commands = w.commands();
        let repo = Repository::default();
        let git = Git { commands: &commands, repo: &repo };
        prop_assert!(git.observe_stack("trunk", "HEAD", false).is_err());
        prop_assert_eq!(w.0.lock().unwrap().trace.len(), 1);
    }

    #[test]
    fn managed_updates_preserve_human_sections_and_reach_a_fixed_point(
        before in human_text(), after in human_text(), number in 1_u64..1000,
        crlf in any::<bool>(),
    ) {
        let before = format!("Human before: {before}");
        let after = format!("Human after: {after}");
        let existing = format!("{before}\n\n{}\n* #{}\n{}\n\n{after}", description::START, number, description::END);
        let existing = if crlf { existing.replace('\n', "\r\n") } else { existing };
        let footer = format!("* #{}", number + 1000);
        let actual = Body::parse(&existing).unwrap().render("Ignored message", &footer, "Ignored template").unwrap();
        let expected = format!("{}\n\n{}\n{footer}\n{}\n\n{}", before.trim_end_matches([' ', '\t', '\n']), description::START, description::END, after.trim_start_matches([' ', '\t', '\n']));
        prop_assert_eq!(&actual, &expected);
        let body = Body::parse(&actual).unwrap();
        prop_assert_eq!(body.history().iter().map(|entry| entry.number).collect::<Vec<_>>(), [number + 1000]);
        prop_assert_eq!(body.render_update("Different message", &footer, "Different template").unwrap(), None);
    }

    #[test]
    fn unmanaged_human_text_survives_adoption_and_repeated_updates(text in human_text()) {
        let existing = format!("Human notes: {text}");
        let footer = "* #101";
        let actual = Body::parse(&existing).unwrap().render("Ignored message", footer, "Ignored template").unwrap();
        let expected = format!("{existing}\n\n---\n{}\n{footer}\n{}", description::START, description::END);
        prop_assert_eq!(&actual, &expected);
        prop_assert_eq!(Body::parse(&actual).unwrap().render("", footer, "").unwrap(), actual);
    }

    #[test]
    fn history_retains_first_identity_occurrence_and_its_state(
        rows in prop::collection::vec((1_u64..12, any::<bool>()), 0..32),
    ) {
        let footer = rows.iter().map(|&(number, merged)| format!("* {}#{number}", if merged { "✔️ " } else { "" })).collect::<Vec<_>>().join("\n");
        let source = format!("{}\n{footer}\n{}", description::START, description::END);
        let mut expected = Vec::new();
        for &(number, merged) in &rows {
            if !expected.iter().any(|entry: &description::HistoryEntry| entry.number == number) {
                expected.push(description::HistoryEntry { number, merged });
            }
        }
        prop_assert_eq!(Body::parse(&source).unwrap().history(), expected);
    }

    #[test]
    fn malformed_managed_markers_are_rejected_without_rendering_human_text(
        text in human_text(), variant in 0_u8..6,
    ) {
        let start = description::START;
        let end = description::END;
        let source = match variant {
            0 => format!("{text}\n{start}"),
            1 => format!("{text}\n{end}"),
            2 => format!("{end}\n{text}\n{start}"),
            3 => format!("{start}\n{text}\n{start}\n{end}"),
            4 => format!("{start}\n{text}\n{end}\n{end}"),
            _ => format!("{start}\n{text}\n{end}\n{start}\n{end}"),
        };
        prop_assert!(Body::parse(&source).is_err());
    }
}
