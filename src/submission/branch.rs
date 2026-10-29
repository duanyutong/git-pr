//! Ordinary branch publication when no managed stack has been selected.

use anyhow::{Context, Result};
use std::collections::HashSet;

use crate::{
    cli::Flags,
    commands::Commands,
    config::{Config, checkpoint::Checkpoint},
    diagnostics::errors::redact_url_credentials,
    git::Git,
    identity::{resolve, store::Records},
    model::outcome::Operation,
};

use super::Application;

pub(super) fn candidate(
    config: &Config,
    flags: &Flags,
    commands: &Commands,
) -> Result<Option<String>> {
    if config.repo.jj
        || config.selection.explicit
        || requests_pull_requests(flags)
        || matches!(
            config.stop_after,
            Some(Checkpoint::Rewrite | Checkpoint::PrCreate)
        )
    {
        return Ok(None);
    }
    let branch = commands.git(&["branch", "--show-current"])?;
    if branch.is_empty() || branch == config.repo.trunk {
        return Ok(None);
    }
    Git::validate_remote_ref(&branch)?;
    let branches = resolve::inventory(commands)?;
    if !branches.contains_key(&branch) {
        return Ok(None);
    }
    let records = Records::read(commands)?;
    if records.0.contains_key("git-pr.pendingcreate") || associated(&records, &branch) {
        return Ok(None);
    }
    let git = Git {
        commands,
        repo: &config.repo,
    };
    let history = git.branch_history(
        &format!("refs/remotes/{}/{}", config.repo.remote, config.repo.trunk),
        "HEAD",
    )?;
    let hashes: HashSet<_> = history.iter().map(|commit| commit.hash.as_str()).collect();
    if history.iter().any(|commit| !commit.remote_ref().is_empty())
        || branches
            .iter()
            .any(|(name, oid)| name != &branch && hashes.contains(oid.as_str()))
    {
        return Ok(None);
    }
    if config.stop_after == Some(Checkpoint::GetCommits) {
        commands.out(&format!("Branch {branch}"));
        for commit in history.iter().rev() {
            commands.out(&commit.to_string());
        }
    }
    Ok(Some(branch))
}

fn associated(records: &Records, branch: &str) -> bool {
    [
        "gitprhead",
        "gitprpullrequest",
        "gitprchangeid",
        "github-pr-owner-number",
    ]
    .iter()
    .any(|field| records.0.contains_key(&format!("branch.{branch}.{field}")))
}

fn requests_pull_requests(flags: &Flags) -> bool {
    [
        "draft",
        "github-stack",
        "no-stack",
        "include-other-authors",
        "skip-draft",
        "include-draft",
        "draft-pattern",
        "add-label",
        "add-tip-label",
        "t",
        "compact-stack",
        "branch-from-title",
        "output",
    ]
    .iter()
    .any(|flag| flags.supplied.contains(*flag))
}

impl Application {
    pub(super) fn publish_branch(&self, config: &Config, branch: &str) -> Result<()> {
        self.commands.terminal.finish(true);
        if self.checkpoint(config, Checkpoint::GetCommits) {
            return Ok(());
        }
        self.commands.terminal.section(if config.dry_run {
            "Preview"
        } else {
            "Publication"
        });
        let reference = format!("refs/heads/{branch}");
        if config.dry_run {
            self.commands.out(&format!(
                "[DRY-RUN] Would push {branch} to {} and set its upstream",
                config.repo.remote
            ));
            return Ok(());
        }
        let _guard = self.lock_repository(config)?;
        self.stage(&format!("Pushing branch {branch}"), "Branch pushed");
        let operation = Operation::Branch(branch.into());
        self.commands.outcomes.start(operation.clone());
        let output = self
            .commands
            .git(&[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--set-upstream",
                "--",
                &config.repo.remote,
                &format!("{reference}:{reference}"),
            ])
            .context("branch push failed; inspect Git's diagnostic before retrying")?;
        self.commands.outcomes.confirm(operation);
        self.commands.terminal.finish(true);
        self.commands.out(&redact_url_credentials(&output));
        self.checkpoint(config, Checkpoint::Push);
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/branch.rs"]
mod unit;
