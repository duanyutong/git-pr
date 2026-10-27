//! Test support excluded from production coverage.

use std::collections::HashMap;

use super::ignored_branch;

pub(crate) fn match_wildcard(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let text = text.to_lowercase();
    match_normalised(&pattern, &text)
}

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
