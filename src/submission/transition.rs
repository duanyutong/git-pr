//! Observation and guarded execution of publication-base transitions.

use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Instant,
};

use super::{
    plan::{
        Ancestry, DesiredPublication, ObservedPublication, PublicationPlan, Strategy,
        temporary_base, temporary_owner,
    },
    state::State,
};
use crate::{
    git::Git,
    github::{GitHub, policy},
    model::{Commit, pull_request::RecoveryPullRequest},
};

#[derive(Clone, Debug)]
struct Entry {
    index: usize,
    reference: String,
    pr: Option<RecoveryPullRequest>,
}

#[cfg(test)]
#[path = "../tests/unit/base_recovery.rs"]
mod unit;

#[derive(Clone, Debug)]
pub(super) struct Transition {
    entries: Vec<Entry>,
    parents: BTreeMap<String, MergedParent>,
    pub(super) refs: Arc<BTreeMap<String, Option<String>>>,
    destination: String,
    bases: Vec<String>,
    pub(super) plan: Arc<PublicationPlan>,
    temporary: BTreeMap<String, String>,
    native: Option<(u64, Instant)>,
    anchor: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MergedParent {
    pr: RecoveryPullRequest,
    merge: String,
}

impl Transition {
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
            refs: Arc::default(),
            destination: git.recovery_destination()?,
            bases: bases.to_vec(),
            plan: Arc::default(),
            temporary: BTreeMap::new(),
            native: None,
            anchor: git.trunk_snapshot()?,
        };
        let selected: BTreeSet<_> = stack
            .iter()
            .skip(start)
            .filter(|c| !c.skip)
            .map(Commit::remote_ref)
            .collect();
        let mut observed = Self::observe_prs(github, stack, start)?;
        let mut references = BTreeSet::new();
        let mut obsolete = Vec::new();
        for (index, commit) in stack
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, c)| !c.skip)
        {
            let reference = commit.remote_ref();
            let pr = observed.remove(reference);
            references.insert(reference.to_owned());
            references.insert(bases[index].clone());
            if let Some(pr) = &pr {
                references.insert(pr.base.name.clone());
                if pr.base.name != bases[index]
                    && pr.base.name != git.repo.trunk
                    && !selected.contains(pr.base.name.as_str())
                {
                    obsolete.push((index, pr.clone()));
                }
            }
            recovery.entries.push(Entry {
                index,
                reference: reference.to_owned(),
                pr,
            });
        }
        let numbers = recovery.numbers();
        let reserved = temporary_base(numbers.iter().copied());
        if let Some(reference) = &reserved {
            references.insert(reference.clone());
        }
        for (index, pr) in obsolete {
            if temporary_owner(&pr.base.name).is_some_and(|number| numbers.contains(&number)) {
                continue;
            }
            if bases[index] != git.repo.trunk {
                bail!(
                    "include the complete published stack before changing PR #{}",
                    pr.number
                );
            }
            recovery.observe_parent(github, &pr)?;
        }
        // Observe mutable development/helper refs that the transition changes or
        // depends upon. Trunk is deliberately excluded: PR bases name a branch,
        // and unrelated merges must not invalidate a submission at any stage.
        // The separate local anchor supplies all trunk-related graph evidence.
        recovery.refs = Arc::new(
            git.recovery_refs(
                &recovery.destination,
                &references
                    .into_iter()
                    .filter(|reference| reference != &git.repo.trunk)
                    .collect::<Vec<_>>(),
            )?,
        );
        recovery.check_evidence(git)?;
        recovery.observe_temporary(git, &numbers, reserved)?;
        recovery.check_dependants(github, false, None)?;
        recovery.native = recovery.observe_native(github, None)?;
        Ok(recovery)
    }

    pub(super) fn prepare_staging(&mut self, git: Git<'_>) -> Result<()> {
        let Strategy::Stage(reference) = &self.plan.strategy else {
            return Ok(());
        };
        let reference = reference.clone();
        let observed = git.recovery_refs(&self.destination, std::slice::from_ref(&reference))?;
        if let Some(tip) = observed.get(&reference).and_then(Option::as_ref) {
            self.adopt_temporary(git, reference.clone(), tip.clone())?;
        }
        Arc::make_mut(&mut self.refs).extend(observed);
        self.temporary.insert(reference, self.anchor.clone());
        Ok(())
    }

    fn observe_temporary(
        &mut self,
        git: Git<'_>,
        numbers: &[u64],
        reserved: Option<String>,
    ) -> Result<()> {
        let temporary = self
            .entries
            .iter()
            .filter_map(|entry| entry.pr.as_ref())
            .map(|pr| pr.base.name.clone())
            .filter(|reference| {
                temporary_owner(reference).is_some_and(|number| numbers.contains(&number))
            })
            .chain(reserved)
            .collect::<BTreeSet<_>>();
        for reference in temporary {
            if let Some(tip) = self.refs.get(&reference).and_then(Option::as_ref).cloned() {
                self.adopt_temporary(git, reference, tip)?;
            }
        }
        Ok(())
    }

    fn adopt_temporary(&mut self, git: Git<'_>, reference: String, tip: String) -> Result<()> {
        if !git.is_ancestor(&tip, &self.anchor)? {
            bail!(
                "reserved temporary publication base {reference:?} is not a verified trunk ancestor; refusing to reuse or remove it"
            );
        }
        self.temporary.insert(reference, tip);
        Ok(())
    }

    fn observe_prs(
        github: &GitHub<'_>,
        stack: &[Commit],
        start: usize,
    ) -> Result<BTreeMap<String, RecoveryPullRequest>> {
        let expected = stack
            .iter()
            .skip(start)
            .filter(|c| !c.skip)
            .map(|c| github.publication_pr(c.remote_ref()))
            .collect::<Result<Vec<_>>>()?;
        let observed = github.recovery_prs(
            &expected
                .iter()
                .flatten()
                .map(|p| p.number)
                .collect::<Vec<_>>(),
        )?;
        if expected.iter().flatten().count() != observed.len() {
            bail!("incomplete recovery identity observation");
        }
        expected
            .into_iter()
            .flatten()
            .zip(observed)
            .map(|(expected, pr)| {
                policy::require_recovery_open(&pr)?;
                if pr.number != expected.number
                    || pr.node_id != expected.node_id
                    || pr.head.name != expected.head.name
                    || pr.base.name != expected.base.name
                {
                    bail!("PR #{} changed during recovery observation", pr.number);
                }
                Ok((pr.head.name.clone(), pr))
            })
            .collect()
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

    fn check_evidence(&self, git: Git<'_>) -> Result<()> {
        let trunk = &self.anchor;
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

    fn desired<'a>(&'a self, stack: &'a [Commit], link_native: bool) -> DesiredPublication<'a> {
        DesiredPublication::new(
            stack,
            self.entries
                .first()
                .map_or(stack.len(), |entry| entry.index),
            &self.bases,
            link_native,
        )
    }

    pub(super) fn observed<'a>(&'a self, trunk: &'a str) -> ObservedPublication<'a> {
        ObservedPublication {
            refs: &self.refs,
            prs: self
                .entries
                .iter()
                .filter_map(|entry| {
                    entry
                        .pr
                        .as_ref()
                        .map(|pr| (entry.index, (pr.number, pr.base.name.clone())))
                })
                .collect(),
            trunk,
            anchor: &self.anchor,
        }
    }

    pub(super) fn anchor(&self) -> &str {
        &self.anchor
    }

    pub(super) fn ancestry(&self, git: Git<'_>, stack: &[Commit]) -> Result<Ancestry> {
        let pairs = Ancestry::requirements(
            &self.desired(stack, self.plan.link_native),
            &self.observed(&git.repo.trunk),
        )?;
        if !git.complete_graph()? {
            return git.scoped_ancestry(&pairs).map(Ancestry);
        }
        crate::commands::parallel_cached(
            pairs.into_iter().collect(),
            |_| None,
            |(ancestor, descendant)| {
                git.is_ancestor_native(&ancestor, &descendant)
                    .map(|answer| ((ancestor, descendant), answer))
            },
        )
        .map(|answers| Ancestry(answers.into_iter().collect()))
    }

    fn staging(&self) -> Option<&str> {
        if let Strategy::Stage(reference) = &self.plan.strategy {
            Some(reference)
        } else {
            None
        }
    }

    pub(super) const fn cleanup_candidates(&self) -> &BTreeMap<String, String> {
        &self.temporary
    }

    pub(super) fn destination(&self) -> &str {
        &self.destination
    }

    fn check_dependants(
        &self,
        github: &GitHub<'_>,
        prepared: bool,
        started: Option<Instant>,
    ) -> Result<()> {
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
        github.check_publication_dependants_with_budget(
            &self
                .refs
                .keys()
                .filter(|&r| r != &github.config.repo.trunk)
                .cloned()
                .collect::<Vec<_>>(),
            &selected,
            started,
        )
    }

    fn observe_native(
        &self,
        github: &GitHub<'_>,
        started: Option<Instant>,
    ) -> Result<Option<(u64, Instant)>> {
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
            return github
                .check_transition_stack(number, &self.numbers(), &claimed, started)
                .map(|acquired| Some((number, acquired)));
        }
        Ok(None)
    }

    fn numbers(&self) -> Vec<u64> {
        self.entries
            .iter()
            .filter_map(|e| e.pr.as_ref().map(|p| p.number))
            .collect()
    }

    pub(super) fn publication_base(&self, index: usize) -> &str {
        self.staging().unwrap_or(&self.bases[index])
    }

    pub(super) const fn changes_native(&self) -> bool {
        self.native.is_some()
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
            bail!(
                "publication selection changed during base recovery; retry from the current local stack"
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn apply(&self, github: &GitHub<'_>, state: &State, bases: &[String]) -> Result<()> {
        self.apply_with_ancestry(github, state, bases, None)
            .map(|_| ())
    }

    fn check_strategy(
        &self,
        git: Git<'_>,
        stack: &[Commit],
        prior: Option<&Ancestry>,
    ) -> Result<()> {
        let desired = self.desired(stack, self.plan.link_native);
        let observed = self.observed(&git.repo.trunk);
        let requirements = Ancestry::requirements(&desired, &observed)?;
        let reusable = prior.filter(|answers| answers.0.keys().eq(requirements.iter()));
        let current;
        let ancestry = match reusable {
            Some(prior) if git.complete_graph()? => prior,
            _ => {
                current = self.ancestry(git, stack)?;
                &current
            }
        };
        ancestry.verify_strategy(&self.plan.strategy, &desired, &observed)
    }

    /// Return the start of the final dependant observation for publication dispatch.
    pub(super) fn apply_with_ancestry(
        &self,
        github: &GitHub<'_>,
        state: &State,
        bases: &[String],
        ancestry: Option<&Ancestry>,
    ) -> Result<Instant> {
        self.validate_selection(&state.stack, state.start, bases)?;
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        self.check_strategy(git, &state.stack, ancestry)?;
        let (current, dependants_acquired) = self.prepare_native(github, state, bases)?;
        if let Some(reference) = self.staging() {
            let anchor = Commit {
                hash: self.anchor.clone(),
                attrs: vec![("remote-ref".into(), reference.to_owned())],
                ..Commit::default()
            };
            git.push_with_leases(&[&anchor], Some(&current.refs), &|| {
                super::policy::require_fresh_publication(github.now(), [dependants_acquired])
            })?;
        }
        let changes: Vec<_> = current
            .entries
            .iter()
            .filter_map(|entry| entry.pr.as_ref().map(|pr| (entry, pr)))
            .filter_map(|(entry, pr)| {
                let base = self.publication_base(entry.index);
                (pr.base.name != base).then(|| (pr.clone(), base.to_owned()))
            })
            .collect();
        github
            .commands
            .diagnostics
            .phase("PR base transitions", || {
                github.retarget_recoveries(&changes)
            })?;
        let (_, dependants_acquired) = crate::commands::parallel_pair(
            || self.fresh_refs(git, &state.stack),
            || {
                self.verify_heads(github, state, false)?;
                let acquired = github.now();
                self.check_dependants(github, true, None)?;
                Ok(acquired)
            },
        )?;
        github.invalidate_heads(
            &self
                .entries
                .iter()
                .map(|e| e.reference.clone())
                .collect::<Vec<_>>(),
        );
        Ok(dependants_acquired)
    }

    fn prepare_native(
        &self,
        github: &GitHub<'_>,
        state: &State,
        bases: &[String],
    ) -> Result<(Self, Instant)> {
        let started = github.now();
        let mut attempt = 0;
        loop {
            let (current, dependants_acquired) = self.fresh(github, &state.stack, started)?;
            if self.changes_native() {
                let base = state
                    .stack
                    .iter()
                    .enumerate()
                    .skip(state.start)
                    .find(|(_, commit)| !commit.skip)
                    .map_or(github.config.repo.trunk.as_str(), |(index, _)| {
                        bases[index].as_str()
                    });
                github.check_stack_eligibility_with_budget(
                    state.selected(),
                    base,
                    true,
                    Some(started),
                )?;
            }
            let Some((number, acquired)) = current.native else {
                return Ok((current, dependants_acquired));
            };
            match github.unstack_transition(number, acquired, started) {
                Ok(()) => {
                    let (current, dependants_acquired) =
                        self.fresh(github, &state.stack, started)?;
                    if current
                        .entries
                        .iter()
                        .any(|entry| entry.pr.as_ref().is_some_and(|pr| pr.stack.is_some()))
                    {
                        bail!(
                            "native stack transition is incomplete; publication stopped; retry from fresh remote observations"
                        );
                    }
                    github
                        .commands
                        .outcomes
                        .confirm(crate::model::outcome::Operation::NativeStack);
                    return Ok((current, dependants_acquired));
                }
                Err(error) => {
                    let Some((delay, reason)) = crate::github::limits::deferral(&error, attempt)
                    else {
                        return Err(error);
                    };
                    if attempt == crate::github::limits::RETRIES {
                        return Err(
                            error.context("native transition exhausted its automatic retries")
                        );
                    }
                    github.wait_for_recovery(delay, reason.label(), started)?;
                    attempt += 1;
                }
            }
        }
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
        // These expectations guard publication destinations and development
        // dependencies. Never add trunk here: its SHA is neither a PR target
        // parameter nor something this operation owns, even after native linking.
        for (reference, original) in self.refs.iter() {
            let actual = &refs[reference];
            let intended = self
                .entries
                .iter()
                .find(|e| &e.reference == reference)
                .map(|e| &stack[e.index].hash)
                .or_else(|| (self.staging() == Some(reference.as_str())).then_some(&self.anchor));
            if actual != original
                && !(intended.is_some() && actual.as_ref() == intended)
                && !(self.temporary.contains_key(reference) && actual.is_none())
            {
                bail!(
                    "remote ref {reference:?} changed during publication; retry from fresh remote observations"
                );
            }
        }
        Ok(refs)
    }

    fn fresh(
        &self,
        github: &GitHub<'_>,
        stack: &[Commit],
        started: Instant,
    ) -> Result<(Self, Instant)> {
        let git = Git {
            commands: github.commands,
            repo: &github.config.repo,
        };
        let references = self
            .entries
            .iter()
            .map(|entry| entry.reference.clone())
            .collect::<Vec<_>>();
        let (refs, (observed_recovery, acquired)) = crate::commands::parallel_pair(
            || self.fresh_refs(git, stack),
            || github.observe_recovery_with_budget(&references, &self.numbers(), started),
        )?;
        for parent in self.parents.values() {
            if github.recovery_pr_with_budget(parent.pr.number, Some(started))? != parent.pr {
                bail!("merged downstack evidence changed before base recovery");
            }
        }
        let mut current = self.clone();
        current.refs = Arc::new(refs);
        for entry in &current.entries {
            let observed = github.publication_pr(&entry.reference)?;
            if observed.as_ref().map(|pr| (pr.number, &pr.node_id))
                != entry.pr.as_ref().map(|pr| (pr.number, &pr.node_id))
            {
                bail!("PR identity changed before base recovery");
            }
        }
        for (expected, actual) in current
            .entries
            .iter_mut()
            .filter_map(|entry| entry.pr.as_mut())
            .zip(observed_recovery)
        {
            let mut before = expected.clone();
            if actual.stack.is_none() {
                before.stack = None;
            }
            policy::same_recovery_head(&before, &actual, &before.base.name)?;
            *expected = actual;
        }
        let dependants_acquired = github.now();
        current.check_dependants(github, false, Some(started))?;
        current.native = current
            .observe_native(github, Some(started))?
            .map(|(number, topology_time)| (number, topology_time.min(acquired)));
        crate::github::limits::check_window(
            github.now(),
            std::time::Duration::ZERO,
            crate::github::limits::WaitReason::Mutation,
            None,
            Some(started),
        )?;
        Ok((current, dependants_acquired))
    }

    fn verify_heads(&self, github: &GitHub<'_>, state: &State, published: bool) -> Result<()> {
        let observed = github.recovery_prs(&self.numbers())?;
        self.verify_observed(state, published, &observed)
    }

    fn verify_observed(
        &self,
        state: &State,
        published: bool,
        observed: &[RecoveryPullRequest],
    ) -> Result<()> {
        if observed.len() != self.numbers().len() {
            bail!("incomplete publication transition observation");
        }
        for ((entry, pr), actual) in self
            .entries
            .iter()
            .filter_map(|entry| entry.pr.as_ref().map(|pr| (entry, pr)))
            .zip(observed)
        {
            let mut expected = pr.clone();
            expected.stack = None;
            if published {
                expected.head.sha.clone_from(&state.stack[entry.index].hash);
            }
            policy::same_recovery_head(
                &expected,
                actual,
                if published {
                    &self.bases[entry.index]
                } else {
                    self.publication_base(entry.index)
                },
            )?;
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
        let changes: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| entry.pr.as_ref().map(|pr| (entry, pr)))
            .filter_map(|(entry, pr)| {
                let base = self.publication_base(entry.index);
                if base == self.bases[entry.index] {
                    return None;
                }
                let mut expected = pr.clone();
                expected.stack = None;
                expected.head.sha.clone_from(&state.stack[entry.index].hash);
                base.clone_into(&mut expected.base.name);
                Some((expected, self.bases[entry.index].clone()))
            })
            .collect();
        github
            .commands
            .diagnostics
            .phase("PR base transitions", || {
                github.retarget_recoveries(&changes)
            })?;
        Ok(())
    }

    pub(super) fn verify_published(
        &self,
        state: &State,
        observed: &[RecoveryPullRequest],
    ) -> Result<()> {
        self.verify_observed(state, true, observed)
    }

    pub(super) fn finish(&self, github: &GitHub<'_>, state: &State) -> Result<()> {
        self.require_published_refs(github, state)?;
        let observed = github.recovery_prs(&self.numbers())?;
        for ((entry, pr), actual) in self
            .entries
            .iter()
            .filter_map(|entry| entry.pr.as_ref().map(|pr| (entry, pr)))
            .zip(observed)
        {
            let mut expected = pr.clone();
            expected.head.sha.clone_from(&state.stack[entry.index].hash);
            expected.stack.clone_from(&actual.stack);
            policy::same_recovery_head(&expected, &actual, &self.bases[entry.index])?;
        }
        Ok(())
    }
}
