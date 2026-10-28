use crate::{
    config::checkpoint::Checkpoint,
    git::policy::{CommitDisposition, classify_commit},
    github::policy::is_draft,
    model::Commit,
    submission::policy::{duplicate_ref, mark_other_authors},
};

#[test]
fn commit_acceptance_distinguishes_missing_titles_and_empty_changes() {
    for (title, empty, expected) in [
        ("Feature", false, CommitDisposition::Keep),
        ("Feature", true, CommitDisposition::SkipEmpty),
        ("", true, CommitDisposition::SkipUntitledEmpty),
        ("", false, CommitDisposition::RejectUntitled),
    ] {
        assert_eq!(classify_commit(title, empty), expected);
    }
}

#[test]
fn draft_creation_respects_defaults_and_case_insensitive_patterns() {
    let patterns =
        crate::stack::selection::DraftPatterns::new(&["wip:*".into(), "draft:*".into()]).unwrap();
    for (title, default, expected) in [
        ("Feature", false, false),
        ("Feature", true, true),
        ("WIP: feature", false, true),
        ("draft: feature", false, true),
    ] {
        assert_eq!(is_draft(title, default, &patterns), expected);
    }
    assert!(!is_draft(
        "WIP: feature",
        false,
        &crate::stack::selection::DraftPatterns::new(&[]).unwrap()
    ));
}

#[test]
fn author_exclusion_preserves_existing_skips_and_reports_each_foreign_author() {
    let commits = vec![
        Commit {
            author_email: "me@example.com".into(),
            ..Commit::default()
        },
        Commit {
            author_email: "other@example.com".into(),
            ..Commit::default()
        },
        Commit {
            skip: true,
            ..Commit::default()
        },
        Commit {
            author_email: "me@example.com".into(),
            skip: true,
            ..Commit::default()
        },
    ];
    let mut included = commits.clone();
    assert_eq!(
        mark_other_authors(&mut included, "me@example.com", true),
        Vec::<usize>::new()
    );
    assert_eq!(included, commits);
    let mut excluded = commits;
    assert_eq!(
        mark_other_authors(&mut excluded, "me@example.com", false),
        [1, 2]
    );
    assert_eq!(
        excluded.iter().map(|c| c.skip).collect::<Vec<_>>(),
        [false, true, true, true]
    );
    assert_eq!(
        mark_other_authors(&mut [], "me@example.com", false),
        Vec::<usize>::new()
    );
}

#[test]
fn duplicate_references_ignore_absence_and_reject_the_first_collision_even_when_skipped() {
    let mut first = Commit {
        hash: "first".into(),
        ..Commit::default()
    };
    first.set_attr("remote-ref", "topic");
    let mut second = first.clone();
    second.hash = "second".into();
    second.skip = true;
    let unique = [Commit::default(), first.clone()];
    assert!(duplicate_ref(&unique).is_none());
    assert!(duplicate_ref(&[]).is_none());
    let commits = [Commit::default(), first, second.clone(), second];
    let (previous, current) = duplicate_ref(&commits).unwrap();
    assert_eq!(
        (previous.hash.as_str(), current.hash.as_str()),
        ("first", "second")
    );
}

#[test]
fn checkpoints_parse_once_and_preserve_their_external_names() {
    assert_eq!(Checkpoint::parse("").unwrap(), None);
    for (name, expected) in [
        ("validate", Checkpoint::Validate),
        ("get-commits", Checkpoint::GetCommits),
        ("rewrite", Checkpoint::Rewrite),
        ("push", Checkpoint::Push),
        ("pr-create", Checkpoint::PrCreate),
    ] {
        assert_eq!(Checkpoint::parse(name).unwrap(), Some(expected));
        assert_eq!(expected.as_str(), name);
    }
    assert_eq!(
        Checkpoint::parse("unknown").unwrap_err().to_string(),
        "ERROR: invalid --stop-after \"unknown\"; must be one of: validate, get-commits, prepare, rewrite, push, pr-create"
    );
}
#[test]
fn native_actions_distinguish_noops_creation_extension_and_rebuilding() {
    use crate::github::policy::{NativeAction, native_action};
    for (desired, current, base, expected) in [
        (vec![], vec![], true, NativeAction::Unchanged),
        (vec![1], vec![], true, NativeAction::Unchanged),
        (vec![1, 2], vec![], true, NativeAction::Create),
        (vec![1, 2], vec![1, 2], true, NativeAction::Unchanged),
        (vec![2, 3], vec![1, 2, 3, 4], true, NativeAction::Unchanged),
        (vec![1, 2, 3], vec![1, 2], true, NativeAction::Append(2)),
        (vec![2, 3], vec![1, 2], true, NativeAction::Append(1)),
        (vec![2, 3, 4], vec![1, 2, 3], true, NativeAction::Append(2)),
        (vec![2, 3], vec![1, 2], false, NativeAction::Rebuild),
        (vec![2, 4], vec![1, 2, 3], true, NativeAction::Rebuild),
        (vec![3, 2, 4], vec![1, 2, 3], true, NativeAction::Rebuild),
        (vec![2, 3, 1], vec![1, 2], true, NativeAction::Rebuild),
        (vec![3, 4], vec![1, 2], true, NativeAction::Rebuild),
        (vec![2, 1], vec![1, 2], true, NativeAction::Rebuild),
        (vec![1, 2], vec![1, 2], false, NativeAction::Rebuild),
    ] {
        assert_eq!(native_action(&desired, &current, base), expected);
    }
}
