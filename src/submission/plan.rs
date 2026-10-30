//! Deterministic publication intent, observations and mutation decisions.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use crate::model::Commit;

#[cfg(test)]
#[path = "../tests/unit/publication_plan.rs"]
mod unit;

#[derive(Clone, Debug)]
pub(super) struct DesiredHead<'a> {
    pub index: usize,
    pub reference: &'a str,
    pub hash: &'a str,
    pub base: &'a str,
}

#[derive(Debug)]
pub(super) struct DesiredPublication<'a> {
    pub heads: Vec<DesiredHead<'a>>,
    pub link_native: bool,
}

impl<'a> DesiredPublication<'a> {
    pub(super) fn new(
        stack: &'a [Commit],
        start: usize,
        bases: &'a [String],
        link_native: bool,
    ) -> Self {
        Self {
            heads: stack
                .iter()
                .enumerate()
                .skip(start)
                .filter(|(_, head)| !head.skip)
                .map(|(index, head)| DesiredHead {
                    index,
                    reference: head.remote_ref(),
                    hash: &head.hash,
                    base: &bases[index],
                })
                .collect(),
            link_native,
        }
    }

    fn tip(&self, observed: &ObservedPublication<'_>, reference: &str) -> Result<String> {
        self.heads
            .iter()
            .find(|head| head.reference == reference)
            .map_or_else(
                || observed.tip(reference).map(str::to_owned),
                |head| Ok(head.hash.to_owned()),
            )
    }

    pub(super) fn changes_bases(&self, observed: &ObservedPublication<'_>) -> bool {
        self.heads.iter().any(|head| {
            observed
                .prs
                .get(&head.index)
                .is_some_and(|(_, base)| base != head.base)
        })
    }
}

#[derive(Debug)]
pub(super) struct ObservedPublication<'a> {
    pub refs: &'a BTreeMap<String, Option<String>>,
    pub prs: BTreeMap<usize, (u64, String)>,
    pub trunk: &'a str,
    /// Immutable local evidence for staging; not an expected live trunk tip.
    pub anchor: &'a str,
}

impl ObservedPublication<'_> {
    fn anchor(&self) -> Result<&str> {
        (!self.anchor.is_empty())
            .then_some(self.anchor)
            .context("missing publication base snapshot")
    }

    fn base_tip(&self, reference: &str) -> Option<&str> {
        if reference == self.trunk {
            self.anchor().ok()
        } else {
            self.refs.get(reference).and_then(Option::as_deref)
        }
    }

    fn tip(&self, reference: &str) -> Result<&str> {
        self.base_tip(reference)
            .with_context(|| format!("missing remote publication base {reference:?}"))
    }
}

/// Verified ancestry answers acquired by the Git adapter before policy evaluation.
#[derive(Debug, Default)]
pub(super) struct Ancestry(pub BTreeMap<(String, String), bool>);

impl Ancestry {
    pub(super) fn requirements(
        desired: &DesiredPublication<'_>,
        observed: &ObservedPublication<'_>,
    ) -> Result<BTreeSet<(String, String)>> {
        let trunk = observed.anchor()?;
        let mut pairs = BTreeSet::new();
        for head in &desired.heads {
            let new_base = desired.tip(observed, head.base)?;
            // Only selected development layers must contain their predecessor.
            // The bottom PR targets the trunk branch by name and may be behind
            // its latest tip; submission must not impose a rebase requirement.
            if head.base != observed.trunk {
                pairs.insert((new_base.clone(), head.hash.to_owned()));
            }
            let old = observed
                .prs
                .contains_key(&head.index)
                .then(|| observed.tip(head.reference))
                .transpose()?;
            for hash in old.into_iter().chain([head.hash]) {
                pairs.insert((hash.to_owned(), trunk.to_owned()));
                if let Some(base) = observed.base_tip(head.base) {
                    pairs.insert((hash.to_owned(), base.to_owned()));
                }
                pairs.insert((hash.to_owned(), new_base.clone()));
            }
        }
        Ok(pairs)
    }

    fn contains(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        self.0
            .get(&(ancestor.to_owned(), descendant.to_owned()))
            .copied()
            .context("missing verified publication ancestry")
    }

    pub(super) fn check_final(
        &self,
        desired: &DesiredPublication<'_>,
        observed: &ObservedPublication<'_>,
    ) -> Result<()> {
        for head in &desired.heads {
            let base = desired.tip(observed, head.base)?;
            // Trunk is a moving integration branch, not a publication output.
            // Its snapshot supplies graph evidence, not a required ancestor of
            // the bottom head. Development bases remain exact stack edges.
            if head.hash == base
                || (head.base != observed.trunk && !self.contains(&base, head.hash)?)
            {
                bail!(
                    "prepared head {:?} is not strictly above its verified publication base; synchronise before retrying",
                    head.reference
                );
            }
        }
        Ok(())
    }

    pub(super) fn direct_is_safe(
        &self,
        desired: &DesiredPublication<'_>,
        observed: &ObservedPublication<'_>,
    ) -> Result<bool> {
        for head in &desired.heads {
            let Some(old_base) = observed.base_tip(head.base) else {
                return Ok(false);
            };
            let new_base = desired
                .heads
                .iter()
                .find(|candidate| candidate.reference == head.base)
                .map_or(old_base, |candidate| candidate.hash);
            let old = observed
                .prs
                .contains_key(&head.index)
                .then(|| observed.tip(head.reference))
                .transpose()?;
            for hash in old.into_iter().chain([head.hash]) {
                for base in [old_base, new_base] {
                    if self.contains(hash, base)? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    pub(super) fn check_staging(
        &self,
        desired: &DesiredPublication<'_>,
        observed: &ObservedPublication<'_>,
    ) -> Result<()> {
        let anchor = observed.anchor()?;
        for head in &desired.heads {
            let old = observed
                .prs
                .contains_key(&head.index)
                .then(|| observed.tip(head.reference))
                .transpose()?;
            for hash in old.into_iter().chain([head.hash]) {
                if self.contains(hash, anchor)? {
                    bail!(
                        "temporary publication base would indirectly merge head {:?}",
                        head.reference
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn verify_strategy(
        &self,
        strategy: &Strategy,
        desired: &DesiredPublication<'_>,
        observed: &ObservedPublication<'_>,
    ) -> Result<()> {
        self.check_final(desired, observed)?;
        match strategy {
            Strategy::Publish => {
                if desired.changes_bases(observed) {
                    bail!("publication base changes require a verified transition");
                }
                Ok(())
            }
            Strategy::Retarget => {
                if !self.direct_is_safe(desired, observed)? {
                    bail!(
                        "prepared commits require a different publication transition; retry from fresh state"
                    );
                }
                Ok(())
            }
            Strategy::Stage(_) => self.check_staging(desired, observed),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Strategy {
    #[default]
    Publish,
    Retarget,
    Stage(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PublicationPlan {
    pub strategy: Strategy,
    pub changing: Vec<usize>,
    pub link_native: bool,
}

impl PublicationPlan {
    pub(super) fn derive(
        desired: &DesiredPublication<'_>,
        observed: &ObservedPublication<'_>,
        ancestry: &Ancestry,
    ) -> Result<Self> {
        let strategy = if desired.changes_bases(observed) {
            ancestry.check_final(desired, observed)?;
            if ancestry.direct_is_safe(desired, observed)? {
                Strategy::Retarget
            } else {
                ancestry.check_staging(desired, observed)?;
                Strategy::Stage(
                    temporary_base(observed.prs.values().map(|(number, _)| *number))
                        .context("publication transition has no existing PR identity")?,
                )
            }
        } else {
            Strategy::Publish
        };
        Ok(Self {
            strategy,
            changing: desired
                .heads
                .iter()
                .filter(|head| {
                    observed.refs.get(head.reference).and_then(Option::as_deref) != Some(head.hash)
                })
                .map(|head| head.index)
                .collect(),
            link_native: desired.link_native,
        })
    }
}

pub(super) fn temporary_base(numbers: impl Iterator<Item = u64>) -> Option<String> {
    numbers
        .filter(|number| *number != 0)
        .min()
        .map(|number| format!("git-pr/transition/{number}"))
}

pub(super) fn temporary_owner(reference: &str) -> Option<u64> {
    let suffix = reference.strip_prefix("git-pr/transition/")?;
    let number = suffix.parse::<u64>().ok()?;
    (number != 0 && suffix == number.to_string()).then_some(number)
}
