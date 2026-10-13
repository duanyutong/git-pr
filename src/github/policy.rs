//! Pull-request and label decisions without external execution.

use crate::git::policy::valid_oid;
use crate::model::{Commit, PullRequest};

pub(crate) fn require_open(pr: &PullRequest) -> anyhow::Result<()> {
    if pr.state != "open" || pr.is_merged() {
        anyhow::bail!(
            "PR #{} for head {:?} is {} (recorded base {:?}); refusing to publish or create a replacement. Inspect this PR and its actual merge target before changing the local stack; a merged PR need not have reached the trunk branch",
            pr.number,
            pr.head.name,
            if pr.is_merged() { "merged" } else { "closed" },
            pr.base.name,
        );
    }
    Ok(())
}

pub(crate) fn require_publication_base(pr: &PullRequest, base: &str) -> anyhow::Result<()> {
    require_open(pr)?;
    if pr.base.name != base {
        anyhow::bail!(
            "unsafe publication: PR #{} for head {:?} has base {:?}, but the local stack requires {base:?}. Publishing reordered branches before their PR relationships are safely transitioned can cause GitHub to mark PRs as merged. This base change has not been verified as recovery from a merged downstack; restore the published order or arrange a separately verified transition before retrying. Neither --yes nor --no-stack bypasses this check",
            pr.number,
            pr.head.name,
            pr.base.name,
        );
    }
    Ok(())
}

pub(crate) fn validate_recovery_observation(
    pr: &crate::model::pull_request::RecoveryPullRequest,
    number: u64,
    repository: &str,
) -> anyhow::Result<()> {
    if number == 0
        || pr.number != number
        || pr.node_id.is_empty()
        || !matches!(pr.state.as_str(), "open" | "closed")
        || [&pr.head, &pr.base].iter().any(|reference| {
            reference.name.is_empty()
                || !valid_oid(&reference.sha)
                || !reference.repo.full_name.eq_ignore_ascii_case(repository)
        })
    {
        anyhow::bail!("incomplete or unexpected recovery identity for PR #{number}");
    }
    Ok(())
}

pub(crate) fn require_recovery_open(
    pr: &crate::model::pull_request::RecoveryPullRequest,
) -> anyhow::Result<()> {
    if pr.state != "open" || pr.merged || pr.merged_at.is_some() {
        anyhow::bail!(
            "PR #{} is no longer open and unmerged; base recovery cannot reopen or replace it",
            pr.number
        );
    }
    Ok(())
}

pub(crate) fn recovery_merge(
    pr: &crate::model::pull_request::RecoveryPullRequest,
    reference: &str,
) -> anyhow::Result<String> {
    if pr.head.name != reference || pr.state != "closed" || !pr.merged {
        anyhow::bail!("obsolete base {reference:?} does not identify a merged downstack PR");
    }
    pr.merge_commit_sha
        .as_ref()
        .filter(|oid| valid_oid(oid))
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "merged downstack PR #{} has no verified merge result",
                pr.number
            )
        })
}

pub(crate) fn same_recovery_head(
    expected: &crate::model::pull_request::RecoveryPullRequest,
    actual: &crate::model::pull_request::RecoveryPullRequest,
    base: &str,
) -> anyhow::Result<()> {
    require_recovery_open(actual)?;
    if actual.number != expected.number
        || actual.node_id != expected.node_id
        || actual.head != expected.head
        || actual.base.name != base
        || actual.stack != expected.stack
    {
        anyhow::bail!(
            "PR #{} identity, head, base or native membership changed during base recovery",
            expected.number
        );
    }
    Ok(())
}

pub(crate) fn is_draft(
    title: &str,
    default: bool,
    patterns: &crate::stack::selection::DraftPatterns,
) -> bool {
    default || patterns.matches(title)
}

pub(crate) fn select_pr<'a>(
    prs: impl IntoIterator<Item = &'a PullRequest>,
    reference: &str,
) -> u64 {
    let mut first = None;
    for pr in prs {
        if pr.head.name != reference {
            continue;
        }
        if pr.state == "open" {
            return pr.number;
        }
        first.get_or_insert(pr.number);
    }
    first.unwrap_or(0)
}

pub(crate) fn label_changes(
    commit: &Commit,
    defaults: &[String],
    common: &[String],
    tip_labels: &[String],
    tip: bool,
) -> (Vec<String>, Vec<String>) {
    let mut add = commit.tags(defaults);
    add.extend(common.iter().cloned());
    if tip {
        add.extend(tip_labels.iter().cloned());
    }
    let mut required = std::collections::HashSet::new();
    add.retain(|label| required.insert(label.to_ascii_lowercase()));
    let remove = if tip {
        Vec::new()
    } else {
        tip_labels
            .iter()
            .filter(|label| required.insert(label.to_ascii_lowercase()))
            .cloned()
            .collect()
    };
    (add, remove)
}
