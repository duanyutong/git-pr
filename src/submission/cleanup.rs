//! Guarded temporary-reference cleanup, independent of publication transitions.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail};

use crate::{git::Git, github::GitHub};

#[cfg(test)]
#[path = "../tests/unit/publication_cleanup.rs"]
mod unit;

pub(super) fn remove_temporary(
    github: &GitHub<'_>,
    destination: &str,
    candidates: &BTreeMap<String, String>,
    anchor: Option<&str>,
) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    let git = Git {
        commands: github.commands,
        repo: &github.config.repo,
    };
    if git.recovery_destination()? != destination {
        bail!("publication destination changed before temporary-base cleanup");
    }
    // Reuse the transition's immutable proof, or capture a local snapshot when
    // cleaning an orphan on a later run. Cleanup leases only the helper ref;
    // observing live trunk would introduce an unrelated race and could require
    // objects that an independent remote merge has not made available locally.
    let anchor = anchor.map_or_else(|| git.trunk_snapshot(), |anchor| Ok(anchor.to_owned()))?;
    let references = candidates.keys().cloned().collect::<Vec<_>>();
    let refs = git.recovery_refs(destination, &references)?;
    for (reference, expected) in candidates {
        let Some(actual) = refs.get(reference).and_then(Option::as_deref) else {
            continue;
        };
        if actual != expected {
            bail!(
                "temporary publication base {reference:?} changed; retained for a subsequent observation"
            );
        }
        if !git.is_ancestor(expected, &anchor)? {
            bail!(
                "temporary publication base {reference:?} is not a verified trunk ancestor; retained without deletion"
            );
        }
        github.check_publication_dependants(std::slice::from_ref(reference), &HashMap::new())?;
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
    Ok(())
}
