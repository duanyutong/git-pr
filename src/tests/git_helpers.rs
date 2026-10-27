//! Test support excluded from production coverage.

use anyhow::{Context, Result, bail};

use super::{Git, resolve_refs};
use crate::{diagnostics::errors::Fatal, model::Commit, stack::selection};

impl Git<'_> {
    pub(crate) fn depth(&self, from: &str, to: &str) -> Result<usize> {
        let output = self
            .commands
            .git(&["rev-list", "--count", &format!("{from}..{to}")])
            .context(Fatal)?;
        output
            .parse()
            .with_context(|| format!("ERROR: failed to parse commit count for {from}..{to}"))
    }

    pub(crate) fn at_depths(&self, head: &str, depths: &[usize]) -> Result<Vec<String>> {
        resolve_refs(
            self.commands,
            &depths
                .iter()
                .map(|depth| format!("{head}~{depth}"))
                .collect::<Vec<_>>(),
        )
    }

    pub(crate) fn stack(
        &self,
        base: &str,
        tip: &str,
        include_working_copy: bool,
    ) -> Result<Vec<Commit>> {
        self.observe_stack(base, tip, include_working_copy)
            .map(|observation| observation.commits)
    }

    pub(crate) fn local_branch(&self, commit: &Commit, output: &str) -> String {
        if let Some(branch) = selection::local_branch(output, &commit.hash) {
            return branch;
        }
        self.commands.out(&format!("[BRANCH-NOT-FOUND] commit {} not on any local branch\n  Commit title: {}\n  Available local branches:", commit.short_hash(), commit.title));
        for (branch, hash) in output
            .lines()
            .filter_map(|line| line.trim().rsplit_once('|'))
        {
            if !selection::ignored_branch(branch) {
                self.commands.out(&format!(
                    "    {branch} -> {}",
                    hash.get(..8).unwrap_or(hash)
                ));
            }
        }
        String::new()
    }

    pub(crate) fn reword_all(&self, commits: &[&Commit]) -> Result<()> {
        if commits.is_empty() {
            return Ok(());
        }
        let messages: Vec<_> = commits
            .iter()
            .map(|c| (c.hash.clone(), c.full_message()))
            .collect();
        // Backend editor delimiters cannot represent every legal commit message.
        // Retain the backend's literal-message path for those uncommon inputs.
        if commits.len() == 1
            || messages
                .iter()
                .any(|(_, m)| m.contains("++ reword") || m.lines().any(|l| l.starts_with("JJ:")))
        {
            for (commit, (_, message)) in commits.iter().zip(&messages).rev() {
                self.reword_message(commit, message)?;
            }
            return Ok(());
        }
        let program = if self.repo.jj {
            "jj"
        } else if self.repo.branchless {
            "git"
        } else {
            return self.reword_message(commits[0], &messages[0].1).map(|_| ());
        };
        let mut args = vec![if self.repo.jj {
            "describe".into()
        } else {
            "reword".into()
        }];
        args.extend(commits.iter().map(|c| c.hash.clone()));
        let _activity = self.commands.activity("Rewriting commit messages", None);
        self.commands
            .runner
            .edit_messages(
                program,
                &args,
                &self.commands.cwd,
                &self.commands.env,
                &messages,
            )
            .and_then(crate::commands::CommandOutput::success)
            .with_context(|| format!("failed to rewrite commit messages with {program}; inspect backend status before retrying"))
            .context(Fatal)?;
        Ok(())
    }

    pub(crate) fn reword(&self, commit: &Commit) -> Result<String> {
        self.reword_message(commit, &commit.full_message())
    }

    fn reword_message(&self, commit: &Commit, message: &str) -> Result<String> {
        if self.repo.jj {
            if commit.change_id.is_empty() {
                bail!("commit {} has no change ID", commit.short_hash());
            }
            return self
                .commands
                .jj(&["describe", "-r", &commit.change_id, "-m", message])
                .context(Fatal);
        }
        if self.repo.branchless {
            return self
                .commands
                .git(&["reword", &commit.hash, "-m", message])
                .context(Fatal);
        }
        bail!(
            "ERROR: neither jj nor git-branchless is available\n\nThis tool requires either:\n  1. Jujutsu (jj) - install from https://martinvonz.github.io/jj/\n     OR\n  2. git-branchless - install from https://github.com/arxanas/git-branchless\n     Then run: git branchless init\n\nAfter installation, try again."
        )
    }

    pub(crate) fn push_all(&self, commits: &[&Commit]) -> Result<()> {
        self.push_with_leases(commits, None)
    }
}
