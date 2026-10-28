//! Submission workflow sequencing across explicit external boundaries.

pub(crate) mod policy;
mod presentation;
mod pull_requests;
mod recovery;
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
        self.commands.explain = flags.verbosity() > 0;
        self.commands.outcomes = crate::model::outcome::Outcomes::default();
        // Publication transfers stored objects, so every observation
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
        let mut reporting = self.commands.clone();
        reporting.verbose = flags.verbosity() > 1;
        let result = self.submit(flags, env);
        terminal.finish(result.is_ok());
        diagnostics.summary(result.is_ok());
        if let Err(error) = &result {
            presentation::failure_outcome(&reporting, error);
        }
        result
    }

    fn submit(mut self, flags: &Flags, env: &BTreeMap<String, String>) -> Result<()> {
        let diagnostics = self.commands.diagnostics.clone();
        self.commands.terminal.section("Repository");
        self.stage("Discovering repository", "Repository discovered");
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
        self.stage("Validating working copy", "Working copy validated");
        diagnostics.phase("working-copy validation", || Self::check_working_copy(git))?;
        if self.checkpoint(&config, Checkpoint::Validate) {
            return Ok(());
        }
        self.stage("Reading commits", "Commits read");
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
        let _guard = self.lock_repository(&config)?;
        let publication = diagnostics
            .phase("commit preparation", || {
                self.prepare_publication(&github, &mut state)
            })
            .context(
                "commit preparation failed; inspect the reported identity evidence before retrying",
            )?;
        if self.checkpoint(&config, Checkpoint::Rewrite) {
            return Ok(());
        }
        let git = Git {
            commands: &self.commands,
            repo: &config.repo,
        };
        let (active, completed) = if config.dry_run {
            ("Previewing publication", "Publication previewed")
        } else {
            ("Publishing branches", "Publication reconciled")
        };
        self.stage(active, completed);
        diagnostics.phase("publication and PR reconciliation", || self.push(&config, git, &github, &mut state, &publication))
            .context("publication or PR reconciliation failed; any completed local metadata writes, pushed refs and PR changes are retained. Inspect the reported operation before retrying")?;
        if self.checkpoint(&config, Checkpoint::Push) {
            return Ok(());
        }
        if config.dry_run {
            self.preview_descriptions(&state);
            return Ok(());
        }
        diagnostics.phase("PR-number recovery", || state.fill_numbers(&github, &publication.bases))
            .context("PR lookup or creation failed after publication; pushed refs and completed PR changes are retained")?;
        if self.checkpoint(&config, Checkpoint::PrCreate) {
            return Ok(());
        }
        self.stage("Updating PR descriptions", "Reconciled descriptions");
        let associations = crate::identity::resolve::verify_bindings(
            &self.commands,
            &config.repo,
            state.selected(),
        )?;
        let numbers = diagnostics.phase("PR descriptions and stack sections", || Self::update_descriptions(&config, git, &github, &state))
            .context("PR content reconciliation failed after publication; pushed refs and completed PR changes are retained")?;
        diagnostics.phase("native stack links", || Self::native_stack(&config, &github, &state, &publication.bases))
            .context("native stack reconciliation failed after publication; pushed refs and completed PR changes are retained")?;
        self.finish_submission(&github, &mut state, &publication, &associations, &numbers)
    }

    fn finish_submission(
        &self,
        github: &GitHub<'_>,
        state: &mut State,
        publication: &pull_requests::Publication,
        associations: &crate::identity::store::Records,
        numbers: &[u64],
    ) -> Result<()> {
        publication.finish(github, state)?;
        if let Err(error) = crate::identity::resolve::cleanup(github, associations, &state.stack) {
            self.commands.warn(&format!(
                "could not finish merged publication metadata cleanup; retained associations may be retried: {error:#}"
            ));
        }
        for commit in state.selected_mut() {
            commit.metadata_updated |= github.was_updated(commit.pr_number);
        }
        self.commands.terminal.finish(true);
        let summaries = if github.config.output == "{url} {title}" {
            github.summaries(numbers)
        } else {
            Vec::new()
        };
        self.print_results(github.config, state.selected(), &summaries);
        Ok(())
    }

    fn preview_descriptions(&self, state: &State) {
        self.commands
            .out("[DRY-RUN] Would update PR descriptions for:");
        for commit in state.selected().iter().filter(|c| !c.skip) {
            self.commands
                .out(&format!("  - {}: {}", commit.short_hash(), commit.title));
        }
    }

    fn lock_repository(
        &self,
        config: &Config,
    ) -> Result<Option<Box<dyn config::files::RepositoryLock>>> {
        if config.dry_run || config.stop_after == Some(Checkpoint::Rewrite) {
            return Ok(None);
        }
        let path = self.commands.git_data(
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            &[],
        )?;
        let path =
            std::path::Path::new(path.strip_suffix('\n').unwrap_or(&path)).join("git-pr.lock");
        Ok(Some(self.files.lock(&path).context(
            "another submission is active, or the repository lock could not be acquired",
        )?))
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
        self.stage(
            if config.dry_run {
                "Preparing preview"
            } else {
                "Preparing commits"
            },
            if config.dry_run {
                "Preview prepared"
            } else {
                "Commits prepared"
            },
        );
        state.validate_conflicts()?;
        if !state.selected().iter().any(|commit| !commit.skip) {
            return Ok(pull_requests::Publication::default());
        }
        crate::identity::resolve::prepare(github, &mut state.stack, state.start)?;
        let publication = self.commands.diagnostics.phase("publication safety preflight", || pull_requests::Publication::observe(github, &state.stack, state.start)).context(Fatal)
            .context("publication safety preflight failed; no local metadata, pushes or PR changes were performed")?;
        if !config.dry_run && config.stop_after != Some(Checkpoint::Rewrite) {
            crate::identity::resolve::verify_anchors(&self.commands, state.selected())?;
            crate::identity::resolve::persist(github, state.selected_mut())?;
        }
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
            github
                .commands
                .terminal
                .stage_with_completion("Linking native stack", "Native stack reconciled");
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
