//! Verified transition from a merged downstack to the configured trunk.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::state::State;
use crate::{
    git::Git,
    github::{GitHub, policy},
    model::{Commit, pull_request::RecoveryPullRequest},
};

#[derive(Debug)]
struct Entry {
    index: usize,
    reference: String,
    pr: Option<RecoveryPullRequest>,
}

#[cfg(test)]
#[path = "../tests/unit/base_recovery.rs"]
mod unit;

#[derive(Debug)]
pub(super) struct Recovery {
    entries: Vec<Entry>,
    parents: BTreeMap<String, MergedParent>,
    pub(super) refs: BTreeMap<String, Option<String>>,
    destination: String,
}

#[derive(Debug, PartialEq, Eq)]
struct MergedParent {
    pr: RecoveryPullRequest,
    merge: String,
}

impl Recovery {
    pub(super) fn observe(
        github: &GitHub<'_>,
        stack: &[Commit],
        start: usize,
        bases: &[String],
    ) -> Result<Self> {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        let mut recovery = Self {
            entries: Vec::new(),
            parents: BTreeMap::new(),
            refs: BTreeMap::new(),
            destination: git.recovery_destination()?,
        };
        let mut references = BTreeSet::from([github.config.repo.trunk.clone()]);
        for (index, commit) in stack
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, commit)| !commit.skip)
        {
            let reference = commit.remote_ref();
            let pr = github
                .publication_pr(reference)?
                .map(|expected| {
                    let pr = github.recovery_pr(expected.number)?;
                    policy::require_recovery_open(&pr)?;
                    if pr.node_id != expected.node_id
                        || pr.head.name != reference
                        || pr.base.name != expected.base.name
                    {
                        bail!("PR #{} changed during recovery observation", pr.number);
                    }
                    Ok(pr)
                })
                .transpose()?;
            references.insert(reference.to_owned());
            references.insert(bases[index].clone());
            if let Some(pr) = &pr {
                references.insert(pr.base.name.clone());
                if pr.base.name != bases[index] {
                    if bases[index] != github.config.repo.trunk {
                        bail!("base recovery cannot reorder unmerged PR #{}", pr.number);
                    }
                    GitHub::require_recoverable_membership(pr)?;
                    let parent = github.recovery_parent(&pr.base.name)?;
                    let merge = policy::recovery_merge(&parent, &pr.base.name)?;
                    recovery
                        .parents
                        .insert(pr.base.name.clone(), MergedParent { pr: parent, merge });
                }
            }
            recovery.entries.push(Entry {
                index,
                reference: reference.to_owned(),
                pr,
            });
        }
        recovery.refs = git.recovery_refs(
            &recovery.destination,
            &references.into_iter().collect::<Vec<_>>(),
        )?;
        recovery.check_evidence(git)?;
        recovery.check_graph(git, stack, bases)?;
        recovery.check_dependants(github, bases)?;
        Ok(recovery)
    }

    fn tip(&self, reference: &str) -> Result<&str> {
        self.refs
            .get(reference)
            .and_then(Option::as_deref)
            .with_context(|| format!("missing remote recovery base {reference:?}"))
    }

    fn check_evidence(&self, git: Git<'_>) -> Result<()> {
        let trunk = self.tip(&git.repo.trunk)?;
        for entry in &self.entries {
            if let Some(pr) = &entry.pr {
                if self.tip(&entry.reference)? != pr.head.sha {
                    bail!(
                        "published head for PR #{} disagrees with its live ref",
                        pr.number
                    );
                }
                if let Some(parent) = self.parents.get(&pr.base.name) {
                    if self
                        .refs
                        .get(&pr.base.name)
                        .and_then(Option::as_deref)
                        .is_some_and(|oid| oid != parent.pr.head.sha)
                    {
                        bail!(
                            "obsolete base {:?} was reused or advanced after its recorded merge",
                            pr.base.name
                        );
                    }
                    if !git.is_ancestor(&parent.merge, trunk)?
                        || !git.is_ancestor(&parent.pr.head.sha, &pr.head.sha)?
                    {
                        bail!(
                            "merged downstack PR #{} cannot be verified in trunk and the surviving published history; synchronise objects before retrying",
                            parent.pr.number
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn check_graph(&self, git: Git<'_>, stack: &[Commit], bases: &[String]) -> Result<()> {
        // Check all old/new combinations, covering every partial push batch state.
        for entry in &self.entries {
            let new_head = &stack[entry.index].hash;
            let base = &bases[entry.index];
            let new_base = self
                .entries
                .iter()
                .find(|candidate| candidate.reference == *base)
                .map_or_else(
                    || self.tip(base),
                    |candidate| Ok(stack[candidate.index].hash.as_str()),
                )?;
            if !git.is_ancestor(new_base, new_head)? {
                bail!(
                    "prepared head {:?} is not based on the verified publication base; synchronise before retrying",
                    entry.reference
                );
            }
            for head in [
                entry.pr.as_ref().map(|pr| pr.head.sha.as_str()),
                Some(new_head.as_str()),
            ]
            .into_iter()
            .flatten()
            {
                for base_tip in [
                    self.refs.get(base).and_then(Option::as_deref),
                    Some(new_base),
                ]
                .into_iter()
                .flatten()
                {
                    if git.is_ancestor(head, base_tip)? {
                        bail!(
                            "base recovery would make head {:?} reachable from its base; refusing an indirect merge",
                            entry.reference
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn check_dependants(&self, github: &GitHub<'_>, bases: &[String]) -> Result<()> {
        let selected: HashMap<_, _> = self
            .entries
            .iter()
            .filter_map(|entry| {
                entry.pr.as_ref().map(|pr| {
                    (
                        pr.number,
                        (entry.reference.as_str(), bases[entry.index].as_str()),
                    )
                })
            })
            .collect();
        github.check_publication_dependants(
            &self
                .entries
                .iter()
                .map(|entry| entry.reference.clone())
                .collect::<Vec<_>>(),
            &selected,
        )
    }

    pub(super) fn apply(&self, github: &GitHub<'_>, state: &State, bases: &[String]) -> Result<()> {
        let selected: Vec<_> = state
            .stack
            .iter()
            .enumerate()
            .skip(state.start)
            .filter(|(_, commit)| !commit.skip)
            .collect();
        if bases.len() != state.stack.len()
            || selected.len() != self.entries.len()
            || selected
                .iter()
                .zip(&self.entries)
                .any(|((index, commit), entry)| {
                    *index != entry.index || commit.remote_ref() != entry.reference
                })
        {
            bail!("publication selection changed before base recovery");
        }
        let references: Vec<_> = self
            .entries
            .iter()
            .map(|entry| entry.reference.clone())
            .collect();
        github.invalidate_heads(&references);
        github.observe(&references)?;
        let current = Self::observe(github, &state.stack, state.start, bases)?;
        if self.refs != current.refs
            || self.destination != current.destination
            || self.parents != current.parents
        {
            bail!(
                "remote refs, destination or downstack evidence changed before base recovery; retry from fresh state"
            );
        }
        for (expected, actual) in self.entries.iter().zip(&current.entries) {
            match (&expected.pr, &actual.pr) {
                (Some(expected), Some(actual)) => {
                    policy::same_recovery_head(expected, actual, &expected.base.name)?;
                }
                (None, None) => {}
                _ => bail!("PR identity changed before base recovery"),
            }
        }
        for entry in &current.entries {
            if let Some(pr) = &entry.pr
                && pr.base.name != bases[entry.index]
            {
                github.retarget_recovery(pr, &bases[entry.index])?;
            }
        }
        self.verify_heads(github, state, bases, false)?;
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        if git.recovery_destination()? != self.destination
            || git.recovery_refs(
                &self.destination,
                &self.refs.keys().cloned().collect::<Vec<_>>(),
            )? != self.refs
        {
            bail!(
                "remote refs or destination changed after base recovery; repaired bases are retained and publication stopped"
            );
        }
        self.check_dependants(github, bases)?;
        github.invalidate_heads(&references);
        Ok(())
    }

    fn verify_heads(
        &self,
        github: &GitHub<'_>,
        state: &State,
        bases: &[String],
        published: bool,
    ) -> Result<()> {
        for entry in &self.entries {
            if let Some(pr) = &entry.pr {
                let mut expected = pr.clone();
                if published {
                    expected.head.sha.clone_from(&state.stack[entry.index].hash);
                }
                policy::same_recovery_head(
                    &expected,
                    &github.recovery_pr(pr.number)?,
                    &bases[entry.index],
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn verify_published(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        let bases = crate::stack::selection::resolve_bases(
            &state.stack,
            state.start,
            &github.config.repo.trunk,
        );
        self.verify_heads(github, state, &bases, true)
    }
}
