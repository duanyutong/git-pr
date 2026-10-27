//! Publication identity is independent of commit messages and mutable revisions.

pub(crate) mod resolve;
pub(crate) mod store;
mod types;

pub(crate) use types::{Binding, Head};

use crate::{git::Git, model::Repository};
use anyhow::{Context, Result, bail};

impl Head {
    pub(crate) fn new(repo: &Repository, reference: &str) -> Result<Self> {
        Git::validate_remote_ref(reference)?;
        Ok(Self {
            repository: repository(repo),
            reference: reference.into(),
        })
    }

    pub(crate) fn parse(value: &str, repo: &Repository) -> Result<Self> {
        let (name, reference) = value
            .split_once(' ')
            .context("invalid gitPrHead: expected repository and full head ref")?;
        if !name.eq_ignore_ascii_case(&repository(repo)) {
            bail!(
                "publication repository differs from {}; explicitly associate the intended repository",
                repository(repo)
            );
        }
        Self::new(
            repo,
            reference
                .strip_prefix("refs/heads/")
                .context("gitPrHead must use refs/heads/")?,
        )
    }

    pub(crate) fn value(&self) -> String {
        format!("{} refs/heads/{}", self.repository, self.reference)
    }

    pub(crate) fn matches(&self, value: &str) -> bool {
        value
            .split_once(' ')
            .is_some_and(|(repository, reference)| {
                repository.eq_ignore_ascii_case(&self.repository)
                    && reference.strip_prefix("refs/heads/") == Some(self.reference.as_str())
            })
    }
}

pub(crate) fn repository(repo: &Repository) -> String {
    format!("{}/{}", repo.host, repo.name).to_ascii_lowercase()
}

pub(crate) fn review_url(repo: &Repository, number: u64) -> String {
    format!("https://{}/pull/{number}", repository(repo))
}

pub(crate) fn parse_review(value: &str, repo: &Repository) -> Result<u64> {
    let (prefix, number) = value
        .rsplit_once('/')
        .context("invalid gitPrPullRequest URL")?;
    if !prefix.eq_ignore_ascii_case(&format!("https://{}/pull", repository(repo))) {
        bail!("PR association is outside the selected review repository");
    }
    let parsed = number.parse::<u64>().context("invalid PR number")?;
    if parsed == 0 || parsed.to_string() != number {
        bail!("invalid canonical PR number {number:?}");
    }
    Ok(parsed)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attempt {
    pub id: String,
    pub head: Head,
    pub base: String,
}

impl Attempt {
    pub(crate) fn parse(value: &str, repo: &Repository) -> Result<Self> {
        let fields: Vec<_> = value.split(' ').collect();
        let ["v1", id, head_repo, head_ref, review_repo, base] = fields.as_slice() else {
            bail!("invalid or unsupported git-pr.pendingCreate record");
        };
        if id.len() != 36
            || !id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            bail!("invalid pending creation attempt identifier");
        }
        if !review_repo.eq_ignore_ascii_case(&repository(repo)) {
            bail!("pending creation belongs to a different review repository");
        }
        let head = Head::parse(&format!("{head_repo} {head_ref}"), repo)?;
        let base = base
            .strip_prefix("refs/heads/")
            .context("invalid pending creation base")?;
        Git::validate_remote_ref(base)?;
        Ok(Self {
            id: (*id).into(),
            head,
            base: base.into(),
        })
    }

    pub(crate) fn value(&self) -> String {
        format!(
            "v1 {} {} {} refs/heads/{}",
            self.id,
            self.head.value(),
            self.head.repository,
            self.base
        )
    }
}

#[cfg(test)]
#[path = "../tests/unit/identity.rs"]
mod unit;
