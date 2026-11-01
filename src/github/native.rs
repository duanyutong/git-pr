//! Native topology reconciliation; Git publication is owned exclusively by Git.

use super::{
    GitHub,
    policy::{self, NativeAction},
};
use crate::model::{Commit, PullRequest};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct Topology {
    #[serde(skip, default = "Instant::now")]
    acquired: Instant,
    number: u64,
    pull_requests: Vec<Member>,
    base: crate::model::pull_request::Reference,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
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
    pub(crate) fn check_stack_eligibility(
        &self,
        commits: &[Commit],
        base: &str,
        restoring: bool,
    ) -> Result<()> {
        self.check_stack_eligibility_with_budget(commits, base, restoring, None)
    }

    pub(crate) fn check_stack_eligibility_with_budget(
        &self,
        commits: &[Commit],
        base: &str,
        restoring: bool,
        started: Option<Instant>,
    ) -> Result<()> {
        let selected: Vec<_> = commits.iter().filter(|commit| !commit.skip).collect();
        if selected.len() < 2 {
            return Ok(());
        }
        let mut observed = Vec::new();
        let mut desired = Vec::new();
        for commit in selected {
            let pr = self.publication_pr(commit.remote_ref())?;
            desired.push(pr.as_ref().map_or(0, |pr| pr.number));
            if let Some(pr) = pr {
                observed.push(pr);
            }
        }
        self.refresh_prs_with_budget(
            &observed.iter().map(|pr| pr.number).collect::<Vec<_>>(),
            started,
        )?;
        for pr in &mut observed {
            *pr = self.get_with_budget(pr.number, started)?;
            policy::require_open(pr)?;
        }
        // Eligible observations cannot block any native operation; avoid topology
        // requests solely for preflight when no eligibility decision needs them.
        if observed.iter().all(|pr| pr.auto_merge == Some(false)) {
            return Ok(());
        }
        let action = if restoring {
            NativeAction::Rebuild
        } else {
            for pr in &mut observed {
                *pr = self.get_with_stack(pr.number, started)?;
            }
            let topology = self.selected_topology(&observed, started)?;
            Self::native_plan(topology.as_ref(), &desired, base)
        };
        Self::check_native_plan(action, &desired, &observed)
    }

    fn selected_topology(
        &self,
        observed: &[Arc<PullRequest>],
        started: Option<Instant>,
    ) -> Result<Option<Topology>> {
        let stacks: BTreeSet<_> = observed
            .iter()
            .filter_map(|pr| pr.stack.as_ref().map(|stack| stack.number))
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
        stacks
            .first()
            .map(|&number| self.observe_topology(number, observed, started))
            .transpose()
    }

    fn native_plan(topology: Option<&Topology>, desired: &[u64], base: &str) -> NativeAction {
        let current: Vec<_> = topology.map_or_else(Vec::new, |topology| {
            topology.pull_requests.iter().map(|pr| pr.number).collect()
        });
        let base_agrees = topology.is_none_or(|topology| {
            current.first() != desired.first() || topology.base.name == base
        });
        policy::native_action(desired, &current, base_agrees)
    }

    fn check_native_plan(
        action: NativeAction,
        desired: &[u64],
        observed: &[Arc<PullRequest>],
    ) -> Result<()> {
        let affected = match action {
            NativeAction::Unchanged => return Ok(()),
            NativeAction::Append(start) => &desired[start..],
            NativeAction::Create | NativeAction::Rebuild => desired,
        };
        policy::require_native_eligibility(
            observed
                .iter()
                .filter(|pr| affected.contains(&pr.number))
                .map(AsRef::as_ref),
        )
    }

    fn native_request(
        &self,
        path: &str,
        body: Option<&serde_json::Value>,
        acquired: Instant,
        started: Instant,
    ) -> Result<Vec<u8>> {
        use crate::model::outcome::Operation;
        self.commands.outcomes.start(Operation::NativeStack);
        let encoded = body.map(serde_json::Value::to_string);
        self.request_url_encoded(
            "POST",
            &self.repo_url(path),
            encoded.as_deref().map(str::as_bytes),
            1,
            Some(acquired),
            Some(started),
        )
        .map(|response| response.body)
        .inspect(|_| self.commands.outcomes.confirm(Operation::NativeStack))
        .map_err(|error| {
            if super::limits::rejected(&error) {
                self.commands.outcomes.reject(Operation::NativeStack);
            }
            let number = error
                .downcast_ref::<super::http::HttpStatusError>()
                .filter(|status| status.status.as_u16() == 422)
                .and_then(|status| serde_json::from_str::<serde_json::Value>(&status.body).ok())
                .and_then(|body| {
                    body["message"]
                        .as_str()?
                        .strip_prefix("Pull request #")?
                        .strip_suffix(" has auto-merge enabled")?
                        .parse::<u64>()
                        .ok()
                });
            if let Some(number) = number {
                error.context(policy::NativeEligibility {
                    enabled: vec![number],
                    unknown: Vec::new(),
                })
            } else {
                error
            }
        })
    }

    pub(crate) fn check_transition_stack(
        &self,
        number: u64,
        selected: &[u64],
        claimed: &[u64],
        started: Option<Instant>,
    ) -> Result<Instant> {
        let topology = self.observe_topology(number, &[], started)?;
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
        Ok(topology.acquired)
    }

    pub(crate) fn unstack_transition(
        &self,
        number: u64,
        acquired: Instant,
        started: Instant,
    ) -> Result<()> {
        let operation = crate::model::outcome::Operation::NativeStack;
        self.commands.outcomes.start(operation.clone());
        let result = self.request_url_encoded(
            "POST",
            &self.repo_url(&format!("stacks/{number}/unstack")),
            None,
            1,
            Some(acquired),
            Some(started),
        );
        match result {
            Ok(response) => {
                self.commands.outcomes.confirm(operation);
                if response.status != 204 {
                    bail!("native stack transition is incomplete; some PRs remain stacked");
                }
            }
            Err(error) if super::limits::rejected(&error) => {
                self.commands.outcomes.reject(operation);
                return Err(error);
            }
            // A lost acknowledgement is resolved by fresh membership observations.
            Err(_) => {}
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

    fn observe_topology(
        &self,
        number: u64,
        observed: &[Arc<PullRequest>],
        started: Option<Instant>,
    ) -> Result<Topology> {
        let response = self.request_url_with_budget(
            "GET",
            &self.repo_url(&format!("stacks/{number}")),
            None,
            started,
        )?;
        let mut topology: Topology =
            serde_json::from_slice(&response.body).context("invalid native stack observation")?;
        topology.acquired = response.acquired;
        topology.validate(number, observed)?;
        Ok(topology)
    }

    pub(crate) fn manage_stack(&self, commits: &[Commit], base: &str) -> Result<StackOutcome> {
        let started = self.now();
        let mut attempt = 0;
        loop {
            let result = self.manage_stack_once(commits, base, started);
            let Err(error) = result else { return result };
            let Some((delay, reason)) = super::limits::deferral(&error, attempt) else {
                return Err(error);
            };
            if attempt == super::limits::RETRIES {
                return Err(
                    error.context("native reconciliation exhausted its observation refreshes")
                );
            }
            self.wait_for_recovery(delay, reason.label(), started)?;
            self.invalidate_heads(
                &commits
                    .iter()
                    .filter(|commit| !commit.skip)
                    .map(|commit| commit.remote_ref().to_owned())
                    .collect::<Vec<_>>(),
            );
            attempt += 1;
        }
    }

    fn manage_stack_once(
        &self,
        commits: &[Commit],
        base: &str,
        started: Instant,
    ) -> Result<StackOutcome> {
        super::limits::check_window(
            self.now(),
            std::time::Duration::ZERO,
            super::limits::WaitReason::Mutation,
            None,
            Some(started),
        )?;
        let selected: Vec<_> = commits.iter().filter(|c| !c.skip).collect();
        if selected.len() < 2 {
            self.warn_blocked(commits);
            self.commands
                .terminal
                .outcome("Native linking not needed: fewer than two active PRs");
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
            self.refresh_prs_with_budget(chunk, Some(started))?;
            for &number in chunk {
                observed.push(self.get_with_stack(number, Some(started))?);
            }
        }
        let topology = self.selected_topology(&observed, Some(started))?;
        super::limits::check_window(
            self.now(),
            std::time::Duration::ZERO,
            super::limits::WaitReason::Mutation,
            None,
            Some(started),
        )?;
        let joined = selected
            .iter()
            .map(|c| c.remote_ref())
            .collect::<Vec<_>>()
            .join(" ");
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
        let action = Self::native_plan(topology.as_ref(), &desired, base);
        Self::check_native_plan(action, &desired, &observed)?;
        if action == NativeAction::Unchanged {
            self.commands
                .terminal
                .detail(&format!("GitHub Stack unchanged: {joined}"));
            self.commands
                .terminal
                .outcome("Native stack already up to date");
            return Ok(StackOutcome::Unchanged);
        }
        let acquired = self.acquisition_time(desired.iter().copied())?;
        let acquired = topology
            .as_ref()
            .map_or(acquired, |topology| acquired.min(topology.acquired));
        if let Some(topology) = topology {
            if let NativeAction::Append(start) = action {
                self.native_request(
                    &format!("stacks/{}/add", topology.number),
                    Some(&json!({"pull_requests":&desired[start..]})),
                    acquired,
                    started,
                )?;
            } else if self.rebuild(
                &topology, &observed, &desired, base, &joined, acquired, started,
            )? == StackOutcome::Declined
            {
                self.commands.out("skipped GitHub Stack rebuild");
                self.commands
                    .terminal
                    .outcome("Native stack rebuild skipped");
                self.warn_blocked(commits);
                return Ok(StackOutcome::Declined);
            }
        } else {
            self.native_request(
                "stacks",
                Some(&json!({"pull_requests":desired})),
                acquired,
                started,
            )?;
        }
        self.commands
            .terminal
            .detail(&format!("GitHub Stack updated: {joined}"));
        self.commands.terminal.outcome("Native stack linked");
        Ok(StackOutcome::Updated)
    }
    fn rebuild(
        &self,
        topology: &Topology,
        observed: &[Arc<PullRequest>],
        desired: &[u64],
        base: &str,
        joined: &str,
        acquired: Instant,
        started: Instant,
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
        let acquired = if !self.config.auto_accept || !self.fresh(&acquired) {
            self.recheck_rebuild(topology, observed, desired, started)?
        } else {
            acquired
        };
        let url = format!(
            "https://api.{}/repos/{}/stacks/{}/unstack",
            self.config.repo.host, self.config.repo.name, topology.number
        );
        self.commands
            .outcomes
            .start(crate::model::outcome::Operation::NativeStack);
        let remainder = self
            .request_url_encoded("POST", &url, None, 1, Some(acquired), Some(started))
            .inspect_err(|error| {
                if super::limits::rejected(error) {
                    self.commands
                        .outcomes
                        .reject(crate::model::outcome::Operation::NativeStack);
                }
            })?;
        self.commands
            .outcomes
            .confirm(crate::model::outcome::Operation::NativeStack);
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
        self.refresh_prs_with_budget(desired, Some(started))?;
        let refreshed = desired
            .iter()
            .map(|number| self.get_with_budget(*number, Some(started)))
            .collect::<Result<Vec<_>>>()?;
        policy::require_native_eligibility(refreshed.iter().map(AsRef::as_ref))?;
        for (i, (original, actual)) in observed.iter().zip(&refreshed).enumerate() {
            if actual.number != original.number
                || actual.node_id != original.node_id
                || actual.head != original.head
                || actual.stack.is_some()
            {
                bail!("native rebuild identity or membership changed after dissolution");
            }
            let base = if i == 0 {
                base
            } else {
                observed[i - 1].head.name.as_str()
            };
            policy::require_publication_base(actual, base)?;
        }
        self.acquisition_time(desired.iter().copied())
            .and_then(|acquired| {
                self.native_request(
                    "stacks",
                    Some(&json!({"pull_requests":desired})),
                    acquired,
                    started,
                )
            })
            .map(|_| StackOutcome::Updated)
    }

    fn recheck_rebuild(
        &self,
        original: &Topology,
        observed: &[Arc<PullRequest>],
        desired: &[u64],
        started: Instant,
    ) -> Result<Instant> {
        let references = observed
            .iter()
            .map(|pr| pr.head.name.clone())
            .collect::<Vec<_>>();
        self.invalidate_heads(&references);
        self.observe_with_budget(&references, started)?;
        let current = observed
            .iter()
            .map(|before| {
                let actual = self
                    .publication_pr(&before.head.name)?
                    .context("native rebuild member disappeared after confirmation")?;
                if actual.number != before.number
                    || actual.node_id != before.node_id
                    || actual.head != before.head
                    || actual.base != before.base
                    || actual.stack != before.stack
                {
                    bail!("native rebuild identity or relationships changed after confirmation");
                }
                Ok(actual)
            })
            .collect::<Result<Vec<_>>>()?;
        Self::check_native_plan(NativeAction::Rebuild, desired, &current)?;
        let topology = self.observe_topology(original.number, &current, Some(started))?;
        if topology.base != original.base || topology.pull_requests != original.pull_requests {
            bail!("native rebuild membership or base changed after confirmation");
        }
        self.acquisition_time(desired.iter().copied())
            .map(|acquired| topology.acquired.min(acquired))
    }
}
