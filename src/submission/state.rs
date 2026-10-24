//! Discovery and tracking of the selected commit stack.

use anyhow::{Context, Result, bail};

use crate::{
    commands::Commands, config::Config, diagnostics::errors::Fatal, git::Git, jj, model::Commit,
    stack::selection,
};

#[derive(Debug)]
pub(super) struct State {
    pub(super) stack: Vec<Commit>,
    pub(super) start: usize,
    pub(super) observed_tip: String,
}

impl State {
    pub(super) fn selected(&self) -> &[Commit] {
        &self.stack[self.start..]
    }
    pub(super) fn selected_mut(&mut self) -> &mut [Commit] {
        &mut self.stack[self.start..]
    }
    pub(super) fn validate_conflicts(&self) -> Result<()> {
        // Skipped ancestors remain reachable from a selected publication head.
        if let Some(last) = (self.start..self.stack.len()).rfind(|&i| !self.stack[i].skip)
            && let Some(commit) = self.stack[..=last].iter().find(|commit| commit.conflicted)
        {
            bail!(
                "commit {} has unresolved Jujutsu conflicts in its ancestry; resolve them before submitting",
                commit.short_hash()
            );
        }
        Ok(())
    }
    pub(super) fn validate_refs(&self, trunk: &str) -> Result<()> {
        for commit in self.stack[self.start..]
            .iter()
            .filter(|c| !c.skip && !c.remote_ref().is_empty())
        {
            crate::git::policy::protect_trunk(commit.remote_ref(), trunk)?;
            Git::validate_remote_ref(commit.remote_ref()).with_context(|| {
                format!(
                    "commit {} has invalid Remote-Ref {:?} — refusing to push\nTitle: {}",
                    commit.short_hash(),
                    commit.remote_ref(),
                    commit.title
                )
            })?;
        }
        Ok(())
    }

    pub(super) fn discover(config: &mut Config, commands: &Commands) -> Result<Self> {
        let git = Git {
            commands,
            repo: &config.repo,
        };
        let trunk = format!("refs/remotes/{}/{}", config.repo.remote, config.repo.trunk);
        let head = if !config.selection.selected.is_empty() && config.repo.jj {
            // Include a described working-copy change without adding an empty @.
            jj::resolve_revision(commands, "heads(@- | (@ ~ empty()))")?
        } else if !config.selection.selected.is_empty() {
            "HEAD".into()
        } else if config.selection.explicit {
            config.selection.tip.clone()
        } else {
            git.head()?
        };
        let observation = git
            .observe_stack(&trunk, &head, !config.selection.explicit)
            .context(Fatal)?;
        let trunk_hash = observation.base_hash;
        let mut stack = observation.commits;
        let known_depths = observation.depths;
        let mut observed_tip = observation.tip_hash;
        if !config.selection.selected.is_empty() {
            let (lo, hi) = selection::order_selection(&stack, &config.selection.selected)?;
            config.selection.tip.clone_from(&stack[hi].hash);
            config.selection.base = if lo == 0 {
                trunk_hash.clone()
            } else {
                stack[lo - 1].hash.clone()
            };
            observed_tip.clone_from(&config.selection.tip);
            stack.truncate(hi + 1);
        }
        let (base, tip) = if config.selection.explicit {
            (config.selection.base.clone(), config.selection.tip.clone())
        } else {
            (trunk, head)
        };
        if stack.is_empty() {
            bail!("no commits to submit");
        }
        let start = if config.selection.explicit {
            selection::slice_start(&stack, &base, &trunk_hash, &known_depths)
        } else {
            0
        };
        if start == stack.len() {
            bail!(
                "no commits to submit in range {}..{} (base {} not found on trunk..tip path)",
                abbreviated(&base),
                abbreviated(&tip),
                abbreviated(&base)
            );
        }
        let mut state = Self {
            stack,
            start,
            observed_tip,
        };
        if !config.selection.selected.is_empty() {
            for announcement in
                selection::mark_unselected(state.selected_mut(), &config.selection.selected)
            {
                commands.out(&announcement);
            }
        }
        Ok(state)
    }
}

fn abbreviated(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
