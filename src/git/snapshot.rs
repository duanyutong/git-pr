//! Framed Git metadata and tree identities, independent of presentation settings.

use crate::{model::Commit, model::trailers};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;

pub(super) const FORMAT: &str = "--format=%H%x00%an%x00%ae%x00%aI%x00%T%x00%P%x00%B%x00";

#[derive(Debug)]
pub(super) struct Snapshot {
    pub commits: Vec<Commit>,
    pub trees: HashMap<String, String>,
    pub parents: HashMap<String, Vec<String>>,
}

impl Snapshot {
    pub(super) fn reject_merges(&self) -> Result<()> {
        if let Some(commit) = self.commits.iter().find(|c| {
            self.parents
                .get(&c.hash)
                .is_some_and(|parents| parents.len() > 1)
        }) {
            bail!(
                "merge commit {} is not supported; rebase the stack onto a linear history before submission",
                commit.short_hash()
            );
        }
        Ok(())
    }
}

pub(super) fn parse(output: &str) -> Result<Snapshot> {
    let mut records = output.split_terminator('\0');
    let mut commits = Vec::new();
    let mut trees = HashMap::new();
    let mut parents = HashMap::new();
    while let Some(hash) = records.next() {
        let hash = hash.trim_start_matches('\n');
        if hash.is_empty() {
            continue;
        }
        if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("invalid commit identity in Git metadata");
        }
        let author_name = records.next().context("missing author name")?;
        let author_email = records.next().context("missing author email")?;
        if author_name.is_empty() || author_email.is_empty() {
            bail!("missing author identity in Git metadata");
        }
        let date = records.next().context("missing author date")?;
        let tree = records.next().context("missing tree identity")?;
        let parent = records.next().context("missing parent identities")?;
        let body = records.next().context("missing commit message")?;
        let (title, body) = body.split_once('\n').unwrap_or((body, ""));
        let (message, attrs) = trailers::parse(&body.lines().collect::<Vec<_>>());
        commits.push(Commit {
            hash: hash.into(),
            author_name: author_name.into(),
            author_email: author_email.into(),
            date: Some(chrono::DateTime::parse_from_rfc3339(date)?.with_timezone(&chrono::Utc)),
            title: title.trim().into(),
            message,
            attrs,
            ..Commit::default()
        });
        trees.insert(hash.into(), tree.into());
        parents.insert(
            hash.into(),
            parent.split_whitespace().map(str::to_owned).collect(),
        );
    }
    Ok(Snapshot {
        commits,
        trees,
        parents,
    })
}

pub(super) fn linear_depths(
    commits: &[Commit],
    parents: &HashMap<String, Vec<String>>,
) -> HashMap<String, usize> {
    // Rank arithmetic is valid only for the complete, verified single-parent path.
    let mut depths = std::collections::HashMap::new();
    if let Some(base_oid) = commits
        .last()
        .and_then(|c| parents.get(&c.hash))
        .and_then(|p| if p.len() == 1 { p.first() } else { None })
    {
        let linear = commits.iter().enumerate().all(|(i, c)| {
            let parent = commits
                .get(i + 1)
                .map_or(base_oid.as_str(), |c| c.hash.as_str());
            parents
                .get(&c.hash)
                .is_some_and(|p| p.as_slice() == [parent])
        });
        if linear {
            depths.insert(base_oid.to_owned(), commits.len());
            depths.extend(commits.iter().enumerate().map(|(i, c)| (c.hash.clone(), i)));
        }
    }
    depths
}
