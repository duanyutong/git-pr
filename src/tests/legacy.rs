//! Historical protocol adapters retained solely for the captured reference corpus.
//! Current workflow and protocol tests exercise the replacement production paths.

use crate::{
    cli::output::Template,
    model::Commit,
    model::trailers::parse,
    stack::description::{HistoryEntry, StackRenderer, Style},
};
use anyhow::{Result, bail};
use std::collections::HashSet;

pub(crate) fn parse_logs(logs: &str) -> Result<Vec<Commit>> {
    let logs = logs.trim();
    if logs.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let lines: Vec<_> = logs.split('\n').collect();
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.strip_prefix("commit ").is_some_and(valid_hash) {
            parts.push(&lines[start..i]);
            start = i;
        }
    }
    parts.push(&lines[start..]);
    parts.into_iter().map(parse_log_commit).collect()
}

fn parse_log_commit(lines: &[&str]) -> Result<Commit> {
    let mut commit = Commit::default();
    let header_end = lines
        .iter()
        .position(|s| s.is_empty())
        .unwrap_or(lines.len());
    for line in &lines[..header_end] {
        if let Some(hash) = line.strip_prefix("commit ")
            && valid_hash(hash)
        {
            commit.hash = hash.into();
        }
        if let Some(author) = line.strip_prefix("Author: ")
            && let Some((name, email)) = author.rsplit_once(" <")
            && let Some(email) = email.strip_suffix('>')
        {
            commit.author_name = name.into();
            commit.author_email = email.into();
        }
        if let Some(date) = line.strip_prefix("Date:") {
            let date = date.trim();
            // Git's textual weekday is informational; validate the calendar date independently.
            let parsed = date
                .get(4..)
                .and_then(|s| chrono::DateTime::parse_from_str(s, "%b %e %T %Y %z").ok())
                .or_else(|| chrono::DateTime::parse_from_str(date, "%Y-%m-%d %T %z").ok());
            let Some(parsed) = parsed else {
                bail!("failed to parse time from {date:?}");
            };
            commit.date = Some(parsed.with_timezone(&chrono::Utc));
        }
    }
    if let Some(body) = lines.get(header_end + 1..)
        && let Some((title, body)) = body.split_first()
    {
        commit.title = title.trim().into();
        let body: Vec<_> = body
            .iter()
            .map(|s| s.strip_prefix("    ").unwrap_or(s))
            .collect();
        (commit.message, commit.attrs) = parse(&body);
    }
    if commit.hash.is_empty() || commit.author_name.is_empty() || commit.author_email.is_empty() {
        bail!("failed to parse commit with log:\n{}", lines.join("\n"));
    }
    Ok(commit)
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 40
        && hash
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub(crate) fn parse_pr_url(output: &str) -> Result<u64> {
    for (_, suffix) in output
        .match_indices("/pull/")
        .map(|(index, _)| (index, &output[index + 6..]))
    {
        let digits: String = suffix.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            return Ok(digits.parse()?);
        }
    }
    bail!("PR URL absent")
}

pub(crate) fn classify_stack_error(error: &str) -> StackFailure {
    if error.contains("unknown command") {
        StackFailure::Missing
    } else if error.contains("Cannot update stack") {
        StackFailure::Rebuild
    } else if error.contains("non-fast-forward") || error.contains("[rejected]") {
        StackFailure::PushRejected
    } else {
        StackFailure::Other
    }
}

pub(crate) fn stack_reason(output: &str) -> &str {
    output
        .lines()
        .find(|l| l.contains("Cannot update stack"))
        .map_or("", |s| s.trim().trim_start_matches('✗').trim())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackFailure {
    Missing,
    Rebuild,
    PushRejected,
    Other,
}

pub(crate) fn resolve_base(
    stack: &[Commit],
    selected_start: usize,
    index: usize,
    trunk: &str,
) -> String {
    if let Some(previous) = previous_non_skipped(&stack[selected_start..], &stack[index].hash) {
        return previous.remote_ref().into();
    }
    if index > 0 && !stack[index - 1].remote_ref().is_empty() {
        return stack[index - 1].remote_ref().into();
    }
    trunk.into()
}

pub(crate) fn previous_non_skipped<'a>(stack: &'a [Commit], target: &str) -> Option<&'a Commit> {
    let index = stack.iter().position(|c| c.hash == target)?;
    stack[..index].iter().rfind(|c| !c.skip)
}

pub(crate) fn generate_stack(
    stack: &[Commit],
    current: &Commit,
    history: &[HistoryEntry],
    merged: &HashSet<u64>,
    style: Style<'_>,
) -> String {
    StackRenderer::new(stack.iter(), history, merged, style).render(current)
}

pub(crate) fn format_pr(template: &str, commit: &Commit, host: &str, repo: &str) -> String {
    Template::new(template).render(commit, host, repo)
}
