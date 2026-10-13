//! Repository rewriting and recovery of selected identities.

use anyhow::{Context, Result, bail};

use crate::{
    commands::{Clock, Commands},
    config::Config,
    diagnostics::errors::Fatal,
    git::Git,
    jj,
    model::REMOTE_REF,
    stack::selection,
};

use super::State;

impl State {
    pub(super) fn rewrite(
        &mut self,
        config: &Config,
        commands: &Commands,
        _clock: &dyn Clock,
        preflight: &mut dyn FnMut(&[crate::model::Commit], usize) -> Result<()>,
    ) -> Result<()> {
        let git = Git {
            commands,
            repo: &config.repo,
        };
        self.validate_conflicts()?;
        self.validate_refs(&config.repo.trunk)?;
        if !self
            .selected()
            .iter()
            .any(|c| !c.skip && c.remote_ref().is_empty())
        {
            return preflight(&self.stack, self.start);
        }
        let branches = commands
            .git(&["branch", "--format=%(refname:strip=2)|%(objectname)"])
            .context("failed to observe local branches")?;
        let names = selection::local_branches(&branches);
        let mut mappings = Vec::new();
        for i in self.start..self.stack.len() {
            let commit = &self.stack[i];
            if commit.skip || !commit.remote_ref().is_empty() {
                continue;
            }
            let branch = names.get(commit.hash.as_str()).map_or_else(
                || git.local_branch(commit, &branches),
                |name| (*name).to_owned(),
            );
            if branch.is_empty() {
                commands.out(&format!("❌ ERROR: commit {} is not on any local branch\n   Title: {}\n   Expected git-branchless to create a branch for this commit.", commit.short_hash(), commit.title));
                bail!("commit {} is not on any local branch", commit.short_hash());
            }
            mappings.push((i, branch));
        }
        for (i, branch) in &mappings {
            if !self.validated_refs.contains(branch) {
                crate::git::policy::protect_trunk(branch, &config.repo.trunk)?;
                Git::validate_remote_ref(branch)?;
                self.validated_refs.insert(branch.clone());
            }
            self.stack[*i].set_attr(REMOTE_REF, branch);
        }
        super::presentation::validate_duplicates(self.selected(), commands)?;
        let recovery_ref = if config.selection.explicit && !config.repo.jj {
            names.get(self.tip.as_str()).copied()
        } else {
            None
        };
        if config.selection.explicit
            && !config.repo.jj
            && recovery_ref.is_none()
            && config.selection.selected.is_empty()
        {
            self.tip_depth = git.depth(&self.tip, "HEAD")?;
            // Verify the exact position required by first-parent recovery.
            if git
                .at_depth("HEAD", self.tip_depth)
                .context("failed to verify selected tip in the current checkout")?
                != self.tip
            {
                bail!(
                    "selected tip is not at the expected position in the current checkout; check out its stack before rewriting"
                );
            }
        }

        preflight(&self.stack, self.start)?;
        if config.dry_run {
            for (i, branch) in &mappings {
                commands.out(&format!("[DRY-RUN] Would insert Remote-Ref: {branch} into {} (published identity determined by rewrite)", self.stack[*i].short_hash()));
            }
            return Ok(());
        }
        git.reword_all(
            &mappings
                .iter()
                .map(|(i, _)| &self.stack[*i])
                .collect::<Vec<_>>(),
        )?;
        self.recover_rewrite(config, commands, git, recovery_ref)
    }

    fn recover_rewrite(
        &mut self,
        config: &Config,
        commands: &Commands,
        git: Git<'_>,
        recovery_ref: Option<&str>,
    ) -> Result<()> {
        let head = git.head()?;
        // Rewriting selected descendants cannot change their excluded base.
        if config.selection.explicit {
            self.tip = if let Some(reference) = recovery_ref {
                git.resolve(&format!("refs/heads/{reference}"))
                    .context(Fatal)?
            } else {
                selection::recover_tip(
                    config.repo.jj,
                    &self.tip_change_id,
                    &head,
                    self.tip_depth,
                    |id| jj::resolve_revision(commands, id),
                    |head, depth| git.at_depth(head, depth),
                )
                .context(Fatal)?
            };
        } else {
            self.tip.clone_from(&head);
        }
        let observation = git
            .observe_stack(&self.trunk, &self.tip, !config.selection.explicit)
            .context(Fatal)?;
        self.observed_tip = observation.tip_hash;
        self.stack = observation.commits;
        let depths = observation.depths;
        self.start = if config.selection.explicit {
            selection::slice_start(&self.stack, &self.base, &self.trunk_hash, &depths)
        } else {
            0
        };
        if self.start == self.stack.len() {
            bail!("selected range disappeared after rewriting");
        }
        if !config.selection.selected.is_empty() {
            let selected = if config.repo.jj {
                let identities: std::collections::HashMap<_, _> = self
                    .stack
                    .iter()
                    .map(|c| (c.change_id.as_str(), c.hash.as_str()))
                    .collect();
                if identities.len() != self.stack.len() {
                    bail!("ambiguous change identities after rewriting");
                }
                self.selected_changes
                    .iter()
                    .map(|id| {
                        identities
                            .get(id.as_str())
                            .map(|hash| (*hash).to_owned())
                            .context("selected change disappeared after rewriting")
                    })
                    .collect::<Result<Vec<_>>>()
                    .context(Fatal)?
            } else if depths.is_empty() {
                // Unverified histories cannot supply ranks; bound fallback process count.
                git.at_depths(&head, &self.selected_depths).context(Fatal)?
            } else {
                let identities: std::collections::HashMap<_, _> = depths
                    .into_iter()
                    .map(|(hash, depth)| (depth, hash))
                    .collect();
                self.selected_depths
                    .iter()
                    .map(|depth| {
                        depth
                            .checked_sub(self.tip_depth)
                            .and_then(|depth| identities.get(&depth))
                            .cloned()
                            .context("selected commit disappeared after rewriting")
                    })
                    .collect::<Result<Vec<_>>>()
                    .context(Fatal)?
            };
            let _ = selection::mark_unselected(self.selected_mut(), &selected);
        }
        let _ = selection::mark_drafts(
            self.selected_mut(),
            config.skip_draft,
            config.include_draft,
            &config.draft_patterns,
        );
        let _ = super::policy::mark_other_authors(
            self.selected_mut(),
            &config.repo.email,
            config.include_other_authors,
        );
        self.validate_conflicts()?;
        Ok(())
    }
}
