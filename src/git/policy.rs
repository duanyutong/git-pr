//! Deterministic interpretation and acceptance of Git values.

use anyhow::{Result, bail};
use regex::Regex;

use crate::model::Repository;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitDisposition {
    Keep,
    SkipEmpty,
    SkipUntitledEmpty,
    RejectUntitled,
}

pub(crate) const fn classify_commit(title: &str, empty: bool) -> CommitDisposition {
    match (title.is_empty(), empty) {
        (false, false) => CommitDisposition::Keep,
        (false, true) => CommitDisposition::SkipEmpty,
        (true, true) => CommitDisposition::SkipUntitledEmpty,
        (true, false) => CommitDisposition::RejectUntitled,
    }
}

pub(crate) const REMOTE_PATTERNS: [&str; 2] = [
    r"^(\S+)\s+(git@([^:\s]+):([^/\s]+)/([^/\s]+?)(?:\.git)?)(?:\s+\((?:push|fetch)\))?$",
    r"^(\S+)\s+(https://(?:[^/\s?#]*@)?([^/@\s?#]+)/([^/\s?#]+)/([^/\s?#]+?)(?:\.git)?)(?:\s+\((?:push|fetch)\))?$",
];

/// Select one publication destination before interpreting its repository URL.
pub(crate) fn select_remote<'a>(output: &'a str, configured: Option<&str>) -> Result<&'a str> {
    let names: std::collections::BTreeSet<_> = output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    let name = match configured {
        Some(name) if names.contains(name) => name,
        Some(name) => bail!(
            "configured publication remote {:?} is not a named remote; choose an existing remote with `git config remote.pushDefault <remote>` or correct the current branch's pushRemote/remote setting",
            crate::diagnostics::errors::redact_url_credentials(name)
        ),
        None if names.contains("origin") => "origin",
        None if names.len() == 1 => names.iter().next().copied().unwrap_or_default(),
        None => bail!(
            "no unambiguous publication remote; inspect `git remote` and configure a destination with `git config remote.pushDefault <remote>`"
        ),
    };
    let selected: Vec<_> = output
        .lines()
        .filter(|line| line.split_whitespace().next() == Some(name))
        .collect();
    let pushes: Vec<_> = selected
        .iter()
        .copied()
        .filter(|line| line.trim_end().ends_with("(push)"))
        .collect();
    if pushes.len() > 1 {
        bail!(
            "publication remote {name:?} has multiple push URLs; configure exactly one push destination before submitting PRs"
        );
    }
    Ok(pushes
        .first()
        .copied()
        .or_else(|| selected.first().copied())
        .unwrap_or_default())
}

pub(crate) fn parse_remote_patterns(output: &str, patterns: &[&str]) -> Result<Repository> {
    let patterns = patterns
        .iter()
        .map(|pattern| Regex::new(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    parse_remote(output, &patterns)
}

pub(crate) fn parse_remote(output: &str, patterns: &[Regex]) -> Result<Repository> {
    let line = output
        .lines()
        .find(|line| line.trim_end().ends_with("(push)"))
        .or_else(|| output.lines().next())
        .unwrap_or("")
        .trim();
    for pattern in patterns {
        if let Some(m) = pattern.captures(line) {
            let fields = [1, 3, 4, 5].map(|index| m.get(index).map(|value| value.as_str()));
            let [Some(remote), Some(host), Some(owner), Some(name)] = fields else {
                bail!("remote pattern must capture remote, host, owner and repository");
            };
            return Ok(Repository {
                remote: remote.into(),
                host: host.into(),
                name: format!("{owner}/{name}"),
                ..Repository::default()
            });
        }
    }
    bail!(
        "ERROR: failed to parse remote url:\n  expect git@<host>:<user>/<repo> or https://<host>/<user>/<repo>\n  HTTPS user information must percent-encode whitespace, '/', '?' and '#'; query strings and fragments are not supported\n  got {:?}",
        crate::diagnostics::errors::redact_url_credentials(output)
    )
}

pub(crate) fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

pub(crate) fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn protect_trunk(reference: &str, trunk: &str) -> Result<()> {
    if reference == trunk {
        bail!(
            "refusing to publish directly to the remote trunk {trunk:?}; use a separate PR branch"
        );
    }
    Ok(())
}

/// Validate a literal branch name, without expanding checkout-history expressions.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Git reserves the exact case-sensitive .lock suffix"
)]
pub(crate) fn valid_branch_name(name: &str) -> bool {
    // Equivalent to Git's branch rules for literal names, including HEAD and '-'.
    // Remote-Ref is a publication destination, so @{-n} must never be expanded.
    !name.starts_with('-')
        && name != "HEAD"
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name
            .bytes()
            .any(|b| b <= b' ' || b == 127 || b"~^:?*[\\".contains(&b))
        && name
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}
