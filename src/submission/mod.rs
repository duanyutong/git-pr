//! Submission workflow sequencing across explicit external boundaries.

pub(crate) mod policy;
mod presentation;
mod pull_requests;
mod recovery;
mod rewrite;
mod runtime;
mod state;

pub use runtime::run;

use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, bail};

use crate::{
    cli::Flags,
    commands::{Clock, Commands},
    config::{self, Config, Loaded, checkpoint::Checkpoint},
    credentials::CredentialStore,
    diagnostics::errors::Fatal,
    git::Git,
    github::{GitHub, http::Transport},
};

use presentation::validate_duplicates;
use state::State;

#[derive(Clone, Debug)]
pub(crate) struct Application {
    pub commands: Commands,
    pub clock: Arc<dyn Clock>,
    pub transport: Arc<dyn Transport>,
    pub credentials: Arc<dyn CredentialStore>,
    pub files: Arc<dyn config::files::Files>,
}

impl Application {
    pub(crate) fn execute(mut self, flags: &Flags, env: &BTreeMap<String, String>) -> Result<()> {
        // Publication transfers stored objects, so every observation and rewrite
        // must use those same objects rather than a local replacement view.
        self.commands
            .env
            .entry("GIT_NO_REPLACE_OBJECTS".into())
            .or_insert_with(|| "1".into());
        self.commands.env.insert("GIT_GRAFT_FILE".into(), "".into());
        if flags.verbosity() > 0 {
            self.commands.diagnostics =
                crate::diagnostics::Diagnostics::new(Arc::clone(&self.commands.terminal));
            self.commands.runner = Arc::new(crate::diagnostics::TimedRunner {
                inner: Arc::clone(&self.commands.runner),
                diagnostics: self.commands.diagnostics.clone(),
            });
        }
        let diagnostics = self.commands.diagnostics.clone();
        let terminal = Arc::clone(&self.commands.terminal);
        let result = self.submit(flags, env);
        terminal.finish(result.is_ok());
        diagnostics.summary(result.is_ok());
        result
    }

    fn submit(mut self, flags: &Flags, env: &BTreeMap<String, String>) -> Result<()> {
        let diagnostics = self.commands.diagnostics.clone();
        self.commands.terminal.section("Repository");
        self.commands.terminal.stage("Discovering repository");
        let Loaded::Ready(mut config) =
            diagnostics.phase("configuration and repository discovery", || {
                config::load(
                    flags,
                    env,
                    &mut self.commands,
                    self.credentials.as_ref(),
                    self.files.as_ref(),
                )
            })?
        else {
            return Ok(());
        };
        let git = Git {
            commands: &self.commands,
            repo: &config.repo,
        };
        self.commands.terminal.stage("Validating working copy");
        diagnostics.phase("working-copy validation", || Self::check_working_copy(git))?;
        if self.checkpoint(&config, Checkpoint::Validate) {
            return Ok(());
        }
        self.commands.terminal.stage("Reading commits");
        let mut state = diagnostics.phase("commit discovery", || {
            State::discover(&mut config, &self.commands)
        })?;
        self.commands.terminal.finish(true);
        self.present_selection(&config, &mut state);
        if self.checkpoint(&config, Checkpoint::GetCommits) {
            return Ok(());
        }
        self.filter_authors(&config, &mut state);
        validate_duplicates(state.selected(), &self.commands)?;
        let template = if config.dry_run || config.stop_after == Some(Checkpoint::Rewrite) {
            String::new()
        } else {
            diagnostics.phase("PR template", || {
                config::load_template(&config.repo.root, self.files.as_ref())
            })?
        };
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: Some(self.clock.as_ref()),
            config: &config,
            commands: &self.commands,
            transport: self.transport.as_ref(),
            template: &template,
        };
        let publication = diagnostics.phase("commit preparation", || self.prepare_publication(&github, &mut state))
            .context("commit preparation failed; inspect Git or Jujutsu status before retrying because local history may already have changed")?;
        if self.checkpoint(&config, Checkpoint::Rewrite) {
            return Ok(());
        }
        let git = Git {
            commands: &self.commands,
            repo: &config.repo,
        };
        self.commands.terminal.stage(if config.dry_run {
            "Previewing publication"
        } else {
            "Publishing branches"
        });
        diagnostics.phase("publication and PR reconciliation", || self.push(&config, git, &github, &mut state, &publication))
            .context("publication or PR reconciliation failed; any completed local rewrites, pushed refs and PR changes are retained. Inspect the reported operation before retrying")?;
        if self.checkpoint(&config, Checkpoint::Push) {
            return Ok(());
        }
        if config.dry_run {
            self.commands
                .out("[DRY-RUN] Would update PR descriptions for:");
            for commit in state.selected().iter().filter(|c| !c.skip) {
                self.commands
                    .out(&format!("  - {}: {}", commit.short_hash(), commit.title));
            }
            return Ok(());
        }
        diagnostics.phase("PR-number recovery", || state.fill_numbers(&github, &publication.bases))
            .context("PR lookup or creation failed after publication; pushed refs and completed PR changes are retained")?;
        if self.checkpoint(&config, Checkpoint::PrCreate) {
            return Ok(());
        }
        self.commands.terminal.stage("Updating PR descriptions");
        diagnostics.phase("PR descriptions and stack sections", || Self::update_descriptions(&config, git, &github, &state))
            .context("PR content reconciliation failed after publication; pushed refs and completed PR changes are retained")?;
        diagnostics.phase("native stack links", || Self::native_stack(&config, &github, &state, &publication.bases))
            .context("native stack reconciliation failed after publication; pushed refs and completed PR changes are retained")?;
        for commit in state.selected_mut() {
            commit.metadata_updated = github.was_updated(commit.pr_number);
        }
        self.commands.terminal.finish(true);
        self.print_results(&config, state.selected());
        Ok(())
    }

    fn prepare_publication(
        &self,
        github: &GitHub<'_>,
        state: &mut State,
    ) -> Result<pull_requests::Publication> {
        let config = github.config;
        self.commands.terminal.section(if config.dry_run {
            "Preview"
        } else {
            "Submission"
        });
        self.commands.terminal.stage(if config.dry_run {
            "Preparing preview"
        } else {
            "Preparing commits"
        });
        let mut publication = pull_requests::Publication::default();
        state.rewrite(github.config, &self.commands, self.clock.as_ref(), &mut |stack, start| {
            publication = self.commands.diagnostics.phase("publication safety preflight", || pull_requests::Publication::observe(github, stack, start)).context(Fatal)
                .context("publication safety preflight failed; no local rewrites, pushes or PR changes were performed")?;
            Ok(())
        })?;
        Ok(publication)
    }

    fn check_working_copy(git: Git<'_>) -> Result<()> {
        if !git.clean()? {
            if git.repo.jj {
                bail!(
                    "ERROR: uncommitted changes in the Jujutsu working copy have no description; set one with `jj describe -m '<description>'` before submitting"
                );
            }
            bail!(
                "ERROR: git status reports uncommitted changes\n\nHint: commit or stash tracked changes before submitting"
            );
        }
        Ok(())
    }

    fn native_stack(
        config: &Config,
        github: &GitHub<'_>,
        state: &State,
        bases: &[String],
    ) -> Result<()> {
        if config.github_stack {
            github.commands.terminal.stage("Linking native stack");
            let base = state
                .stack
                .iter()
                .enumerate()
                .skip(state.start)
                .find(|(_, c)| !c.skip)
                .map_or(config.repo.trunk.as_str(), |(i, _)| bases[i].as_str());
            github.manage_stack(state.selected(), base)?;
        }
        Ok(())
    }

    fn checkpoint(&self, config: &Config, phase: Checkpoint) -> bool {
        if config.stop_after == Some(phase) {
            self.commands
                .out(&format!("stopped after: {}", phase.as_str()));
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/workflow_parts.rs"]
mod unit;
