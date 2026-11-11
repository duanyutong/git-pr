//! Verification and consent for indirect merges outside a publication selection.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Mutex,
    time::Instant,
};

use anyhow::{Context as _, Result, bail};

use crate::{git::Git, github::GitHub};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Risk {
    number: u64,
    head: String,
    base: String,
    head_hash: String,
    base_hash: String,
}

#[derive(Debug, Default)]
pub(super) struct Approvals {
    confirmed: Mutex<BTreeSet<Risk>>,
    ancestry: Mutex<BTreeMap<(String, String), bool>>,
    outside: Mutex<BTreeSet<String>>,
}

impl Approvals {
    pub(super) fn has_outside(&self, references: &[String]) -> bool {
        let outside = self
            .outside
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        references
            .iter()
            .any(|reference| outside.contains(reference))
    }

    pub(super) fn verify(
        &self,
        github: &GitHub<'_>,
        references: &[String],
        selected: &HashMap<u64, (&str, &str)>,
        proposed: &BTreeMap<&str, &str>,
    ) -> Result<Instant> {
        for _ in 0..4 {
            let acquired = github.now();
            let risks = self.risks(github, references, selected, proposed)?;
            let pending: Vec<_> = {
                let approved = self
                    .confirmed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                risks.difference(&approved).cloned().collect()
            };
            if pending.is_empty() {
                return Ok(acquired);
            }
            for risk in &pending {
                github.commands.warn(&format!(
                    "Publishing {:?} at {} would make PR #{} (head {:?} at {}) reachable from its base. GitHub may mark that unselected PR as indirectly merged even without its required reviews or checks.",
                    risk.base, risk.base_hash, risk.number, risk.head, risk.head_hash,
                ));
            }
            if !github.config.auto_accept
                && !github.commands.terminal.confirm(
                    "Continue publication and allow these unselected PRs to be indirectly merged?",
                )
            {
                bail!("publication declined because it could indirectly merge an unselected PR");
            }
            self.confirmed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(pending);
            // Confirmation can take arbitrarily long; reacquire the complete observation.
        }
        bail!(
            "indirect-merge evidence changed repeatedly during confirmation; inspect the affected PRs before retrying"
        )
    }

    fn risks(
        &self,
        github: &GitHub<'_>,
        references: &[String],
        selected: &HashMap<u64, (&str, &str)>,
        proposed: &BTreeMap<&str, &str>,
    ) -> Result<BTreeSet<Risk>> {
        let mut candidates = Vec::new();
        for dependant in github.publication_dependants(references, selected)? {
            let base_hash = proposed
                .get(dependant.base.as_str())
                .context("missing proposed publication base for an unselected dependant")?;
            candidates.push(Risk {
                number: dependant.number,
                head: dependant.head,
                base: dependant.base,
                head_hash: dependant.hash,
                base_hash: (*base_hash).into(),
            });
        }
        {
            let mut outside = self
                .outside
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for reference in references {
                outside.remove(reference);
            }
            outside.extend(candidates.iter().map(|risk| risk.base.clone()));
        }
        // Stored-parent proofs depend only on immutable object identities.
        // Refresh live PRs every time, but acquire each distinct pair only once.
        let mut ancestry = self
            .ancestry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let missing = candidates
            .iter()
            .map(|risk| (risk.head_hash.clone(), risk.base_hash.clone()))
            .filter(|pair| !ancestry.contains_key(pair))
            .collect();
        Git { commands: github.commands, repo: &github.config.repo }
            .scoped_ancestry(&missing)
            .context("cannot determine whether publication would indirectly merge an unselected PR; synchronise the relevant history before retrying")
            .map(|observed| ancestry.extend(observed))?;
        let risks = Self::classify(&candidates, &ancestry);
        drop(ancestry);
        risks.inspect(|risks| {
            for candidate in &candidates {
                github.commands.explain(&format!(
                    "PR #{} outside the selection: head {:?} at {}; proposed base {:?} at {}; indirect merge: {}.",
                    candidate.number, candidate.head, candidate.head_hash, candidate.base, candidate.base_hash, risks.contains(candidate),
                ));
            }
        })
    }

    fn classify(
        candidates: &[Risk],
        ancestry: &BTreeMap<(String, String), bool>,
    ) -> Result<BTreeSet<Risk>> {
        let mut risks = BTreeSet::new();
        for candidate in candidates {
            let pair = (candidate.head_hash.clone(), candidate.base_hash.clone());
            let reachable = *ancestry
                .get(&pair)
                .context("missing verified dependant ancestry")?;
            if reachable {
                risks.insert(candidate.clone());
            }
        }
        Ok(risks)
    }
}

#[cfg(test)]
#[path = "../tests/unit/dependants.rs"]
mod unit;
