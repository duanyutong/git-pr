//! Ordered pull-request operations and completion barriers.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::{
    commands::parallel_cached,
    config::Config,
    diagnostics::errors::Fatal,
    git::Git,
    github::GitHub,
    model::{Commit, shorten_title},
    stack::{
        description::{self, HistoryEntry, Style},
        selection,
    },
};

use super::{Application, State};

/// Identities and relationships authorised before any local rewrite or publication.
#[derive(Debug, Default)]
pub(super) struct Publication {
    pub(super) bases: Vec<String>,
    entries: Vec<(usize, String, Option<Identity>)>,
    dependant_refs: Vec<String>,
    dependant_times: Vec<Cell<Instant>>,
    recovery: Option<super::recovery::Recovery>,
    leases: std::collections::BTreeMap<String, Option<String>>,
}

#[derive(Debug)]
struct Identity {
    number: u64,
    node_id: String,
}

impl Publication {
    pub(super) fn observe(github: &GitHub<'_>, stack: &[Commit], start: usize) -> Result<Self> {
        if github.config.dry_run
            || github.config.stop_after == Some(crate::config::checkpoint::Checkpoint::Rewrite)
            || !stack[start..].iter().any(|c| !c.skip)
        {
            return Ok(Self::default());
        }
        let bases = selection::resolve_bases(stack, start, &github.config.repo.trunk);
        let mut plan = Self {
            bases,
            entries: Vec::new(),
            dependant_refs: Vec::new(),
            dependant_times: Vec::new(),
            recovery: None,
            leases: std::collections::BTreeMap::new(),
        };
        let mut recover = !github.config.transition_journal.is_empty();
        for (group, chunk) in stack.chunks(25).enumerate() {
            let references: Vec<_> = chunk
                .iter()
                .map(|commit| commit.remote_ref().to_owned())
                .collect();
            github.observe(&references)?;
            github.refresh_prs(
                &references
                    .iter()
                    .map(|reference| github.observed_head(reference, "all"))
                    .filter(|number| *number != 0)
                    .collect::<Vec<_>>(),
            )?;
            for (offset, commit) in chunk.iter().enumerate() {
                let index = group * 25 + offset;
                if index < start || commit.skip {
                    continue;
                }
                let reference = commit.remote_ref();
                let pr = github.publication_pr(reference)?;
                if let Some(number) = commit
                    .publication
                    .as_ref()
                    .and_then(|binding| binding.review)
                    && pr.as_ref().map(|pr| pr.number) != Some(number)
                {
                    bail!(
                        "adopted PR #{number} differs from current head history; refusing a replacement"
                    );
                }
                if let Some(pr) = &pr {
                    if pr.base.name != plan.bases[index]
                        && github.config.transition_journal.is_empty()
                        && plan.bases[index] != github.config.repo.trunk
                        && pr.base.name != github.config.repo.trunk
                        && !stack[start..]
                            .iter()
                            .any(|commit| !commit.skip && commit.remote_ref() == pr.base.name)
                    {
                        crate::github::policy::require_publication_base(pr, &plan.bases[index])?;
                    }
                    recover |= pr.base.name != plan.bases[index];
                }
                if pr.is_none() {
                    crate::stack::description::initial_body(&commit.message, github.template)?;
                }
                let identity = pr.map(|pr| Identity {
                    number: pr.number,
                    node_id: pr.node_id.clone(),
                });
                plan.entries.push((index, reference.to_owned(), identity));
            }
        }
        plan.observe_refs(github, stack)?;
        if recover {
            plan.recovery = Some(
                super::recovery::Recovery::observe(github, stack, start, &plan.bases)
                    .context("unsafe publication: publication transition could not be verified")?,
            );
            return Ok(plan);
        }
        // A leased push of the already published object cannot change its dependants.
        // Use the actual push destination rather than cached tracking refs or PR heads.
        plan.dependant_refs = plan
            .entries
            .iter()
            .filter(|(index, reference, _)| {
                plan.leases.get(reference).and_then(Option::as_ref) != Some(&stack[*index].hash)
            })
            .map(|(_, reference, _)| reference.clone())
            .collect();
        let selected = plan.selected_prs();
        plan.dependant_times = plan
            .dependant_refs
            .chunks(25)
            .map(|chunk| {
                let acquired = github.now();
                github.check_publication_dependants(chunk, &selected)?;
                Ok(Cell::new(acquired))
            })
            .collect::<Result<_>>()?;
        Ok(plan)
    }

    fn observe_refs(&mut self, github: &GitHub<'_>, stack: &[Commit]) -> Result<()> {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        let destination = git.recovery_destination()?;
        self.leases = git.recovery_refs(
            &destination,
            &self
                .entries
                .iter()
                .map(|(_, reference, _)| reference.clone())
                .collect::<Vec<_>>(),
        )?;
        for (index, reference, pr) in &self.entries {
            if pr.is_none()
                && let Some(Some(oid)) = self.leases.get(reference)
            {
                let commit = &stack[*index];
                if commit
                    .publication
                    .as_ref()
                    .is_some_and(|binding| !binding.adopted)
                    && oid != &commit.hash
                    && !git.is_ancestor(oid, &commit.hash)?
                {
                    bail!(
                        "remote head {reference:?} exists without a verified review or revision association; explicitly bind it before publication"
                    );
                }
            }
        }
        Ok(())
    }

    fn selected_prs(&self) -> HashMap<u64, (&str, &str)> {
        self.entries
            .iter()
            .filter_map(|(i, reference, pr)| {
                pr.as_ref()
                    .map(|pr| (pr.number, (reference.as_str(), self.bases[*i].as_str())))
            })
            .collect()
    }

    pub(super) fn finish(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        if let Some(recovery) = &self.recovery {
            if recovery.restore_native() && !github.config.github_stack {
                let base = state
                    .stack
                    .iter()
                    .enumerate()
                    .skip(state.start)
                    .find(|(_, commit)| !commit.skip)
                    .map_or(github.config.repo.trunk.as_str(), |(index, _)| {
                        self.bases[index].as_str()
                    });
                github.manage_stack(state.selected(), base)?;
            }
            recovery.finish(github, state)?;
        }
        Ok(())
    }

    pub(super) fn verify(
        &self,
        github: &GitHub<'_>,
        state: &mut State,
        published: bool,
    ) -> Result<()> {
        if self.bases.len() != state.stack.len()
            || self.entries.len() != state.selected().iter().filter(|c| !c.skip).count()
        {
            bail!(
                "publication selection changed after preparation; refusing to publish an unverified selection"
            );
        }
        if published {
            github.invalidate_heads(
                &self
                    .entries
                    .iter()
                    .map(|(_, reference, _)| reference.clone())
                    .collect::<Vec<_>>(),
            );
        }
        let mut commits = state
            .stack
            .iter_mut()
            .enumerate()
            .skip(state.start)
            .filter(|(_, commit)| !commit.skip);
        for chunk in self.entries.chunks(25) {
            github.observe(
                &chunk
                    .iter()
                    .map(|(_, reference, _)| reference.clone())
                    .collect::<Vec<_>>(),
            )?;
            github.refresh_prs(
                &chunk
                    .iter()
                    .map(|(_, reference, _)| github.observed_head(reference, "all"))
                    .filter(|n| *n != 0)
                    .collect::<Vec<_>>(),
            )?;
            for ((index, reference, expected), (actual_index, commit)) in
                chunk.iter().zip(commits.by_ref())
            {
                if actual_index != *index || commit.remote_ref() != reference {
                    bail!(
                        "publication selection changed after preparation; refusing to publish or reconcile a different stack"
                    );
                }
                let actual = github.publication_pr(reference)?;
                if actual.as_ref().map(|p| (p.number, &p.node_id))
                    != expected.as_ref().map(|p| (p.number, &p.node_id))
                {
                    bail!(
                        "PR identity for head {reference:?} changed during submission; refusing to create a replacement or modify a different PR. Inspect the remote state before retrying"
                    );
                }
                if let Some(pr) = actual {
                    let base = if published {
                        &self.bases[*index]
                    } else {
                        self.recovery
                            .as_ref()
                            .map_or(self.bases[*index].as_str(), |plan| {
                                plan.publication_base(*index)
                            })
                    };
                    crate::github::policy::require_publication_base(&pr, base)?;
                    commit.pr_number = pr.number;
                }
            }
        }
        let mut selected = None;
        // Preserve each bounded group's own acquisition age across long rewrites.
        for (chunk, observed) in self.dependant_refs.chunks(25).zip(&self.dependant_times) {
            let acquired = github.now();
            if acquired.saturating_duration_since(observed.get()) > Duration::from_secs(30) {
                let selected = selected.get_or_insert_with(|| self.selected_prs());
                github.check_publication_dependants(chunk, selected)?;
                observed.set(acquired);
            }
        }
        if published && let Some(recovery) = &self.recovery {
            recovery.verify_published(github, state)?;
        }
        Ok(())
    }
}

impl Application {
    pub(super) fn push(
        &self,
        config: &Config,
        git: Git<'_>,
        github: &GitHub<'_>,
        state: &mut State,
        publication: &Publication,
    ) -> Result<()> {
        let missing = selection::missing_refs(state.selected());
        if !missing.is_empty() {
            bail!(
                "ERROR: {} commit(s) have no resolved publication identity: {}; reobserve the stack before retrying",
                missing.len(),
                missing.join(", ")
            );
        }
        if config.dry_run {
            self.commands.out("[DRY-RUN] Would push commits:");
        }
        let indices: Vec<_> = (state.start..state.stack.len())
            .filter(|&i| !state.stack[i].skip)
            .collect();
        state.validate_refs(&config.repo.trunk)?;
        for &i in &indices {
            let commit = &state.stack[i];
            let announcement = format!(
                "{}push {} {} {}:refs/heads/{}",
                if config.dry_run { "[DRY-RUN] " } else { "" },
                "--force-with-lease",
                config.repo.remote,
                commit.short_hash(),
                commit.remote_ref()
            );
            if config.dry_run {
                self.commands.out(&announcement);
            } else {
                self.commands.detail(&announcement);
            }
        }
        if config.dry_run || indices.is_empty() {
            return Ok(());
        }
        crate::identity::resolve::verify_bindings(&self.commands, &config.repo, state.selected())?;
        git.recovery_destination()?;
        if let Some(recovery) = &publication.recovery {
            self.commands
                .terminal
                .stage("Preparing publication transition");
            recovery.apply(github, state, &publication.bases)?;
            self.commands.terminal.stage("Publishing branches");
        }
        crate::identity::resolve::verify_anchors(&self.commands, state.selected())?;
        publication.verify(github, state, false)?;
        let commits = indices.iter().map(|&i| &state.stack[i]).collect::<Vec<_>>();
        if let Some(recovery) = &publication.recovery {
            git.push_with_leases(&commits, Some(&recovery.refs))?;
        } else {
            git.push_with_leases(&commits, Some(&publication.leases))?;
        }
        git.sync_refs(state.selected());
        if let Some(recovery) = &publication.recovery {
            recovery.complete(github, state)?;
        }
        publication.verify(github, state, true)
            .context("PR state changed or could not be verified after publication; pushed refs are retained, and PR creation and updates have been stopped")?;
        self.commands.terminal.stage("Reconciling pull requests");
        self.reconcile_prs(&publication.bases, github, state, &indices)?;
        if config.no_stack {
            github.warn_blocked(state.selected());
        }
        Ok(())
    }

    pub(super) fn reconcile_prs(
        &self,
        bases: &[String],
        github: &GitHub<'_>,
        state: &mut State,
        needs_pr: &[usize],
    ) -> Result<()> {
        crate::identity::resolve::verify_bindings(
            &self.commands,
            &github.config.repo,
            state.selected(),
        )?;
        let lookup = needs_pr.to_vec();
        if !lookup.is_empty() {
            self.commands.detail("\nLooking up existing PRs...");
            let numbers = parallel_cached(
                lookup,
                |&i| {
                    github
                        .cached_find(&state.stack[i])
                        .map(|number| (i, number))
                },
                |i| Ok((i, github.find(&state.stack[i]).context(Fatal)?)),
            )?;
            for (i, number) in numbers {
                state.stack[i].pr_number = number;
            }
        }
        let create: Vec<_> = needs_pr
            .iter()
            .copied()
            .filter(|&i| state.stack[i].pr_number == 0)
            .collect();
        if !create.is_empty() {
            self.commands
                .detail(&format!("\nCreating {} PR(s)...", create.len()));
            let total = create.len();
            self.commands.terminal.progress("Creating PRs", 0, total);
            for (done, i) in create.into_iter().enumerate() {
                let base = &bases[i];
                self.commands.terminal.progress(
                    &format!("Creating {}", shorten_title(&state.stack[i].title)),
                    done,
                    total,
                );
                self.commands
                    .detail(&format!("  {}", shorten_title(&state.stack[i].title)));
                let attempt = crate::identity::Attempt {
                    id: self
                        .files
                        .attempt_id()
                        .context("failed to allocate creation attempt identity")?,
                    head: crate::identity::Head::new(
                        &github.config.repo,
                        state.stack[i].remote_ref(),
                    )?,
                    base: base.clone(),
                };
                crate::identity::store::begin(&self.commands, &github.config.repo, &attempt)?;
                github.create(&mut state.stack[i], base).context(Fatal)?;
                let commit = &state.stack[i];
                if let Some(binding) = &commit.publication {
                    crate::identity::store::persist(
                        &self.commands,
                        &github.config.repo,
                        binding,
                        &commit.change_id,
                        commit.pr_number,
                    )?;
                }
                crate::identity::store::finish(
                    &self.commands,
                    &github.config.repo,
                    commit.remote_ref(),
                )?;
                self.commands.progress_prs(
                    "Created",
                    std::iter::once(state.stack[i].pr_number),
                    done + 1,
                    total,
                );
            }
        }
        let update: Vec<_> = needs_pr
            .iter()
            .copied()
            .filter(|&i| !state.stack[i].newly_created)
            .collect();
        if !update.is_empty() {
            self.commands.detail("Updating PR(s)...");
            let indices: HashSet<_> = update.into_iter().collect();
            let changes = state
                .stack
                .iter()
                .enumerate()
                .filter(|(i, _)| indices.contains(i))
                .map(|(i, commit)| (commit, bases[i].as_str()))
                .collect();
            github.verify_bases(changes).context(Fatal)?;
        }
        Ok(())
    }

    pub(super) fn update_descriptions(
        config: &Config,
        git: Git<'_>,
        github: &GitHub<'_>,
        state: &State,
    ) -> Result<()> {
        if !state.selected().iter().any(|c| !c.skip) {
            return Ok(());
        }
        let targets: Vec<_> = state.selected().iter().filter(|c| !c.skip).collect();
        github.commands.progress_prs(
            "Preparing",
            targets.iter().map(|commit| commit.pr_number),
            0,
            targets.len(),
        );
        github
            .refresh_observations(
                &state
                    .stack
                    .iter()
                    .map(|c| c.remote_ref().to_owned())
                    .collect::<Vec<_>>(),
            )
            .context(Fatal)?;
        let descendants = description_descendants(config, git, github, state)?;
        let description_stack: Vec<_> = state.stack.iter().chain(&descendants).collect();
        let mut bodies = description::BodyCache::default();
        let (history, merged) =
            description_history(config, github, &description_stack, &mut bodies)?;
        let tip_hash = targets.last().map(|c| c.hash.as_str());
        let style = Style {
            stack_up: config.stack_up,
            emoji: config.emoji,
            host: &config.repo.host,
            repo: &config.repo.name,
        };
        let renderer = if config.compact_stack {
            description::Renderer::Compact(description::CompactRenderer::new(
                description_stack.iter().copied(),
            ))
        } else {
            description::Renderer::Full(description::StackRenderer::new(
                description_stack.iter().copied(),
                &history,
                &merged,
                style,
            ))
        };
        github
            .update_content(&targets, &mut |commit, pr| {
                bodies.get_shared(pr.number, &pr.body)?.render_footer(
                    &commit.message,
                    &renderer.footer(commit),
                    github.template,
                )
            })
            .context(Fatal)?;
        github.commands.terminal.stage("Updating PR labels");
        github
            .update_labels(
                &targets
                    .iter()
                    .map(|commit| (*commit, tip_hash == Some(commit.hash.as_str())))
                    .collect::<Vec<_>>(),
            )
            .context(Fatal)?;

        Ok(())
    }
}

impl State {
    pub(super) fn fill_numbers(&mut self, github: &GitHub<'_>, _bases: &[String]) -> Result<()> {
        if !self.selected().iter().any(|c| !c.skip) {
            return Ok(());
        }
        let indices: Vec<_> = (self.start..self.stack.len())
            .filter(|&i| self.stack[i].pr_number == 0)
            .collect();
        let updates = parallel_cached(
            indices,
            |&i| {
                let number = github.cached_find(&self.stack[i])?;
                if number == 0 && !self.stack[i].skip {
                    return None;
                }
                Some((i, number, None))
            },
            |i| {
                let mut commit = self.stack[i].clone();
                commit.pr_number = github.find(&commit).context(Fatal)?;
                if commit.pr_number == 0 && !commit.skip {
                    bail!(
                        "selected PR is missing after reconciliation; refusing unplanned creation"
                    );
                }
                Ok((i, commit.pr_number, Some(commit)))
            },
        )?;
        for (i, number, commit) in updates {
            if let Some(commit) = commit {
                self.stack[i] = commit;
            } else {
                self.stack[i].pr_number = number;
            }
        }
        if github.config.selection.explicit {
            let indices: Vec<_> = (0..self.stack.len())
                .filter(|&i| self.stack[i].pr_number == 0 && !self.stack[i].remote_ref().is_empty())
                .collect();
            let numbers = parallel_cached(
                indices,
                |&i| {
                    github
                        .cached_find(&self.stack[i])
                        .filter(|&n| n != 0)
                        .map(|n| (i, n))
                },
                |i| Ok((i, github.lookup(&self.stack[i]).context(Fatal)?)),
            )?;
            for (i, number) in numbers {
                self.stack[i].pr_number = number;
            }
        }
        Ok(())
    }
}

pub(super) fn description_history(
    config: &Config,
    github: &GitHub<'_>,
    stack: &[&Commit],
    bodies: &mut description::BodyCache,
) -> Result<(Vec<HistoryEntry>, HashSet<u64>)> {
    if config.compact_stack {
        return Ok((Vec::new(), HashSet::new()));
    }
    let mut history = Vec::<HistoryEntry>::new();
    let mut seen = HashSet::new();
    let mut merged = HashSet::new();
    for chunk in stack.chunks(25) {
        github
            .refresh_prs(
                &chunk
                    .iter()
                    .map(|commit| commit.pr_number)
                    .filter(|number| *number != 0)
                    .collect::<Vec<_>>(),
            )
            .context("failed to refresh PR stack history")
            .context(Fatal)?;
        for commit in chunk {
            if commit.pr_number == 0 {
                continue;
            }
            let pr = github
                .get(commit.pr_number)
                .with_context(|| format!("failed to read PR #{} stack history", commit.pr_number))
                .context(Fatal)?;
            if pr.is_merged() {
                merged.insert(commit.pr_number);
            }
            bodies
                .get_shared(pr.number, &pr.body)
                .context(Fatal)?
                .append_history(&mut history, &mut seen);
        }
    }
    if config.emoji {
        let current: HashSet<_> = stack
            .iter()
            .map(|c| c.pr_number)
            .filter(|n| *n != 0)
            .collect();
        let historical: Vec<_> = history
            .iter_mut()
            .filter(|entry| !current.contains(&entry.number))
            .collect();
        let merged_history = github
            .merged_prs(
                &historical
                    .iter()
                    .map(|entry| entry.number)
                    .collect::<Vec<_>>(),
            )
            .context(Fatal)?;
        for entry in historical {
            entry.merged = merged_history.contains(&entry.number);
        }
    }
    Ok((history, merged))
}

fn description_descendants(
    config: &Config,
    git: Git<'_>,
    github: &GitHub<'_>,
    state: &State,
) -> Result<Vec<Commit>> {
    let mut descendants = Vec::new();
    if !config.selection.explicit {
        let tip = git.stack_tip(&state.observed_tip).context(Fatal)?;
        if tip != state.observed_tip {
            let stack = git
                .description_commits(&state.observed_tip, &tip)
                .context(Fatal)?;
            let submitted: HashSet<_> = state.stack.iter().map(|c| &c.hash).collect();
            descendants = stack
                .into_iter()
                .filter(|c| !submitted.contains(&c.hash))
                .collect();
            let context_len = descendants.len();
            crate::identity::resolve::prepare(github, &mut descendants, context_len)?;
            github
                .observe(
                    &descendants
                        .iter()
                        .map(|c| c.remote_ref().to_owned())
                        .collect::<Vec<_>>(),
                )
                .context(Fatal)?;
            for commit in &mut descendants {
                if !commit.remote_ref().is_empty() {
                    commit.pr_number = github.observed_head(commit.remote_ref(), "open");
                }
            }
        }
    }
    Ok(descendants)
}
