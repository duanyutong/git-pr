//! Submission eligibility and reference invariants without external effects.

use std::collections::BTreeMap;

use crate::model::Commit;

pub(crate) fn mark_other_authors(
    commits: &mut [Commit],
    email: &str,
    include_other_authors: bool,
) -> Vec<usize> {
    let mut excluded = Vec::new();
    for (index, commit) in commits.iter_mut().enumerate() {
        if commit.author_email != email && !include_other_authors {
            commit.skip = true;
            excluded.push(index);
        }
    }
    excluded
}

pub(crate) fn duplicate_ref(commits: &[Commit]) -> Option<(&Commit, &Commit)> {
    let mut seen = BTreeMap::new();
    for commit in commits {
        if commit.remote_ref().is_empty() {
            continue;
        }
        if let Some(previous) = seen.insert(commit.remote_ref(), commit) {
            return Some((previous, commit));
        }
    }
    None
}
