//! Bounded semantic oracles shared by stable corpus replay and coverage-guided fuzzing.
#![allow(dead_code, unused_imports)]
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

// Source inclusion keeps the harness on the exact production implementation while
// preserving the production crate's visibility, dependencies and coverage inventory.
#[path = "../../src/stack/description.rs"]
mod description;
#[path = "../../src/model/mod.rs"]
mod model;
#[path = "../../src/git/snapshot.rs"]
mod snapshot;
use model::trailers as trailer_parser;

pub const MAX_INPUT: usize = 4096;
pub const TARGETS: [&str; 3] = ["trailers", "snapshots", "descriptions"];

pub fn run(target: &str, data: &[u8]) {
    assert!(
        data.len() <= MAX_INPUT,
        "input exceeds declared campaign bound"
    );
    match target {
        "trailers" => trailers(data),
        "snapshots" => snapshots(data),
        "descriptions" => descriptions(data),
        _ => panic!("unknown fuzz target: {target}"),
    }
}

/// Arbitrary text retains Unicode and punctuation; the structured value excludes
/// line separators so it represents precisely one non-empty trailer value.
fn value(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let text: String = text
        .chars()
        .filter(|character| !matches!(character, '\n' | '\r' | '\0'))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        "x".into()
    } else {
        text.into()
    }
}

fn assert_attrs(attrs: &[(String, String)]) {
    for (key, value) in attrs {
        assert!(!key.is_empty());
        assert!(
            key.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        );
        assert!(!value.is_empty());
        assert_eq!(value, value.trim());
    }
}

pub fn trailers(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let arbitrary = String::from_utf8_lossy(data);
    let (body, attrs) = trailer_parser::parse(&arbitrary.lines().collect::<Vec<_>>());
    assert_eq!(body, body.trim());
    assert_attrs(&attrs);

    // Independent grammar construction, rather than serialising to produce input.
    // Duplicate precedence and unknown one-character metadata are explicit oracles.
    let generated = value(data);
    let lines = [
        "Human paragraph.".to_owned(),
        String::new(),
        "Remote-Ref: old".to_owned(),
        "Unknown: x".to_owned(),
        format!(" \tTaGs \t: \t{generated} \t"),
        format!("Remote-Ref: {generated}"),
    ];
    let (body, attrs) =
        trailer_parser::parse(&lines.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(body, "Human paragraph.");
    assert_eq!(
        attrs,
        vec![
            ("remote-ref".into(), generated.clone()),
            ("tags".into(), generated.clone()),
            ("unknown".into(), "x".into()),
            ("remote-ref".into(), "old".into()),
        ]
    );
    let commit = model::Commit {
        attrs: attrs.clone(),
        ..model::Commit::default()
    };
    assert_eq!(commit.remote_ref(), generated);

    let rendered = trailer_parser::serialise("Title", &body, &attrs);
    let (_, footer) = rendered
        .split_once('\n')
        .expect("serialisation retains title separator");
    let (new_body, new_attrs) = trailer_parser::parse(&footer.lines().collect::<Vec<_>>());
    assert_eq!(new_body, body);
    let rewritten = model::Commit {
        attrs: new_attrs.clone(),
        ..model::Commit::default()
    };
    assert_eq!(rewritten.remote_ref(), commit.remote_ref());
    // Stable sorting groups duplicates; serialisation deliberately retains each
    // occurrence. Compare the multiset, since source order is not canonical.
    let mut expected = attrs;
    let mut actual = new_attrs;
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected);
}

pub fn snapshots(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let arbitrary = String::from_utf8_lossy(data);
    if let Ok(parsed) = snapshot::parse(&arbitrary) {
        for commit in &parsed.commits {
            assert!(matches!(commit.hash.len(), 40 | 64));
            assert!(commit.hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(!commit.author_name.is_empty());
            assert!(!commit.author_email.is_empty());
            assert!(commit.date.is_some());
            assert_attrs(&commit.attrs);
        }
    }

    let first = data.first().copied().unwrap_or(0);
    let count = usize::from(first % 5) + 1;
    let width = if first & 8 == 0 { 40 } else { 64 };
    let text = value(data);
    let mut framed = String::new();
    let base = format!("{:0width$x}", count + 1);
    let mut expected_parents = HashMap::new();
    let mut expected_trees = HashMap::new();
    for index in 0..count {
        let hash = format!("{:0width$x}", index + 1);
        let tree = format!("{:0width$x}", index + count + 2);
        let parent = format!("{:0width$x}", index + 2);
        let body = format!("Title {index}\n\nHuman {text}\n\nTags: x\nRemote-Ref: topic/{index}\n");
        let fields = [
            hash.as_str(),
            "Fuzz Author",
            "fuzz@example.invalid",
            "2026-10-15T12:34:56+02:00",
            &tree,
            &parent,
            &body,
        ];
        for field in fields {
            framed.push_str(field);
            framed.push('\0');
        }
        framed.push('\n');
        expected_parents.insert(hash.clone(), vec![parent]);
        expected_trees.insert(hash, tree);
    }
    let parsed = snapshot::parse(&framed).expect("independently framed valid metadata");
    assert_eq!(parsed.commits.len(), count);
    assert_eq!(parsed.parents, expected_parents);
    assert_eq!(parsed.trees, expected_trees);
    assert!(parsed.reject_merges().is_ok());
    let depths = snapshot::linear_depths(&parsed.commits, &parsed.parents);
    assert_eq!(depths.len(), count + 1);
    assert_eq!(depths.get(&base), Some(&count));
    for (index, commit) in parsed.commits.iter().enumerate() {
        assert_eq!(commit.hash, format!("{:0width$x}", index + 1));
        assert_eq!(commit.author_name, "Fuzz Author");
        assert_eq!(commit.author_email, "fuzz@example.invalid");
        assert_eq!(
            commit.date.unwrap().to_rfc3339(),
            "2026-10-15T10:34:56+00:00"
        );
        assert_eq!(commit.title, format!("Title {index}"));
        assert_eq!(commit.message, format!("Human {text}"));
        assert_eq!(commit.attr("tags"), "x");
        assert_eq!(commit.remote_ref(), format!("topic/{index}"));
        assert_eq!(depths.get(&commit.hash), Some(&index));
    }

    // Each missing first-record field is invalid, independently of arbitrary bytes.
    let fields: Vec<_> = framed.split('\0').take(7).collect();
    let cut = usize::from(first % 6) + 1;
    assert!(snapshot::parse(&fields[..cut].join("\0")).is_err());
}

fn observed(source: &str) -> anyhow::Result<description::BodyCache> {
    let mut cache = description::BodyCache::default();
    cache.get_shared(1, &Arc::from(source))?;
    Ok(cache)
}

fn render(source: &str, footer: &description::Footer<'_>) -> anyhow::Result<Option<String>> {
    let mut cache = observed(source)?;
    cache
        .get_shared(1, &Arc::from(source))?
        .render_footer("", footer, "")
}

fn normalised_human(text: &str) -> String {
    let mut expected = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\n' {
            expected.truncate(expected.trim_end_matches('\r').len());
        }
        expected.push(character);
    }
    expected
}

pub fn descriptions(data: &[u8]) {
    if data.len() > MAX_INPUT {
        return;
    }
    let arbitrary = String::from_utf8_lossy(data);
    let footer = description::Footer::default();
    if let Ok(Some(rendered)) = render(&arbitrary, &footer) {
        assert!(rendered.len() <= arbitrary.len() + 256);
        assert!(observed(&rendered).is_ok());
        assert_eq!(
            render(&rendered, &footer).expect("valid rendered description"),
            None
        );
    }

    // Reserved markers are not human input in this structured case. Escape only
    // those delimiters; all remaining Unicode, punctuation and whitespace survive.
    let human = arbitrary
        .replace(description::START, "&lt;!-- git-pr-stack-start -->")
        .replace(description::END, "&lt;!-- git-pr-stack-end -->");
    let before = normalised_human(&format!("Human prefix\n{human}\nEnd of prefix."));
    let after = normalised_human(&format!("Human suffix\n{human}\nEnd of suffix."));
    let source = format!(
        "{before}\n\n{}\nold managed content\n{}\n\n{after}",
        description::START,
        description::END
    );
    let commit = model::Commit {
        hash: "a".repeat(40),
        pr_number: u64::from(data.first().copied().unwrap_or(0)) + 1,
        ..model::Commit::default()
    };
    let renderer = description::Renderer::Compact(description::CompactRenderer::new([&commit]));
    let footer = renderer.footer(&commit);
    let managed = footer.parts().concat();
    let expected = format!(
        "{before}\n\n{}\n{managed}\n{}\n\n{after}",
        description::START,
        description::END
    );
    let rendered = render(&source, &footer)
        .expect("valid explicit managed markers")
        .expect("changed footer");
    assert_eq!(rendered, expected);
    assert_eq!(render(&rendered, &footer).expect("repeat rendering"), None);
    // Ambiguous delimiters must fail before editing any human text.
    let malformed = format!("{source}\n{}", description::START);
    assert!(observed(&malformed).is_err());
}
