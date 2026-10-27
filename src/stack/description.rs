#[cfg(test)]
#[path = "../tests/description_helpers.rs"]
mod test_helpers;
#[cfg(test)]
pub(crate) use test_helpers::{extract_history, generate_body};

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::model::Commit;

pub(crate) const START: &str = "<!-- git-pr-stack-start -->";
pub(crate) const END: &str = "<!-- git-pr-stack-end -->";
pub(crate) const BODY_TEMPLATE: &str = "\n# Summary\n\n\n\n\n\n<br><br><br><br>\n";
const EMOJI: [&str; 12] = [
    "🐹", "🐮", "🐯", "🦊", "🐲", "🐼", "🦁", "🐰", "🐵", "🐻", "🐶", "🐷",
];

fn normalise_line_endings(text: &str) -> Option<String> {
    if !text.contains("\r\n") {
        return None;
    }
    let mut normalised = String::with_capacity(text.len());
    for part in text.split_inclusive('\n') {
        if let Some(line) = part.strip_suffix('\n') {
            // Remove the entire run so normalisation cannot manufacture a
            // further CRLF pair which would change the next observation.
            normalised.push_str(line.trim_end_matches('\r'));
            normalised.push('\n');
        } else {
            normalised.push_str(part);
        }
    }
    Some(normalised)
}

pub(crate) fn initial_body<'a>(message: &'a str, template: &'a str) -> Result<&'a str> {
    let (body, source) = if message.is_empty() {
        (template, "PR template")
    } else {
        (message, "commit message")
    };
    if body.contains(START) || body.contains(END) {
        bail!(
            "cannot initialise a PR description from the {source}: `{START}` and `{END}` are reserved managed-footer delimiters; remove these delimiters or replace their opening '<' with '&lt;' in literal examples whilst preserving the text, then retry; this description was not written"
        );
    }
    Ok(body)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HistoryEntry {
    pub number: u64,
    pub merged: bool,
}

fn markers(body: &str) -> Result<Option<(usize, usize)>> {
    let mut starts = body.match_indices(START);
    let mut ends = body.match_indices(END);
    match (starts.next(), ends.next()) {
        (None, None) => Ok(None),
        (Some((start, _)), Some((end, _)))
            if end > start && starts.next().is_none() && ends.next().is_none() =>
        {
            Ok(Some((start, end)))
        }
        _ => bail!(
            "malformed managed stack footer: retain exactly one `{START}` followed by one `{END}`, or remove all stack delimiters whilst preserving human text, then retry; this description was not updated"
        ),
    }
}

fn history_row(line: &str) -> Option<(HistoryEntry, &str, bool)> {
    let line = line.strip_prefix("* ")?;
    let (marker, number) = if let Some(n) = line.strip_prefix('#') {
        ("", n)
    } else {
        let (marker, number) = line.split_once(' ')?;
        (marker, number.strip_prefix('#')?)
    };
    if !["", "✔️", "⬛", "◻️", "👉"].contains(&marker) && !EMOJI.contains(&marker) {
        return None;
    }
    let end = number.bytes().take_while(u8::is_ascii_digit).count();
    let (digits, suffix) = number.split_at(end);
    let number = digits.parse().ok()?;
    if number == 0 || (!suffix.is_empty() && !suffix.starts_with(' ')) {
        return None;
    }
    Some((
        HistoryEntry {
            number,
            merged: marker == "✔️",
        },
        suffix,
        !marker.is_empty(),
    ))
}

fn stack_header(line: &str) -> bool {
    let Some(counts) = line.strip_prefix("This is PR ") else {
        return false;
    };
    let counts = [
        " in a stack (oldest at the top)",
        " in a stack (newest at the top)",
        " in a stack",
    ]
    .iter()
    .find_map(|suffix| counts.strip_suffix(suffix))
    .unwrap_or(counts);
    let counts = counts
        .strip_prefix("**")
        .and_then(|s| s.strip_suffix("**"))
        .unwrap_or(counts);
    let Some((position, total)) = counts.split_once(" of ") else {
        return false;
    };
    matches!((position.parse::<usize>(), total.parse::<usize>()), (Ok(p), Ok(n)) if p > 0 && p <= n)
}

fn legacy_section(section: &str) -> bool {
    let mut rows = 0;
    let mut distinctive = false;
    for (i, line) in section
        .lines()
        .map(str::trim_end)
        .filter(|s| !s.is_empty())
        .enumerate()
    {
        if i == 0 && stack_header(line) {
            distinctive = true;
            continue;
        }
        let Some((_, suffix, marked)) = history_row(line) else {
            return false;
        };
        let current = suffix == " 👈 This PR"
            || suffix
                .strip_prefix(" 👈 This PR (")
                .and_then(|s| s.strip_suffix(')'))
                .is_some_and(|hash| {
                    !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit())
                });
        if !suffix.is_empty() && !current {
            return false;
        }
        rows += 1;
        distinctive |= marked || current;
    }
    rows > 0 && distinctive
}

#[derive(Debug)]
enum Section {
    Managed { start: usize, end: usize },
    Legacy { start: usize, end: usize },
}

/// Analysis of one exact observation; offsets refer to the normalised body.
#[derive(Debug)]
pub(crate) struct Body {
    source: Arc<str>,
    normalised: Option<String>,
    section: Option<Section>,
}

impl Body {
    fn analyse(source: Arc<str>) -> Result<Self> {
        #[cfg(test)]
        crate::tests::unit::support::BODY_ANALYSES.update(|count| count + 1);
        let normalised = normalise_line_endings(&source);
        let text = normalised.as_deref().unwrap_or(&source);
        let section = markers(text)?
            .map(|(start, end)| Section::Managed { start, end })
            .or_else(|| {
                let mut start = 0;
                for part in text.split("\n---\n") {
                    let end = start + part.len();
                    if legacy_section(part) {
                        return Some(Section::Legacy { start, end });
                    }
                    start = end + "\n---\n".len();
                }
                None
            });
        Ok(Self {
            source,
            normalised,
            section,
        })
    }

    fn text(&self) -> &str {
        self.normalised.as_deref().unwrap_or(&self.source)
    }

    pub(crate) fn append_history(&self, history: &mut Vec<HistoryEntry>, seen: &mut HashSet<u64>) {
        let text = self.text();
        let section = match self.section {
            Some(Section::Managed { start, end }) => &text[start + START.len()..end],
            Some(Section::Legacy { start, end }) => &text[start..end],
            None => "",
        };
        let start = history.len();
        history.extend(
            section
                .lines()
                .filter_map(|line| history_row(line).map(|(entry, _, _)| entry))
                .filter(|entry| seen.insert(entry.number)),
        );
        // Deduplicate in source order so the first row retains its merged state.
        // Reverse only this body's new entries, leaving earlier observations intact.
        if section.contains("newest at the top") {
            history[start..].reverse();
        }
    }

    pub(crate) fn render_footer(
        &self,
        message: &str,
        footer: &Footer<'_>,
        template: &str,
    ) -> Result<Option<String>> {
        let existing = self.text();
        if let Some(Section::Managed { start, end }) = self.section {
            if footer.matches(&existing[start..end + END.len()]) {
                return Ok(self.normalised.clone());
            }
            let before = existing[..start].trim_end_matches([' ', '\t', '\n']);
            let after = existing[end + END.len()..].trim_start_matches([' ', '\t', '\n']);
            return Ok(Some(if after.is_empty() {
                footer.body(&[before, "\n\n"], &[])
            } else {
                footer.body(&[before, "\n\n"], &["\n\n", after])
            }));
        }
        if existing.is_empty() || existing == BODY_TEMPLATE || existing == template {
            let seed = if message.is_empty() {
                template
            } else {
                message
            };
            // Exact observed bytes have already passed delimiter analysis.
            let body = if seed == existing {
                existing
            } else {
                initial_body(message, template)?
            };
            return Ok(Some(if message.is_empty() {
                footer.body(&[body, "\n---\n"], &[])
            } else {
                footer.body(&[body, "\n\n---\n"], &[])
            }));
        }
        if let Some(Section::Legacy { start, end }) = self.section {
            return Ok(Some(
                footer.body(&[&existing[..start]], &[&existing[end..]]),
            ));
        }
        Ok(Some(footer.body(&[existing, "\n\n---\n"], &[])))
    }
}

/// Only the header and current row vary; the rest borrows the shared stack text.
#[derive(Debug, Default)]
pub(crate) struct Footer<'a> {
    header: String,
    before: &'a str,
    current: String,
    after: &'a str,
}

impl Footer<'_> {
    pub(crate) fn parts(&self) -> [&str; 4] {
        [&self.header, self.before, &self.current, self.after]
    }

    fn wrapped(&self) -> impl Iterator<Item = &str> {
        [START, "\n"]
            .into_iter()
            .chain(self.parts())
            .chain(["\n", END])
    }

    fn matches(&self, mut text: &str) -> bool {
        for part in self.wrapped() {
            let Some(rest) = text.strip_prefix(part) else {
                return false;
            };
            text = rest;
        }
        text.is_empty()
    }

    fn body(&self, before: &[&str], after: &[&str]) -> String {
        let parts = || {
            before
                .iter()
                .copied()
                .chain(self.wrapped())
                .chain(after.iter().copied())
        };
        let mut text = String::with_capacity(parts().map(str::len).sum());
        for part in parts() {
            #[cfg(test)]
            crate::tests::unit::support::count_description_bytes(part.len());
            text.push_str(part);
        }
        // Canonicalise the composed result as well as observations: seed text
        // and separators can introduce CRLF after the initial body analysis.
        normalise_line_endings(&text).unwrap_or(text)
    }
}

pub(crate) enum Renderer<'a> {
    Full(StackRenderer<'a>),
    Compact(CompactRenderer<'a>),
}

impl Renderer<'_> {
    pub(crate) fn footer(&self, current: &Commit) -> Footer<'_> {
        match self {
            Self::Full(renderer) => renderer.footer(current),
            Self::Compact(renderer) => Footer {
                current: renderer.render(current),
                ..Footer::default()
            },
        }
    }
}

/// Reuse analysis only whilst the observed body is byte-for-byte identical.
#[derive(Debug, Default)]
pub(crate) struct BodyCache {
    bodies: HashMap<u64, Body>,
}

impl BodyCache {
    pub(crate) fn get_shared(&mut self, number: u64, source: &Arc<str>) -> Result<&Body> {
        use std::collections::hash_map::Entry;
        let analyse = || {
            Body::analyse(Arc::clone(source))
                .with_context(|| format!("failed to inspect PR #{number} description"))
        };
        Ok(match self.bodies.entry(number) {
            Entry::Occupied(mut entry) => {
                if !Arc::ptr_eq(&entry.get().source, source) && entry.get().source != *source {
                    entry.insert(analyse()?);
                }
                entry.into_mut()
            }
            Entry::Vacant(entry) => entry.insert(analyse()?),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Style<'a> {
    pub stack_up: bool,
    pub emoji: bool,
    pub host: &'a str,
    pub repo: &'a str,
}

#[derive(Debug)]
pub(crate) struct StackRenderer<'a> {
    text: String,
    positions: HashMap<&'a str, (usize, usize, usize, &'a Commit, bool)>,
    len: usize,
    has_unnumbered: bool,
    style: Style<'a>,
}

impl<'a> StackRenderer<'a> {
    pub(crate) fn new(
        stack: impl Iterator<Item = &'a Commit> + Clone,
        history: &[HistoryEntry],
        merged: &HashSet<u64>,
        style: Style<'a>,
    ) -> Self {
        let current_numbers: HashSet<_> = stack
            .clone()
            .filter(|c| c.pr_number != 0)
            .map(|c| c.pr_number)
            .collect();
        let mut seen = HashSet::new();
        let mut entries: Vec<_> = history
            .iter()
            .filter(|h| !current_numbers.contains(&h.number) && seen.insert(h.number))
            .map(|h| (h.number, None, h.merged))
            .collect();
        let mut has_unnumbered = false;
        for c in stack {
            let unnumbered = c.pr_number == 0;
            has_unnumbered |= unnumbered;
            if unnumbered || seen.insert(c.pr_number) {
                entries.push((c.pr_number, Some(c), merged.contains(&c.pr_number)));
            }
        }
        let len = entries.len();
        let mut indexed: Vec<_> = entries.into_iter().enumerate().collect();
        if style.stack_up {
            indexed.reverse();
        }
        let mut text = String::new();
        let mut positions = HashMap::new();
        let other = Commit {
            hash: String::new(),
            ..Commit::default()
        };
        for (position, (number, commit, is_merged)) in indexed {
            let start = text.len();
            if let Some(commit) = commit {
                text.push_str(&render_commit(commit, &other, is_merged, style));
                positions
                    .entry(commit.hash.as_str())
                    .and_modify(|value: &mut (usize, usize, usize, &Commit, bool)| {
                        if position + 1 < value.0 {
                            *value = (position + 1, start, text.len(), commit, is_merged);
                        }
                    })
                    .or_insert((position + 1, start, text.len(), commit, is_merged));
            } else {
                let marker = if !style.emoji {
                    ""
                } else if is_merged {
                    " ✔️"
                } else {
                    " ⬛"
                };
                let _ = writeln!(text, "*{marker} #{number}");
            }
        }
        Self {
            text,
            positions,
            len,
            has_unnumbered,
            style,
        }
    }

    pub(crate) fn footer(&self, current: &Commit) -> Footer<'_> {
        let row = self.positions.get(current.hash.as_str());
        let position = row.map_or(0, |r| r.0);
        let mut header = String::new();
        if self.len > 1 {
            let order = if self.style.stack_up {
                "newest"
            } else {
                "oldest"
            };
            if self.has_unnumbered {
                let _ = writeln!(
                    header,
                    "This PR is entry **{position} of {}** in a stack ({order} at the top)\n",
                    self.len
                );
            } else {
                let _ = writeln!(
                    header,
                    "This is PR **{position} of {}** in a stack ({order} at the top)\n",
                    self.len
                );
            }
        }
        if let Some((_, start, end, commit, merged)) = row {
            Footer {
                header,
                before: &self.text[..*start],
                current: render_commit(commit, current, *merged, self.style),
                after: &self.text[*end..],
            }
        } else {
            Footer {
                header,
                before: &self.text,
                ..Footer::default()
            }
        }
    }
}

pub(crate) fn render_commit(
    commit: &Commit,
    current: &Commit,
    merged: bool,
    style: Style<'_>,
) -> String {
    let is_current = commit.hash == current.hash;
    let reference = if commit.pr_number != 0 && is_current {
        format!("#{} 👈 This PR", commit.pr_number)
    } else if commit.pr_number != 0 {
        format!("#{}", commit.pr_number)
    } else {
        let (user, host) = commit.author_email.split_once('@').map_or(
            (commit.author_email.as_str(), String::new()),
            |(user, host)| (user, format!("@{host}")),
        );
        format!(
            "[{}](https://{}/{}/commit/{}) — {}&#x200B;{}",
            escape_context_text(&commit.title),
            style.host,
            style.repo,
            commit.hash,
            escape_context_text(user),
            escape_context_text(&host),
        )
    };
    let marker = if !style.emoji {
        String::new()
    } else if is_current {
        format!(" {}", EMOJI[(current.pr_number % 12) as usize])
    } else if merged {
        " ✔️".into()
    } else {
        " ⬛".into()
    };
    format!("*{marker} {reference}\n")
}

fn escape_context_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\\' | '[' | ']' | '`' | '*' | '_' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '\r' | '\n' => escaped.push(' '),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Constant-size navigation for each PR; historical full-stack lists are omitted explicitly.
#[derive(Debug)]
pub(crate) struct CompactRenderer<'a> {
    numbers: Vec<u64>,
    positions: HashMap<&'a str, usize>,
}
impl<'a> CompactRenderer<'a> {
    pub(crate) fn new(stack: impl IntoIterator<Item = &'a Commit>) -> Self {
        let mut seen = HashSet::new();
        let mut numbers = Vec::new();
        let mut positions = HashMap::new();
        for commit in stack {
            if commit.pr_number != 0 && seen.insert(commit.pr_number) {
                positions.insert(commit.hash.as_str(), numbers.len());
                numbers.push(commit.pr_number);
            }
        }
        Self { numbers, positions }
    }
    pub(crate) fn render(&self, commit: &Commit) -> String {
        let Some(&position) = self.positions.get(commit.hash.as_str()) else {
            return String::new();
        };
        let mut text = format!(
            "Stack navigation (PR {} of {})\n\n",
            position + 1,
            self.numbers.len()
        );
        if let Some(previous) = position.checked_sub(1) {
            let _ = writeln!(text, "* #{} — Previous PR", self.numbers[previous]);
        }
        let _ = writeln!(text, "* #{} — This PR", commit.pr_number);
        if let Some(next) = self.numbers.get(position + 1) {
            let _ = writeln!(text, "* #{next} — Next PR");
        }
        text
    }
}
