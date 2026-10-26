//! One observation and constraint procedure for submission and reconstruction.

use super::{Binding, Head, store::Records};
use crate::{
    commands::Commands,
    config::checkpoint::Checkpoint,
    git::Git,
    github::GitHub,
    model::{Commit, REMOTE_REF, Repository},
};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn inventory(commands: &Commands) -> Result<BTreeMap<String, String>> {
    let output = commands
        .git(&["branch", "--format=%(refname:strip=2)|%(objectname)"])
        .context("failed to observe local branches")?;
    let mut branches = BTreeMap::new();
    for line in output.lines() {
        let (name, oid) = line
            .rsplit_once('|')
            .context("invalid local anchor observation")?;
        if crate::stack::selection::ignored_branch(name) || name == "(no branch)" {
            continue;
        }
        Git::validate_remote_ref(name)?;
        if branches.insert(name.into(), oid.into()).is_some() {
            bail!("duplicate local anchor {name:?}");
        }
    }
    Ok(branches)
}

pub(crate) fn prepare(github: &GitHub<'_>, stack: &mut [Commit], start: usize) -> Result<()> {
    let records = Records::read(github.commands)?;
    let branches = inventory(github.commands)?;
    let preview = github.config.dry_run || github.config.stop_after == Some(Checkpoint::Rewrite);
    let attempts = records.attempts(&github.config.repo)?;
    if !preview {
        let mut references = BTreeSet::new();
        for commit in stack.iter() {
            if !commit.attr(REMOTE_REF).is_empty() {
                references.insert(commit.attr(REMOTE_REF).to_owned());
            }
            if let Some(name) = anchor(commit, &records, &branches)?
                && let Some(head) = records.head(&name, &github.config.repo)?
            {
                references.insert(head.reference);
            }
        }
        github
            .observe(&references.into_iter().collect::<Vec<_>>())
            .context(crate::diagnostics::errors::Fatal)?;
    }
    if github.config.repo.jj {
        let mut changes = BTreeSet::new();
        for commit in stack[start..]
            .iter()
            .filter(|commit| !commit.skip && !commit.change_id.is_empty())
        {
            if !changes.insert(&commit.change_id) {
                bail!(
                    "divergent selected Jujutsu change identity {}; explicitly disambiguate",
                    commit.change_id
                );
            }
        }
    }
    for (index, commit) in stack.iter_mut().enumerate() {
        let selected = index >= start && !commit.skip;
        let binding = resolve(github, commit, &records, &branches, preview, selected)?;
        if let Some(binding) = binding {
            if selected {
                crate::git::policy::protect_trunk(
                    &binding.head.reference,
                    &github.config.repo.trunk,
                )?;
            }
            if !preview && selected && attempts.iter().any(|a| a.head == binding.head) {
                github.observe(std::slice::from_ref(&binding.head.reference))?;
                if github.publication_pr(&binding.head.reference)?.is_none() {
                    bail!(
                        "PR creation for {:?} has an unknown outcome; retain git-pr.pendingCreate and verify the outstanding request before retrying",
                        binding.head.reference
                    );
                }
            }
            if selected && preview && !binding.adopted {
                github.commands.out(&format!(
                    "[DRY-RUN] Would associate {} with {} (remote verification outstanding)",
                    commit.short_hash(),
                    binding.head.value()
                ));
            }
            commit.publication = Some(binding);
        }
    }
    validate_unique(stack, start, &records, &branches)
}

fn anchor(
    commit: &Commit,
    records: &Records,
    branches: &BTreeMap<String, String>,
) -> Result<Option<String>> {
    let candidates: Vec<_> = branches
        .iter()
        .filter(|(_, oid)| *oid == &commit.hash)
        .map(|(name, _)| name.as_str())
        .collect();
    let bound: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|name| {
            records.0.contains_key(&format!("branch.{name}.gitprhead"))
                || records
                    .0
                    .contains_key(&format!("branch.{name}.gitprpullrequest"))
        })
        .collect();
    let eligible = if bound.is_empty() {
        &candidates
    } else {
        &bound
    };
    match eligible.as_slice() {
        [] => Ok(branches
            .contains_key(commit.attr(REMOTE_REF))
            .then(|| commit.attr(REMOTE_REF).to_owned())),
        [name] => Ok(Some((*name).into())),
        _ if bound.is_empty() && !commit.attr(REMOTE_REF).is_empty() => Ok(candidates
            .iter()
            .find(|name| **name == commit.attr(REMOTE_REF))
            .map(|name| (*name).to_string())),
        _ => bail!(
            "ambiguous local anchors for {}: {}; explicitly bind one anchor",
            commit.short_hash(),
            eligible.join(", ")
        ),
    }
}

fn previous_anchor<'a>(
    commit: &Commit,
    name: &str,
    records: &'a Records,
    branches: &BTreeMap<String, String>,
) -> Result<Option<&'a str>> {
    if commit.change_id.is_empty() {
        return Ok(None);
    }
    let mut candidates = BTreeSet::new();
    for old in records.anchors().filter(|old| *old != name) {
        if records.claim(old, "gitprchangeid")? == Some(commit.change_id.as_str()) {
            if branches.contains_key(old) {
                bail!(
                    "change identity {} has competing live anchors {old:?} and {name:?}",
                    commit.change_id
                );
            }
            candidates.insert(old);
        }
    }
    if candidates.len() > 1 {
        bail!("ambiguous former anchors for change {}", commit.change_id);
    }
    Ok(candidates.first().copied())
}

fn resolve(
    github: &GitHub<'_>,
    commit: &Commit,
    records: &Records,
    branches: &BTreeMap<String, String>,
    preview: bool,
    selected: bool,
) -> Result<Option<Binding>> {
    let repo = &github.config.repo;
    let anchor = anchor(commit, records, branches)?;
    let previous = match (repo.jj, anchor.as_deref()) {
        (true, Some(name)) => previous_anchor(commit, name, records, branches)?,
        _ => None,
    };
    let primary = anchor.as_deref().is_some_and(|name| {
        records.0.contains_key(&format!("branch.{name}.gitprhead"))
            || records
                .0
                .contains_key(&format!("branch.{name}.gitprpullrequest"))
    });
    let source = if primary {
        anchor.as_deref()
    } else {
        previous.or(anchor.as_deref())
    };
    let mut head = source
        .map(|name| records.head(name, repo))
        .transpose()?
        .flatten();
    let mut review = source
        .map(|name| records.review(name, repo))
        .transpose()?
        .flatten();
    if let Some(old) = previous {
        let prior_head = records.head(old, repo)?;
        let prior_review = records.review(old, repo)?;
        if head
            .as_ref()
            .zip(prior_head.as_ref())
            .is_some_and(|(current, prior)| current != prior)
            || review
                .zip(prior_review)
                .is_some_and(|(current, prior)| current != prior)
        {
            bail!("former anchor {old:?} conflicts with the current association");
        }
        head = head.or(prior_head);
        review = review.or(prior_review);
        records.relocation(github.commands, old)?;
    }
    verify_change_id(records, source, commit, repo.jj)?;
    let trailer = commit.attr(REMOTE_REF);
    if !trailer.is_empty() {
        let legacy = Head::new(repo, trailer)
            .with_context(|| format!("commit {} has invalid Remote-Ref", commit.short_hash()))?;
        if head.as_ref().is_some_and(|head| head != &legacy) {
            bail!(
                "Remote-Ref conflicts with gitPrHead for {}; explicitly reconcile the association",
                commit.short_hash()
            );
        }
        head = Some(legacy);
    }
    let adopted = head.is_some() || review.is_some();
    if !preview {
        verify_review(
            github,
            anchor.as_deref(),
            records,
            selected,
            &mut head,
            &mut review,
        )?;
    }
    let head = if let Some(head) = head {
        head
    } else {
        let Some(name) = &anchor else {
            if selected {
                bail!(
                    "commit {} is not on any local branch or bookmark; create a maintained anchor before submitting",
                    commit.short_hash()
                );
            }
            return Ok(None);
        };
        if review.is_some() && preview {
            bail!(
                "PR-only association for {name:?} requires remote verification; preview cannot determine its head"
            );
        }
        let Some(head) = discover(github, commit, name, records, preview, selected)? else {
            return Ok(None);
        };
        head
    };
    Ok(Some(Binding {
        anchor_oid: anchor.as_ref().and_then(|name| branches.get(name)).cloned(),
        anchor,
        head,
        review,
        adopted,
        previous_anchor: previous.map(str::to_owned),
    }))
}

fn verify_change_id(
    records: &Records,
    anchor: Option<&str>,
    commit: &Commit,
    jujutsu: bool,
) -> Result<()> {
    if let Some(name) = anchor
        && let Some(id) = records.claim(name, "gitprchangeid")?
    {
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            bail!("invalid change identity for {name:?}");
        }
        if jujutsu && id != commit.change_id {
            bail!(
                "anchor {name:?} now identifies a different Jujutsu change; explicitly reassociate it"
            );
        }
    }
    Ok(())
}

fn verify_review(
    github: &GitHub<'_>,
    anchor: Option<&str>,
    records: &Records,
    selected: bool,
    head: &mut Option<Head>,
    review: &mut Option<u64>,
) -> Result<()> {
    let repo = &github.config.repo;
    if let Some(number) = *review {
        let pr = github
            .get(number)
            .context("adopted PR could not be verified; refusing replacement")?;
        let observed = Head::new(repo, &pr.head.name)?;
        if head.as_ref().is_some_and(|head| head != &observed) {
            bail!("adopted PR #{number} conflicts with its recorded publication head");
        }
        if selected {
            crate::github::policy::require_open(&pr)?;
        }
        *head = Some(observed);
    } else if head.is_none()
        && let Some(name) = anchor
        && let Some(hint) = records.hint(&format!("branch.{name}.github-pr-owner-number"))
    {
        let number = editor_review(hint, repo)?;
        *review = Some(number);
        let pr = github.get(number)?;
        if selected {
            crate::github::policy::require_open(&pr)?;
        }
        *head = Some(Head::new(repo, &pr.head.name)?);
    }
    Ok(())
}

fn editor_review(value: &str, repo: &Repository) -> Result<u64> {
    let parts: Vec<_> = value.split('#').collect();
    let [owner, name, number] = parts.as_slice() else {
        bail!("invalid github-pr-owner-number hint");
    };
    if !format!("{owner}/{name}").eq_ignore_ascii_case(&repo.name) {
        bail!("editor PR hint belongs to a different repository");
    }
    super::parse_review(
        &format!("https://{}/{owner}/{name}/pull/{number}", repo.host),
        repo,
    )
}

fn discover(
    github: &GitHub<'_>,
    commit: &Commit,
    anchor: &str,
    records: &Records,
    preview: bool,
    selected: bool,
) -> Result<Option<Head>> {
    let repo = &github.config.repo;
    let named = Head::new(repo, anchor)?;
    if preview {
        return Ok(selected.then_some(named));
    }
    let exact: BTreeSet<_> = github.identity_heads(&commit.hash)?.into_iter().collect();
    let push = github.commands.git(&[
        "for-each-ref",
        "--format=%(push:remotename)%00%(push:remoteref)",
        &format!("refs/heads/{anchor}"),
    ])?;
    let mut preferred = None;
    if let Some((remote, reference)) = push.split_once('\0') {
        if remote == repo.remote
            && let Some(reference) = reference.strip_prefix("refs/heads/")
        {
            crate::git::policy::protect_trunk(reference, &repo.trunk)?;
            preferred = Some(reference.to_owned());
        }
    } else if !push.is_empty() {
        bail!("incomplete Git push-destination observation");
    }
    if preferred.is_none()
        && records.hint(&format!("branch.{anchor}.remote")) == Some(repo.remote.as_str())
        && let Some(reference) = records
            .hint(&format!("branch.{anchor}.merge"))
            .and_then(|v| v.strip_prefix("refs/heads/"))
        && reference != repo.trunk
        && (reference == anchor || exact.contains(reference))
    {
        preferred = Some(reference.into());
    }
    let mut candidates = exact;
    let mut references = vec![anchor.into()];
    references.extend(preferred.iter().cloned());
    github.observe(&references)?;
    if let Some(reference) = preferred
        && (selected || github.observed_head(&reference, "all") != 0)
    {
        candidates.insert(reference);
    }
    if github.observed_head(anchor, "all") != 0 {
        candidates.insert(anchor.into());
    }
    match candidates.into_iter().collect::<Vec<_>>().as_slice() {
        [] => Ok(selected.then_some(named)),
        [reference] => Head::new(repo, reference).map(Some),
        candidates => bail!(
            "ambiguous publication candidates for {}: {}; explicitly set gitPrHead",
            commit.short_hash(),
            candidates.join(", ")
        ),
    }
}

pub(crate) fn validate_unique(
    stack: &[Commit],
    start: usize,
    records: &Records,
    branches: &BTreeMap<String, String>,
) -> Result<()> {
    let mut heads = BTreeSet::new();
    for commit in stack[start..].iter().filter(|c| !c.skip) {
        if commit.remote_ref().is_empty() {
            bail!("missing resolved publication binding");
        }
        if !heads.insert(commit.remote_ref()) {
            bail!("duplicate publication head {:?}", commit.remote_ref());
        }
        let Some(binding) = &commit.publication else {
            continue;
        };
        for name in branches
            .keys()
            .filter(|name| Some(name.as_str()) != binding.anchor.as_deref())
        {
            if let Some(value) = records.claim(name, "gitprhead")?
                && binding.head.matches(value)
            {
                bail!(
                    "publication head {:?} is also bound to live anchor {name:?}; explicitly disambiguate",
                    binding.head.reference
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn persist(github: &GitHub<'_>, commits: &mut [Commit]) -> Result<()> {
    let records = Records::read(github.commands)?;
    let attempts = records.attempts(&github.config.repo)?;
    for commit in commits.iter_mut().filter(|c| !c.skip) {
        let reference = commit.remote_ref().to_owned();
        let binding = commit
            .publication
            .as_mut()
            .context("selected change has no resolved publication binding")?;
        let review = github.publication_pr(&reference)?.map_or(0, |pr| pr.number);
        if binding.review.is_some_and(|number| number != review) {
            bail!("review identity changed before persistence for {reference:?}");
        }
        super::store::persist_observed(
            github.commands,
            &records,
            &super::store::assignments(&github.config.repo, binding, &commit.change_id, review),
        )?;
        binding.review = (review != 0).then_some(review);
        if let Some(old) = &binding.previous_anchor {
            super::store::retire(github.commands, &records.relocation(github.commands, old)?)?;
        }
        if review != 0 && attempts.iter().any(|a| a.head.reference == reference) {
            super::store::finish(github.commands, &github.config.repo, &reference)?;
        }
    }
    validate_unique(
        commits,
        0,
        &Records::read(github.commands)?,
        &inventory(github.commands)?,
    )
}

pub(crate) fn verify_anchors(commands: &Commands, commits: &[Commit]) -> Result<()> {
    let branches = inventory(commands)?;
    for commit in commits.iter().filter(|c| !c.skip) {
        if let Some(anchor) = commit.publication.as_ref().and_then(|b| b.anchor.as_ref())
            && branches.get(anchor)
                != commit
                    .publication
                    .as_ref()
                    .and_then(|binding| binding.anchor_oid.as_ref())
        {
            bail!(
                "selected anchor {anchor:?} changed during submission; reobserve before retrying"
            );
        }
    }
    Ok(())
}

/// Retire only orphaned associations whose adopted reviews are confirmed merged.
pub(crate) fn cleanup(github: &GitHub<'_>, records: &Records, commits: &[Commit]) -> Result<()> {
    let anchors: BTreeSet<_> = records
        .anchors()
        .filter(|anchor| {
            !crate::stack::selection::ignored_branch(anchor)
                && !commits.iter().any(|commit| {
                    commit
                        .publication
                        .as_ref()
                        .and_then(|binding| binding.anchor.as_deref())
                        == Some(*anchor)
                })
        })
        .collect();
    if anchors.is_empty() {
        return Ok(());
    }
    // Ordinary repeats need no further commands. Orphans require fresh pending
    // creation evidence before their exact configuration values may retire.
    let current = Records::read(github.commands)?;
    let branches = inventory(github.commands)?;
    let attempts = current.attempts(&github.config.repo)?;
    let mut candidates = Vec::new();
    for anchor in anchors {
        if branches.contains_key(anchor) {
            continue;
        }
        let (Ok(head), Ok(Some(review))) = (
            records.head(anchor, &github.config.repo),
            records.review(anchor, &github.config.repo),
        ) else {
            continue;
        };
        if attempts
            .iter()
            .any(|attempt| head.as_ref().is_none_or(|head| attempt.head == *head))
        {
            continue;
        }
        // Included definitions remain under their owner's control.
        if let Ok(mut values) = records.relocation(github.commands, anchor) {
            // Keep the merged review identity until every other field has retired,
            // so interrupted cleanup remains eligible without a head association.
            values.sort_by_key(|(key, _)| key.ends_with(".gitprpullrequest"));
            candidates.push((anchor, review, head, values));
        }
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let numbers: Vec<_> = candidates.iter().map(|(_, review, _, _)| *review).collect();
    let merged = github.merged_prs(&numbers)?;
    let branches = inventory(github.commands)?;
    for (anchor, review, head, values) in candidates {
        if merged.contains(&review) && !branches.contains_key(anchor) {
            let current = Records::read(github.commands)?;
            if ["gitprhead", "gitprpullrequest", "gitprchangeid"]
                .iter()
                .any(|field| {
                    let key = format!("branch.{anchor}.{field}");
                    records.0.get(&key) != current.0.get(&key)
                })
            {
                continue;
            }
            if current
                .attempts(&github.config.repo)?
                .iter()
                .any(|attempt| head.as_ref().is_none_or(|head| attempt.head == *head))
            {
                continue;
            }
            super::store::retire(github.commands, &values)?;
        }
    }
    Ok(())
}

pub(crate) fn verify_bindings(
    commands: &Commands,
    repo: &Repository,
    commits: &[Commit],
) -> Result<Records> {
    let records = Records::read(commands)?;
    for commit in commits.iter().filter(|commit| !commit.skip) {
        if let Some(binding) = &commit.publication
            && let Some(anchor) = &binding.anchor
        {
            if records.head(anchor, repo)?.as_ref() != Some(&binding.head) {
                bail!(
                    "publication binding for {anchor:?} changed before mutation; reobserve before retrying"
                );
            }
            if let Some(number) = binding
                .review
                .or_else(|| (commit.pr_number != 0).then_some(commit.pr_number))
                && records.review(anchor, repo)? != Some(number)
            {
                bail!("adopted PR association for {anchor:?} changed before mutation");
            }
        }
    }
    validate_unique(commits, 0, &records, &inventory(commands)?)?;
    Ok(records)
}

#[cfg(test)]
#[path = "../tests/unit/identity_resolution.rs"]
mod unit;
