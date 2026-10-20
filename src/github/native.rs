//! Native topology reconciliation; Git publication is owned exclusively by Git.

use super::GitHub;
use crate::model::{Commit, PullRequest};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Topology {
    number: u64,
    pull_requests: Vec<Member>,
    base: crate::model::pull_request::Reference,
}

#[derive(Debug, Deserialize)]
struct Member {
    number: u64,
}

impl Topology {
    fn validate(&self, expected: u64, observed: &[Arc<PullRequest>]) -> Result<()> {
        if self.number != expected {
            bail!(
                "native stack observation returned stack #{} instead of requested #{expected}; no native stack changes were made",
                self.number
            );
        }
        if self.base.name.is_empty() || self.pull_requests.is_empty() {
            bail!(
                "incomplete native stack #{expected} observation: base or member list is empty; no native stack changes were made"
            );
        }
        let mut members = HashSet::new();
        for pr in &self.pull_requests {
            if pr.number == 0 || !members.insert(pr.number) {
                bail!(
                    "invalid native stack #{expected} member identity #{}; members must be non-zero and unique; no native stack changes were made",
                    pr.number
                );
            }
        }
        for pr in observed.iter().filter(|pr| pr.stack.is_some()) {
            if !members.contains(&pr.number) {
                bail!(
                    "inconsistent native stack #{expected} observation: PR #{} claims membership but is missing from the member list; refresh the stack before retrying",
                    pr.number
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackOutcome {
    Unchanged,
    Updated,
    Declined,
}

impl GitHub<'_> {
    pub(crate) fn check_transition_stack(
        &self,
        number: u64,
        selected: &[u64],
        claimed: &[u64],
    ) -> Result<()> {
        let topology = self.observe_topology(number, &[])?;
        if number == 0
            || topology
                .pull_requests
                .iter()
                .any(|pr| !selected.contains(&pr.number))
            || claimed
                .iter()
                .any(|number| !topology.pull_requests.iter().any(|pr| pr.number == *number))
        {
            bail!("native stack transition requires its complete membership");
        }
        Ok(())
    }

    pub(crate) fn unstack_transition(&self, number: u64) -> Result<()> {
        let result = self.request_url(
            "POST",
            &self.repo_url(&format!("stacks/{number}/unstack")),
            None,
        );
        // A lost acknowledgement is resolved by fresh membership observations.
        if let Ok(response) = result
            && response.status != 204
        {
            bail!("native stack transition is incomplete; some PRs remain stacked");
        }
        Ok(())
    }

    pub(crate) fn require_recoverable_membership(
        pr: &crate::model::pull_request::RecoveryPullRequest,
    ) -> Result<()> {
        if let Some(stack) = &pr.stack {
            bail!(
                "PR #{} belongs to native stack #{}; a separately verified native stack transition is required before repairing its base. Neither --yes nor --no-stack bypasses this requirement",
                pr.number,
                stack.number
            );
        }
        Ok(())
    }

    fn observe_topology(&self, number: u64, observed: &[Arc<PullRequest>]) -> Result<Topology> {
        let bytes = self.request("GET", &format!("stacks/{number}"), None)?;
        let topology: Topology =
            serde_json::from_slice(&bytes).context("invalid native stack observation")?;
        topology.validate(number, observed)?;
        Ok(topology)
    }

    pub(crate) fn manage_stack(&self, commits: &[Commit], base: &str) -> Result<StackOutcome> {
        let selected: Vec<_> = commits.iter().filter(|c| !c.skip).collect();
        if selected.len() < 2 {
            self.warn_blocked(commits);
            return Ok(StackOutcome::Unchanged);
        }
        let desired: Vec<_> = selected.iter().map(|c| c.pr_number).collect();
        if desired.contains(&0) {
            bail!("cannot link a stack without resolved PR identities");
        }
        if desired.iter().collect::<HashSet<_>>().len() != desired.len() {
            bail!("cannot link a native stack with duplicate PR identities");
        }
        let mut observed = Vec::with_capacity(desired.len());
        for chunk in desired.chunks(25) {
            self.refresh_prs(chunk)?;
            for &number in chunk {
                observed.push(self.get_with_stack(number)?);
            }
        }
        let stacks: BTreeSet<_> = observed
            .iter()
            .filter_map(|pr| pr.stack.as_ref().map(|s| s.number))
            .collect();
        if stacks.contains(&0) {
            bail!(
                "invalid native stack identity #0 in PR observation; no native stack changes were made"
            );
        }
        if stacks.len() > 1 {
            bail!(
                "selected PRs belong to multiple native stacks; resolve their membership before linking"
            );
        }
        let topology = stacks
            .first()
            .map(|&number| self.observe_topology(number, &observed))
            .transpose()?;
        let joined = selected
            .iter()
            .map(|c| c.remote_ref())
            .collect::<Vec<_>>()
            .join(" ");
        let current: Vec<_> = topology.as_ref().map_or_else(Vec::new, |t| {
            t.pull_requests.iter().map(|p| p.number).collect()
        });
        for (i, pr) in observed.iter().enumerate() {
            super::policy::require_publication_base(
                pr,
                if i == 0 {
                    base
                } else {
                    selected[i - 1].remote_ref()
                },
            )?;
        }
        let stack_base_agrees = topology
            .as_ref()
            .is_none_or(|t| current.first() != desired.first() || t.base.name == base);
        if stack_base_agrees
            && (current == desired || current.windows(desired.len()).any(|slice| slice == desired))
        {
            self.commands
                .terminal
                .detail(&format!("GitHub Stack unchanged: {joined}"));
            return Ok(StackOutcome::Unchanged);
        }
        if let Some(topology) = topology {
            if stack_base_agrees && desired.starts_with(&current) {
                self.request(
                    "POST",
                    &format!("stacks/{}/add", topology.number),
                    Some(&json!({"pull_requests":&desired[current.len()..]})),
                )?;
            } else if self.rebuild(&topology, &observed, &desired, base, &joined)?
                == StackOutcome::Declined
            {
                self.commands.out("skipped GitHub Stack rebuild");
                self.warn_blocked(commits);
                return Ok(StackOutcome::Declined);
            }
        } else {
            self.request("POST", "stacks", Some(&json!({"pull_requests":desired})))?;
        }
        self.commands
            .terminal
            .detail(&format!("GitHub Stack updated: {joined}"));
        Ok(StackOutcome::Updated)
    }
    fn rebuild(
        &self,
        topology: &Topology,
        observed: &[Arc<PullRequest>],
        desired: &[u64],
        base: &str,
        joined: &str,
    ) -> Result<StackOutcome> {
        // A selected slice cannot authorise dropping surrounding or retained members.
        let desired_set: HashSet<_> = desired.iter().copied().collect();
        if topology
            .pull_requests
            .iter()
            .any(|p| !desired_set.contains(&p.number))
        {
            bail!(
                "rebuilding stack #{} would remove unselected or retained PRs; submit the complete intended stack",
                topology.number
            );
        }
        self.commands.warn(&format!(
            "the existing GitHub stack #{} must be dissolved and rebuilt as: {joined}",
            topology.number
        ));
        if !self.confirm("Rebuild the GitHub stack now?") {
            return Ok(StackOutcome::Declined);
        }
        let url = format!(
            "https://api.{}/repos/{}/stacks/{}/unstack",
            self.config.repo.host, self.config.repo.name, topology.number
        );
        let remainder = self.request_url("POST", &url, None)?;
        if remainder.status != 204 {
            bail!(
                "stack #{} could not be dissolved: some PRs remain stacked",
                topology.number
            );
        }
        // Unstacking is a mutation boundary, not a renewal of the old PR snapshot.
        {
            let mut times = super::guard(&self.session.pr_times);
            for original in observed {
                times.remove(&original.number);
            }
            drop(times);
        }
        self.refresh_prs(desired)?;
        for (i, original) in observed.iter().enumerate() {
            let base = if i == 0 {
                base
            } else {
                observed[i - 1].head.name.as_str()
            };
            self.verify_base(
                &Commit {
                    pr_number: original.number,
                    ..Commit::default()
                },
                base,
            )?;
        }
        self.request("POST", "stacks", Some(&json!({"pull_requests":desired})))?;
        Ok(StackOutcome::Updated)
    }
}
