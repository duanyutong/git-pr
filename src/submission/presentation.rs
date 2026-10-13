//! Terminal presentation of submission decisions and results.

use anyhow::{Result, bail};

use crate::{
    cli::output,
    commands::Commands,
    config::Config,
    model::{Commit, shorten_title},
    stack::selection,
};

use super::{Application, State, policy};

impl Application {
    pub(super) fn present_selection(&self, config: &Config, state: &mut State) {
        let mut visible: Vec<_> = state.selected().iter().collect();
        if config.stack_up {
            visible.reverse();
        }
        if !self.commands.terminal.selection(&visible, config.stack_up) {
            for commit in visible {
                self.commands.out(&commit.to_string());
            }
            self.commands.out("");
        }
        for index in selection::mark_drafts(
            state.selected_mut(),
            config.skip_draft,
            config.include_draft,
            &config.draft_patterns,
        ) {
            let commit = &state.selected()[index];
            self.commands.out(&format!(
                "skip draft \"{}\" ({})",
                shorten_title(&commit.title),
                commit.short_hash()
            ));
        }
    }

    pub(super) fn filter_authors(&self, config: &Config, state: &mut State) {
        for index in policy::mark_other_authors(
            state.selected_mut(),
            &config.repo.email,
            config.include_other_authors,
        ) {
            let commit = &state.selected()[index];
            let author = if commit.author_email.is_empty() {
                "@unknown"
            } else {
                &commit.author_email
            };
            self.commands.out(&format!(
                "skip \"{}\" ({author})",
                shorten_title(&commit.title)
            ));
        }
    }

    pub(super) fn print_results(&self, config: &Config, commits: &[Commit]) {
        let mut visible: Vec<_> = commits.iter().filter(|c| !c.skip).collect();
        if config.stack_up {
            visible.reverse();
        }
        if config.output == "{url} {title}"
            && self
                .commands
                .terminal
                .results(&visible, &config.repo, config.stack_up)
        {
            return;
        }
        self.commands.out("");
        self.commands.out(&format!(
            "Stack of {} ({} at the top):\n\n",
            visible.len(),
            if config.stack_up { "newest" } else { "oldest" }
        ));
        let template = output::Template::new(&config.output);
        for (i, commit) in visible.into_iter().enumerate() {
            if i != 0 {
                self.commands.out("");
            }
            let status = if commit.newly_created {
                " (created)"
            } else if commit.base_updated || commit.metadata_updated {
                " (updated)"
            } else {
                ""
            };
            self.commands.out(&format!(
                "{}{status}",
                template.render(commit, &config.repo.host, &config.repo.name)
            ));
        }
    }
}

pub(super) fn validate_duplicates(commits: &[Commit], commands: &Commands) -> Result<()> {
    if let Some((previous, commit)) = policy::duplicate_ref(commits) {
        commands.out(&format!("[ERROR] DUPLICATED REMOTE REF: {:?}\n  Commit 1: {} - {}\n  Commit 2: {} - {}\n  This means a previous run didn't complete properly.\n  Delete the Remote-Ref from one of these commits and try again.", commit.remote_ref(), previous.short_hash(), previous.title, commit.short_hash(), commit.title));
        bail!(
            "duplicated remote ref {:?} found for {:?} and {:?}",
            commit.remote_ref(),
            previous.short_hash(),
            commit.short_hash()
        );
    }
    Ok(())
}
