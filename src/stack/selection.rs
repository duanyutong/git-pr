use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};

use crate::model::Commit;

const PATTERN_LIMIT: usize = 1024 * 1024;
const PATTERN_GUIDANCE: &str =
    "draft patterns exceed matcher resource limits; shorten --draft-pattern or git-pr.draftPattern";

/// Immutable compiled patterns and invocation-local title classifications.
#[derive(Clone, Debug)]
pub(crate) struct DraftPatterns(Arc<CompiledDraftPatterns>);

#[derive(Debug)]
struct CompiledDraftPatterns {
    literals: Vec<LiteralGlob>,
    matcher: Option<regex::bytes::RegexSet>,
    results: Mutex<HashMap<String, bool>>,
}

#[derive(Debug)]
enum LiteralGlob {
    Exact(Vec<u8>),
    Starred {
        prefix: Vec<u8>,
        suffix: Vec<u8>,
        needles: Vec<Needle>,
    },
}

/// A non-empty literal with its KMP prefix table.
#[derive(Debug)]
struct Needle {
    bytes: Vec<u8>,
    prefix: Vec<usize>,
}

impl Needle {
    fn new(literal: &str) -> Option<Self> {
        if literal.is_empty() {
            return None;
        }
        let bytes = literal.as_bytes().to_vec();
        let mut prefix = Vec::with_capacity(bytes.len());
        prefix.push(0);
        let mut matched = 0;
        for index in 1..bytes.len() {
            // matched < index; every fallback is strictly smaller than matched.
            while matched > 0 && bytes[matched] != bytes[index] {
                matched = prefix[matched - 1];
            }
            if bytes[matched] == bytes[index] {
                matched += 1;
            }
            prefix.push(matched);
        }
        Some(Self { bytes, prefix })
    }

    fn after_match<'a>(&self, text: &'a [u8]) -> Option<&'a [u8]> {
        let mut matched = 0;
        for (index, &byte) in text.iter().enumerate() {
            #[cfg(test)]
            crate::tests::unit::support::LITERAL_STEPS.update(|n| n + 1);
            // Full matches return immediately, so matched remains below bytes.len().
            while matched > 0 && self.bytes[matched] != byte {
                #[cfg(test)]
                crate::tests::unit::support::LITERAL_STEPS.update(|n| n + 1);
                matched = self.prefix[matched - 1];
            }
            if self.bytes[matched] == byte {
                matched += 1;
            }
            if matched == self.bytes.len() {
                return text.get(index + 1..);
            }
        }
        None
    }
}

impl LiteralGlob {
    fn new(pattern: String) -> Self {
        if !pattern.contains('*') {
            return Self::Exact(pattern.into_bytes());
        }
        let mut pieces = pattern.split('*');
        let prefix = pieces.next().unwrap_or_default().as_bytes().to_vec();
        let suffix = pieces.next_back().unwrap_or_default().as_bytes().to_vec();
        let needles = pieces.filter_map(Needle::new).collect();
        Self::Starred {
            prefix,
            suffix,
            needles,
        }
    }

    fn matches(&self, text: &[u8]) -> bool {
        match self {
            Self::Exact(literal) => text == literal,
            Self::Starred {
                prefix,
                suffix,
                needles,
            } => {
                let Some(mut remaining) = text
                    .strip_prefix(prefix.as_slice())
                    .and_then(|rest| rest.strip_suffix(suffix.as_slice()))
                else {
                    return false;
                };
                for needle in needles {
                    let Some(rest) = needle.after_match(remaining) else {
                        return false;
                    };
                    remaining = rest;
                }
                true
            }
        }
    }
}

impl DraftPatterns {
    pub(crate) fn new(patterns: &[String]) -> Result<Self> {
        if patterns
            .iter()
            .fold(0usize, |size, p| size.saturating_add(p.len()))
            > PATTERN_LIMIT
        {
            bail!(PATTERN_GUIDANCE);
        }
        #[cfg(test)]
        crate::tests::unit::support::DRAFT_PREPARATIONS.update(|n| n + patterns.len());
        let mut expressions = Vec::new();
        let mut literals = Vec::new();
        for pattern in patterns {
            let normalised = pattern.to_lowercase();
            if !normalised.contains('?') {
                literals.push(LiteralGlob::new(normalised));
                continue;
            }
            let mut expression = String::from(r"\A");
            for piece in normalised.split_inclusive(['*', '?']) {
                expression.push_str(&regex::escape(piece.trim_end_matches(['*', '?'])));
                match piece.as_bytes().last() {
                    Some(b'*') => expression.push_str(".*"),
                    Some(b'?') => expression.push('.'),
                    _ => {}
                }
            }
            expression.push_str(r"\z");
            expressions.push(expression);
        }
        let matcher = (!expressions.is_empty())
            .then(|| {
                regex::bytes::RegexSetBuilder::new(expressions)
                    .unicode(false)
                    .dot_matches_new_line(true)
                    .build()
            })
            .transpose()
            .context(PATTERN_GUIDANCE)?;
        Ok(Self(Arc::new(CompiledDraftPatterns {
            literals,
            matcher,
            results: Mutex::new(HashMap::new()),
        })))
    }

    pub(crate) fn matches(&self, title: &str) -> bool {
        let mut results = self
            .0
            .results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&matched) = results.get(title) {
            return matched;
        }
        #[cfg(test)]
        crate::tests::unit::support::DRAFT_EVALUATIONS.update(|n| n + 1);
        let normalised = title.to_lowercase();
        let bytes = normalised.as_bytes();
        let matched = self.0.literals.iter().any(|p| p.matches(bytes))
            || self.0.matcher.as_ref().is_some_and(|m| m.is_match(bytes));
        results.insert(title.to_owned(), matched);
        matched
    }
}

pub(crate) fn mark_drafts(
    commits: &mut [Commit],
    enabled: bool,
    include: bool,
    patterns: &DraftPatterns,
) -> Vec<usize> {
    let mut marked = Vec::new();
    if enabled && !include {
        for (index, commit) in commits.iter_mut().enumerate() {
            if !commit.skip && patterns.matches(&commit.title) {
                commit.skip = true;
                marked.push(index);
            }
        }
    }
    marked
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    pub explicit: bool,
    pub base: String,
    pub tip: String,
    pub selected: Vec<String>,
}

/// Classify revision syntax before any repository or working-copy effects.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Revisions<'a> {
    Range { base: &'a str, tip: &'a str },
    Commits(&'a [String]),
}

impl<'a> Revisions<'a> {
    pub(crate) fn parse(args: &'a [String]) -> Result<Self> {
        if let [arg] = args {
            if let Some((base, tip)) = arg.split_once("..") {
                if arg.contains("...") {
                    bail!(
                        "ERROR: symmetric-difference range {arg:?} is not supported; use BASE..TIP instead"
                    );
                }
                if base.is_empty() || tip.is_empty() {
                    bail!(
                        "ERROR: invalid range {arg:?}; expected BASE..TIP with both endpoints non-empty"
                    );
                }
                return Ok(Self::Range { base, tip });
            }
        } else if let Some(arg) = args.iter().find(|arg| arg.contains("..")) {
            bail!(
                "ERROR: cannot combine a range {arg:?} with other commit arguments;\npass either a single BASE..TIP range or a list of individual commits"
            );
        }
        Ok(Self::Commits(args))
    }
}

pub(crate) fn order_selection(stack: &[Commit], selected: &[String]) -> Result<(usize, usize)> {
    let indices: HashMap<_, _> = stack
        .iter()
        .enumerate()
        .map(|(i, c)| (c.hash.as_str(), i))
        .collect();
    let mut positions = Vec::new();
    for hash in selected {
        let Some(&i) = indices.get(hash.as_str()) else {
            bail!(
                "commit {} is not in the current stack",
                hash.get(..8).unwrap_or(hash)
            );
        };
        positions.push(i);
    }
    match (positions.iter().min(), positions.iter().max()) {
        (Some(lo), Some(hi)) => Ok((*lo, *hi)),
        _ => bail!("no commits selected"),
    }
}

pub(crate) fn slice_start(
    stack: &[Commit],
    base: &str,
    trunk_hash: &str,
    depths: &HashMap<String, usize>,
) -> usize {
    if base == trunk_hash {
        return 0;
    }
    if let Some(index) = stack.iter().position(|c| c.hash == base) {
        return index + 1;
    }
    // Empty commits remain valid range boundaries after presentation filtering.
    depths
        .get(base)
        .and_then(|base_depth| {
            stack
                .iter()
                .position(|c| depths.get(&c.hash).is_some_and(|depth| depth < base_depth))
        })
        .unwrap_or(stack.len())
}

pub(crate) fn mark_unselected(stack: &mut [Commit], selected: &[String]) -> Vec<String> {
    let selected: HashSet<_> = selected.iter().collect();
    let mut announcements = Vec::new();
    for commit in stack {
        if !commit.skip && !selected.contains(&commit.hash) {
            commit.skip = true;
            announcements.push(format!(
                "skip \"{}\" ({}) not selected",
                crate::model::shorten_title(&commit.title),
                commit.short_hash()
            ));
        }
    }
    announcements
}

/// Resolve predecessor bases once, retaining contextual anchors before the selection.
pub(crate) fn resolve_bases(stack: &[Commit], selected_start: usize, trunk: &str) -> Vec<String> {
    #[cfg(test)]
    crate::tests::unit::support::BASE_PLANS.update(|count| count + 1);
    let mut previous: Option<&Commit> = None;
    stack
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            let base = previous
                .map(Commit::remote_ref)
                .or_else(|| {
                    i.checked_sub(1)
                        .and_then(|j| stack.get(j))
                        .map(Commit::remote_ref)
                        .filter(|reference| !reference.is_empty())
                })
                .unwrap_or(trunk)
                .to_owned();
            if i >= selected_start && !commit.skip {
                previous = Some(commit);
            }
            base
        })
        .collect()
}

pub(crate) fn missing_refs(commits: &[Commit]) -> Vec<&str> {
    commits
        .iter()
        .filter(|c| !c.skip && c.remote_ref().is_empty())
        .map(Commit::short_hash)
        .collect()
}

pub(crate) fn recover_tip(
    jj: bool,
    change: &str,
    head: &str,
    depth: usize,
    resolve_change: impl FnOnce(&str) -> Result<String>,
    resolve_depth: impl FnOnce(&str, usize) -> Result<String>,
) -> Result<String> {
    if jj {
        resolve_change(change)
    } else {
        resolve_depth(head, depth)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefSync {
    Nothing,
    Move,
    Unrelated,
}

pub(crate) fn decide_ref_sync(
    local: &str,
    pushed: &str,
    fast_forward: bool,
    same_change: bool,
) -> RefSync {
    if local == pushed {
        RefSync::Nothing
    } else if fast_forward || same_change {
        RefSync::Move
    } else {
        RefSync::Unrelated
    }
}

pub(crate) fn remote_ref_matches(output: &str, expected_ref: &str, expected_hash: &str) -> bool {
    output.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields == [expected_hash, expected_ref]
    })
}

pub(crate) fn pick_stack_leaf(
    tips: &[String],
    independent: &dyn Fn(&[String]) -> Result<Vec<String>>,
) -> Result<Option<String>> {
    let Some(first) = tips.first() else {
        return Ok(None);
    };
    if tips.len() <= 128 {
        let roots = independent(tips)?;
        return Ok(if roots.len() == 1 {
            roots.into_iter().next()
        } else {
            None
        });
    }
    // A unique global leaf, if present, wins its batch and every later batch.
    // Otherwise the verification pass detects the discarded incomparable roots.
    let mut candidate = first.clone();
    for chunk in tips[1..].chunks(127) {
        let batch = std::iter::once(candidate)
            .chain(chunk.iter().cloned())
            .collect::<Vec<_>>();
        candidate = independent(&batch)?
            .into_iter()
            .next()
            .context("missing candidate tip")?;
    }
    for chunk in tips.chunks(127) {
        let batch = std::iter::once(candidate.clone())
            .chain(chunk.iter().cloned())
            .collect::<Vec<_>>();
        if independent(&batch)? != [candidate.clone()] {
            return Ok(None);
        }
    }
    Ok(Some(candidate))
}

#[cfg(test)]
pub(crate) fn match_wildcard(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let text = text.to_lowercase();
    match_normalised(&pattern, &text)
}

#[cfg(test)]
pub(crate) fn match_normalised(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let (mut p, mut t, mut star, mut matched) = (0, 0, None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            matched = t;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            matched += 1;
            t = matched;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

pub(crate) fn ignored_branch(name: &str) -> bool {
    name == "main" || name == "master" || name.starts_with("(HEAD detached ")
}

pub(crate) fn local_branch(output: &str, hash: &str) -> Option<String> {
    output
        .lines()
        .filter_map(|line| line.trim().rsplit_once('|'))
        .find(|(branch, target)| !ignored_branch(branch) && *target == hash)
        .map(|(branch, _)| branch.into())
}

pub(crate) fn local_branches(output: &str) -> HashMap<&str, &str> {
    let mut names = HashMap::new();
    for (name, oid) in output
        .lines()
        .filter_map(|line| line.trim().rsplit_once('|'))
    {
        if !ignored_branch(name) {
            names.entry(oid).or_insert(name);
        }
    }
    names
}
