//! Terminal presentation of submission decisions and results.

use anyhow::{Result, bail};
use std::collections::BTreeSet;

use crate::{
    cli::output,
    commands::Commands,
    config::Config,
    model::{Commit, shorten_title},
    stack::selection,
};

use super::{Application, State, policy};

pub(super) fn checked_outcome(commands: &Commands, resource: &str, checked: usize, changed: usize) {
    let result = if changed == 0 {
        "no changes needed".into()
    } else {
        format!("{changed} changed")
    };
    commands
        .terminal
        .outcome(&format!("Checked {checked} {resource}; {result}"));
}

pub(super) fn failure_outcome(commands: &Commands, error: &anyhow::Error) {
    use crate::model::outcome::Operation;
    let diagnostic = error
        .downcast_ref::<crate::github::policy::NativeEligibility>()
        .map_or_else(
            || format!("{error:#}"),
            |cause| {
                if commands.verbose {
                    format!("{cause}\nDetails: {error:#}")
                } else {
                    let request = error
                        .downcast_ref::<crate::github::http::HttpStatusError>()
                        .map_or_else(String::new, |status| {
                            format!(
                                "\nGitHub POST {} failed: HTTP {}.",
                                status.url, status.status
                            )
                        });
                    format!("{cause}{request}")
                }
            },
        );
    commands.terminal.err(&format!(
        "\x1b[31m{}\x1b[0m\n",
        crate::diagnostics::errors::redact_url_credentials(&diagnostic)
    ));
    let effects = commands.outcomes.snapshot();
    let mut branches = 0;
    let mut created = 0;
    let mut updated = BTreeSet::new();
    let mut unconfirmed = Vec::new();
    for (operation, effect) in effects {
        if effect.confirmed {
            match &operation {
                Operation::Branch(_) => branches += 1,
                Operation::Creation(_) => created += 1,
                Operation::Base(number)
                | Operation::Description(number)
                | Operation::Labels(number) => {
                    updated.insert(*number);
                }
                Operation::NativeStack => commands
                    .terminal
                    .err("Native stack mutation acknowledged; completed changes are retained.\n"),
            }
        }
        if effect.unconfirmed {
            let label = match operation {
                Operation::Branch(reference) => format!("branch publication for {reference:?}"),
                Operation::Creation(reference) => format!("PR creation for head {reference:?}"),
                Operation::Base(number) => format!("base update for PR #{number}"),
                Operation::Description(number) => format!("description update for PR #{number}"),
                Operation::Labels(number) => format!("label update for PR #{number}"),
                Operation::NativeStack => "native stack mutation".into(),
            };
            unconfirmed.push(label);
        }
    }
    if branches == 0 && created == 0 && updated.is_empty() && unconfirmed.is_empty() {
        commands
            .terminal
            .err("No branches were pushed or PR content changed.\n");
    } else {
        commands.terminal.err(&format!("Confirmed effects: branches published: {branches}; PRs created: {created}; PRs updated: {}.\n", updated.len()));
    }
    for operation in unconfirmed {
        commands.terminal.err(&format!(
            "Unconfirmed: {operation}; verify remote state before retrying.\n"
        ));
    }
}

impl Application {
    pub(super) fn explain_bases(&self, state: &State, indices: &[usize], bases: &[String]) {
        for &index in indices {
            self.commands.explain(&format!(
                "Head {:?}: selected base {:?}.",
                state.stack[index].remote_ref(),
                bases[index]
            ));
        }
    }

    pub(super) fn report_pr_checks(&self, state: &State, needs_pr: &[usize]) {
        let effects = self.commands.outcomes.snapshot();
        let created = needs_pr
            .iter()
            .filter(|&&i| {
                effects
                    .get(&crate::model::outcome::Operation::Creation(
                        state.stack[i].remote_ref().into(),
                    ))
                    .is_some_and(|effect| effect.confirmed)
            })
            .count();
        let updated = needs_pr
            .iter()
            .filter(|&&i| {
                effects
                    .get(&crate::model::outcome::Operation::Base(
                        state.stack[i].pr_number,
                    ))
                    .is_some_and(|effect| effect.confirmed)
            })
            .count();
        checked_outcome(
            &self.commands,
            "pull requests",
            needs_pr.len(),
            created + updated,
        );
    }

    pub(super) fn stage(&self, active: &str, completed: &str) {
        self.commands
            .terminal
            .stage_with_completion(active, completed);
    }

    pub(super) fn present_selection(&self, config: &Config, state: &mut State) {
        let mut visible: Vec<_> = state.selected().iter().collect();
        if config.stack_up {
            visible.reverse();
        }
        if !self
            .commands
            .terminal
            .selection(&visible, config.stack_up, config.preview_limit)
        {
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
            && self.commands.terminal.results(
                &visible,
                &config.repo,
                config.stack_up,
                config.hyperlinks,
            )
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
