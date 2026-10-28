//! Verified publication transitions, including interrupted operations.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::state::State;
use crate::{
    git::Git,
    github::{GitHub, policy},
    model::{Commit, pull_request::RecoveryPullRequest},
};

const JOURNAL: &str = "git-pr.transitionJournal";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Entry {
    index: usize,
    reference: String,
    prepared: String,
    pr: Option<RecoveryPullRequest>,
}

#[cfg(test)]
#[path = "../tests/unit/base_recovery.rs"]
mod unit;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Recovery {
    entries: Vec<Entry>,
    parents: BTreeMap<String, MergedParent>,
    pub(super) refs: BTreeMap<String, Option<String>>,
    destination: String,
    repository: String,
    bases: Vec<String>,
    staging: Option<String>,
    native: Option<u64>,
    anchor: String,
    link_native: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
        if !github.config.transition_journal.is_empty() {
            return Self::resume(github, stack, start, bases);
        }
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        let mut recovery = Self {
            entries: Vec::new(),
            parents: BTreeMap::new(),
            refs: BTreeMap::new(),
            destination: git.recovery_destination()?,
            repository: github.config.repo.name.clone(),
            bases: bases.to_vec(),
            staging: None,
            native: None,
            anchor: String::new(),
            link_native: github.config.github_stack,
        };
        let selected: BTreeSet<_> = stack
            .iter()
            .skip(start)
            .filter(|c| !c.skip)
            .map(Commit::remote_ref)
            .collect();
        let mut references = BTreeSet::from([github.config.repo.trunk.clone()]);
        for (index, commit) in stack
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, c)| !c.skip)
        {
            let reference = commit.remote_ref();
            let pr = Self::observe_pr(github, reference)?;
            references.insert(reference.to_owned());
            references.insert(bases[index].clone());
            if let Some(pr) = &pr {
                references.insert(pr.base.name.clone());
                if pr.base.name != bases[index]
                    && pr.base.name != git.repo.trunk
                    && !selected.contains(pr.base.name.as_str())
                {
                    // Removing an unselected predecessor requires proof that it landed.
                    if bases[index] != git.repo.trunk {
                        bail!(
                            "include the complete published stack before changing PR #{}",
                            pr.number
                        );
                    }
                    recovery.observe_parent(github, pr)?;
                }
            }
            recovery.entries.push(Entry {
                index,
                reference: reference.to_owned(),
                prepared: commit.hash.clone(),
                pr,
            });
        }
        recovery.refs = git.recovery_refs(
            &recovery.destination,
            &references.into_iter().collect::<Vec<_>>(),
        )?;
        recovery.anchor = recovery.check_evidence(git)?;
        recovery.check_final_graph(git, stack)?;
        recovery.check_dependants(github, false)?;
        recovery.native = recovery.observe_native(github)?;
        if !recovery.direct_is_safe(git, stack)? {
            let identity = recovery
                .entries
                .iter()
                .filter_map(|e| e.pr.as_ref().map(|p| p.number))
                .min()
                .context("publication transition has no existing PR identity")?;
            let reference = format!("git-pr/transition/{identity}");
            let observed =
                git.recovery_refs(&recovery.destination, std::slice::from_ref(&reference))?;
            if observed[&reference].is_some() {
                bail!(
                    "temporary publication base {reference:?} already exists without a matching journal"
                );
            }
            recovery.refs.extend(observed);
            recovery.staging = Some(reference);
            recovery.check_staging(git, stack)?;
        }
        Ok(recovery)
    }

    fn observe_pr(github: &GitHub<'_>, reference: &str) -> Result<Option<RecoveryPullRequest>> {
        github
            .publication_pr(reference)?
            .map(|expected| {
                let pr = github.recovery_pr(expected.number).with_context(|| {
                    format!(
                        "cannot observe PR #{} for publication transition",
                        expected.number
                    )
                })?;
                policy::require_recovery_open(&pr)?;
                if pr.node_id != expected.node_id
                    || pr.head.name != reference
                    || pr.base.name != expected.base.name
                {
                    bail!("PR #{} changed during recovery observation", pr.number);
                }
                Ok(pr)
            })
            .transpose()
    }

    fn observe_parent(&mut self, github: &GitHub<'_>, pr: &RecoveryPullRequest) -> Result<()> {
        GitHub::require_recoverable_membership(pr)?;
        let parent = github.recovery_parent(&pr.base.name)?;
        let merge = policy::recovery_merge(&parent, &pr.base.name)?;
        self.parents
            .insert(pr.base.name.clone(), MergedParent { pr: parent, merge });
        Ok(())
    }

    fn tip(&self, reference: &str) -> Result<&str> {
        self.refs
            .get(reference)
            .and_then(Option::as_deref)
            .with_context(|| format!("missing remote recovery base {reference:?}"))
    }

    fn check_evidence(&self, git: Git<'_>) -> Result<String> {
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
        Ok(trunk.to_owned())
    }

    fn desired_tip<'a>(&'a self, reference: &str, stack: &'a [Commit]) -> Result<&'a str> {
        self.entries
            .iter()
            .find(|e| e.reference == reference)
            .map_or_else(|| self.tip(reference), |e| Ok(stack[e.index].hash.as_str()))
    }

    fn check_final_graph(&self, git: Git<'_>, stack: &[Commit]) -> Result<()> {
        for entry in &self.entries {
            let head = &stack[entry.index].hash;
            let base = self.desired_tip(&self.bases[entry.index], stack)?;
            if head == base || !git.is_ancestor(base, head)? {
                bail!(
                    "prepared head {:?} is not strictly above its verified publication base; synchronise before retrying",
                    entry.reference
                );
            }
        }
        Ok(())
    }

    fn direct_is_safe(&self, git: Git<'_>, stack: &[Commit]) -> Result<bool> {
        // Every old/new combination must be safe, including partial push batches.
        for entry in &self.entries {
            let base = &self.bases[entry.index];
            let Some(old_base) = self.refs.get(base).and_then(Option::as_deref) else {
                return Ok(false);
            };
            let new_base = self
                .entries
                .iter()
                .find(|entry| entry.reference == *base)
                .map_or(old_base, |entry| stack[entry.index].hash.as_str());
            for head in [
                entry.pr.as_ref().map(|p| p.head.sha.as_str()),
                Some(stack[entry.index].hash.as_str()),
            ]
            .into_iter()
            .flatten()
            {
                for base_tip in [old_base, new_base] {
                    if git.is_ancestor(head, base_tip)? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    fn check_staging(&self, git: Git<'_>, stack: &[Commit]) -> Result<()> {
        let anchor = &self.anchor;
        for entry in &self.entries {
            for head in [
                entry.pr.as_ref().map(|p| p.head.sha.as_str()),
                Some(stack[entry.index].hash.as_str()),
            ]
            .into_iter()
            .flatten()
            {
                if git.is_ancestor(head, anchor)? {
                    bail!(
                        "temporary publication base would indirectly merge head {:?}",
                        entry.reference
                    );
                }
            }
        }
        Ok(())
    }

    fn check_dependants(&self, github: &GitHub<'_>, prepared: bool) -> Result<()> {
        let selected: HashMap<_, _> = self
            .entries
            .iter()
            .filter_map(|entry| {
                entry.pr.as_ref().map(|pr| {
                    let base = if prepared {
                        self.publication_base(entry.index)
                    } else {
                        &pr.base.name
                    };
                    (pr.number, (entry.reference.as_str(), base))
                })
            })
            .collect();
        github.check_publication_dependants(
            &self
                .refs
                .keys()
                .filter(|&r| r != &github.config.repo.trunk)
                .cloned()
                .collect::<Vec<_>>(),
            &selected,
        )
    }

    fn observe_native(&self, github: &GitHub<'_>) -> Result<Option<u64>> {
        let stacks: BTreeSet<_> = self
            .entries
            .iter()
            .filter_map(|e| e.pr.as_ref()?.stack.as_ref().map(|s| s.number))
            .collect();
        if stacks.len() > 1 {
            bail!("publication transition spans multiple native stacks")
        }
        let number = stacks.first().copied();
        if let Some(number) = number {
            let claimed: Vec<_> = self
                .entries
                .iter()
                .filter_map(|entry| entry.pr.as_ref())
                .filter(|pr| pr.stack.is_some())
                .map(|pr| pr.number)
                .collect();
            github.check_transition_stack(number, &self.numbers(), &claimed)?;
        }
        Ok(number)
    }

    fn numbers(&self) -> Vec<u64> {
        self.entries
            .iter()
            .filter_map(|e| e.pr.as_ref().map(|p| p.number))
            .collect()
    }

    pub(super) fn publication_base(&self, index: usize) -> &str {
        self.staging.as_deref().unwrap_or(&self.bases[index])
    }

    pub(super) const fn restore_native(&self) -> bool {
        self.native.is_some() || self.link_native
    }

    fn validate_selection(&self, stack: &[Commit], start: usize, bases: &[String]) -> Result<()> {
        let selected: Vec<_> = stack
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, c)| !c.skip)
            .collect();
        if self.bases != bases
            || selected.len() != self.entries.len()
            || selected
                .iter()
                .zip(&self.entries)
                .any(|((index, commit), entry)| {
                    *index != entry.index || commit.remote_ref() != entry.reference
                })
        {
            bail!("publication selection changed before base recovery; resume the recorded stack");
        }
        Ok(())
    }

    pub(super) fn apply(&self, github: &GitHub<'_>, state: &State, bases: &[String]) -> Result<()> {
        self.validate_selection(&state.stack, state.start, bases)?;
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        self.check_final_graph(git, &state.stack)?;
        let mut current = self.fresh(github, &state.stack, false)?;
        if self.restore_native() {
            let base = state
                .stack
                .iter()
                .enumerate()
                .skip(state.start)
                .find(|(_, commit)| !commit.skip)
                .map_or(github.config.repo.trunk.as_str(), |(index, _)| {
                    bases[index].as_str()
                });
            github.check_stack_eligibility(state.selected(), base, true)?;
        }
        if self.staging.is_none() && !self.direct_is_safe(git, &state.stack)? {
            bail!(
                "prepared commits require a different publication transition; retry from fresh state"
            );
        }
        if self.staging.is_some() {
            self.check_staging(git, &state.stack)?;
        }
        let mut journal = self.clone();
        for entry in &mut journal.entries {
            entry.prepared.clone_from(&state.stack[entry.index].hash);
        }
        github.commands.git(&[
            "config",
            "--local",
            "--replace-all",
            JOURNAL,
            &serde_json::json!(journal).to_string(),
        ])?;
        if let Some(number) = current.native {
            github.unstack_transition(number)?;
            current = self.fresh(github, &state.stack, false)?;
            if current
                .entries
                .iter()
                .any(|e| e.pr.as_ref().is_some_and(|p| p.stack.is_some()))
            {
                bail!(
                    "native stack transition is incomplete; publication stopped and journal retained"
                );
            }
            github
                .commands
                .outcomes
                .confirm(crate::model::outcome::Operation::NativeStack);
        }
        if let Some(reference) = &self.staging {
            let anchor = Commit {
                hash: self.anchor.clone(),
                attrs: vec![("remote-ref".into(), reference.clone())],
                ..Commit::default()
            };
            git.push_with_leases(&[&anchor], Some(&current.refs))?;
        }
        for entry in &current.entries {
            if let Some(pr) = &entry.pr {
                let base = self.publication_base(entry.index);
                if pr.base.name != base {
                    github.retarget_recovery(pr, base)?;
                }
            }
        }
        self.verify_heads(github, state, false)?;
        self.fresh_refs(git, &state.stack)?;
        self.check_dependants(github, true)?;
        github.invalidate_heads(
            &self
                .entries
                .iter()
                .map(|e| e.reference.clone())
                .collect::<Vec<_>>(),
        );
        Ok(())
    }

    fn fresh_refs(
        &self,
        git: Git<'_>,
        stack: &[Commit],
    ) -> Result<BTreeMap<String, Option<String>>> {
        if git.recovery_destination()? != self.destination {
            bail!("publication destination changed during base recovery")
        }
        let refs = git.recovery_refs(
            &self.destination,
            &self.refs.keys().cloned().collect::<Vec<_>>(),
        )?;
        for (reference, original) in &self.refs {
            let actual = &refs[reference];
            let intended = self
                .entries
                .iter()
                .find(|e| &e.reference == reference)
                .map(|e| &stack[e.index].hash)
                .or_else(|| (self.staging.as_ref() == Some(reference)).then_some(&self.anchor));
            if actual != original
                && !(intended.is_some() && actual.as_ref() == intended)
                && !(self.staging.as_ref() == Some(reference) && actual.is_none())
            {
                bail!("remote ref {reference:?} changed during base recovery; journal retained");
            }
        }
        Ok(refs)
    }

    fn fresh(&self, github: &GitHub<'_>, stack: &[Commit], resuming: bool) -> Result<Self> {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        let refs = self.fresh_refs(git, stack)?;
        let references: Vec<_> = self.entries.iter().map(|e| e.reference.clone()).collect();
        github.invalidate_heads(&references);
        github.observe(&references)?;
        for parent in self.parents.values() {
            if github.recovery_pr(parent.pr.number)? != parent.pr {
                bail!("merged downstack evidence changed before base recovery");
            }
        }
        let mut current = self.clone();
        current.refs = refs;
        for entry in &mut current.entries {
            let observed = github.publication_pr(&entry.reference)?;
            let Some(expected) = &entry.pr else {
                if let Some(observed) = observed {
                    if !resuming {
                        bail!("PR identity changed before base recovery");
                    }
                    let actual = github.recovery_pr(observed.number)?;
                    if actual.head.sha != entry.prepared
                        || actual.base.name != self.bases[entry.index]
                    {
                        bail!("new PR does not match the recorded publication transition");
                    }
                    entry.pr = Some(actual);
                }
                continue;
            };
            if observed.as_ref().map(|p| (p.number, &p.node_id))
                != Some((expected.number, &expected.node_id))
            {
                bail!("PR identity changed before base recovery");
            }
            let actual = github.recovery_pr(expected.number)?;
            let mut allowed = expected.clone();
            if resuming {
                allowed.head.sha = current.refs[&entry.reference]
                    .clone()
                    .context("missing resumed publication head")?;
                if actual.base.name == self.bases[entry.index]
                    || Some(&actual.base.name) == self.staging.as_ref()
                {
                    allowed.base.name.clone_from(&actual.base.name);
                }
            }
            if actual.stack.is_none() {
                allowed.stack = None;
            } else if resuming
                && actual.head.sha == entry.prepared
                && actual.base.name == self.bases[entry.index]
            {
                allowed.stack.clone_from(&actual.stack);
            }
            policy::same_recovery_head(&allowed, &actual, &allowed.base.name)?;
            entry.pr = Some(actual);
        }
        current.check_dependants(github, false)?;
        current.native = current.observe_native(github)?;
        Ok(current)
    }

    fn resume(
        github: &GitHub<'_>,
        stack: &[Commit],
        start: usize,
        bases: &[String],
    ) -> Result<Self> {
        let saved: Self = serde_json::from_str(&github.config.transition_journal)
            .context("invalid publication transition journal; retain it for inspection")?;
        saved.validate_selection(stack, start, bases)?;
        if !crate::git::policy::valid_oid(&saved.anchor)
            || saved
                .refs
                .get(&github.config.repo.trunk)
                .and_then(Option::as_deref)
                != Some(saved.anchor.as_str())
            || saved.entries.iter().any(|entry| {
                !saved.refs.contains_key(&entry.reference)
                    || !saved.refs.contains_key(&saved.bases[entry.index])
            })
            || saved.staging.as_ref().is_some_and(|reference| {
                !saved.refs.contains_key(reference)
                    || saved
                        .numbers()
                        .into_iter()
                        .min()
                        .map(|number| format!("git-pr/transition/{number}"))
                        .as_ref()
                        != Some(reference)
            })
        {
            bail!("invalid publication transition journal metadata; retain it for inspection");
        }
        if saved.repository != github.config.repo.name
            || saved
                .entries
                .iter()
                .any(|e| e.prepared != stack[e.index].hash)
        {
            bail!(
                "recorded publication transition does not match the repository or prepared commits"
            );
        }
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        saved.check_final_graph(git, stack)?;
        let mut resumed = saved.fresh(github, stack, true)?;
        resumed.native = saved.native;
        Ok(resumed)
    }

    fn verify_heads(&self, github: &GitHub<'_>, state: &State, published: bool) -> Result<()> {
        for entry in &self.entries {
            if let Some(pr) = &entry.pr {
                let mut expected = pr.clone();
                expected.stack = None;
                if published {
                    expected.head.sha.clone_from(&state.stack[entry.index].hash);
                }
                policy::same_recovery_head(
                    &expected,
                    &github.recovery_pr(pr.number)?,
                    if published {
                        &self.bases[entry.index]
                    } else {
                        self.publication_base(entry.index)
                    },
                )?;
            }
        }
        Ok(())
    }

    fn require_published_refs(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        let refs = self.fresh_refs(git, &state.stack)?;
        for entry in &self.entries {
            if refs[&entry.reference].as_deref() != Some(state.stack[entry.index].hash.as_str()) {
                bail!("publication transition has an unpublished head; PR reconciliation stopped");
            }
        }
        Ok(())
    }

    pub(super) fn complete(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        self.require_published_refs(github, state)?;
        // All final refs are now fixed; no old/new combinations remain.
        for entry in &self.entries {
            if let Some(pr) = &entry.pr {
                let mut expected = pr.clone();
                expected.stack = None;
                expected.head.sha.clone_from(&state.stack[entry.index].hash);
                self.publication_base(entry.index)
                    .clone_into(&mut expected.base.name);
                if expected.base.name != self.bases[entry.index] {
                    github.retarget_recovery(&expected, &self.bases[entry.index])?;
                }
            }
        }
        self.verify_published(github, state)
    }

    pub(super) fn verify_published(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        self.verify_heads(github, state, true)
    }

    pub(super) fn finish(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        self.require_published_refs(github, state)?;
        for entry in &self.entries {
            if let Some(pr) = &entry.pr {
                let actual = github.recovery_pr(pr.number)?;
                let mut expected = pr.clone();
                expected.head.sha.clone_from(&state.stack[entry.index].hash);
                expected.stack.clone_from(&actual.stack);
                policy::same_recovery_head(&expected, &actual, &self.bases[entry.index])?;
            }
        }
        if let Some(reference) = &self.staging {
            github
                .check_publication_dependants(std::slice::from_ref(reference), &HashMap::new())?;
            let expected = &self.anchor;
            github.commands.git(&[
                "push",
                "--porcelain",
                "--no-follow-tags",
                &format!("--force-with-lease=refs/heads/{reference}:{expected}"),
                "--",
                &git.repo.remote,
                &format!(":refs/heads/{reference}"),
            ])?;
        }
        github
            .commands
            .git(&["config", "--local", "--unset-all", JOURNAL])?;
        Ok(())
    }
}
