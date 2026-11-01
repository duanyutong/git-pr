//! Submission eligibility and reference invariants without external effects.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};

use crate::model::Commit;

pub(super) fn require_fresh_publication(
    now: Instant,
    observations: impl IntoIterator<Item = Instant>,
) -> Result<()> {
    if observations
        .into_iter()
        .any(|observed| now.saturating_duration_since(observed) > Duration::from_secs(30))
    {
        bail!("publication evidence expired before the next push; retry from fresh observations");
    }
    Ok(())
}

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
