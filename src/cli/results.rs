//! Pure layout of complete submission results, measured before terminal decoration.

use std::fmt::Write as _;

use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use super::{
    hyperlinks,
    progress::{paint, single_line},
};
use crate::model::{Commit, Repository};

const CREATED_GLYPH: &str = "+";
const UPDATED_GLYPH: &str = "~";
const UNCHANGED_GLYPH: &str = "=";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    pub columns: usize,
    pub colour: bool,
    pub hyperlinks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Created,
    Updated,
    Unchanged,
}

impl Status {
    const fn of(commit: &Commit) -> Self {
        if commit.newly_created {
            Self::Created
        } else if commit.metadata_updated || commit.base_updated {
            Self::Updated
        } else {
            Self::Unchanged
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }

    const fn glyph(self) -> &'static str {
        match self {
            Self::Created => CREATED_GLYPH,
            Self::Updated => UPDATED_GLYPH,
            Self::Unchanged => UNCHANGED_GLYPH,
        }
    }

    const fn colour(self) -> &'static str {
        match self {
            Self::Created => "32",
            Self::Updated => "33",
            Self::Unchanged => "2",
        }
    }
}

pub(crate) const fn orientation(stack_up: bool) -> &'static str {
    if stack_up { "tip first" } else { "tip last" }
}

pub(crate) fn render(
    commits: &[&Commit],
    repo: &Repository,
    stack_up: bool,
    options: Options,
) -> String {
    let statuses: Vec<_> = commits.iter().map(|commit| Status::of(commit)).collect();
    let counts: Vec<_> = [Status::Created, Status::Updated, Status::Unchanged]
        .into_iter()
        .filter_map(|status| {
            let count = statuses.iter().filter(|s| **s == status).count();
            (count != 0).then(|| format!("{count} {}", status.label()))
        })
        .collect();
    let heading = if counts.is_empty() {
        "Pull requests".into()
    } else {
        format!("✓ Pull requests · {}", counts.join(" · "))
    };
    let mut output = String::from("\n");
    for line in wrap(&heading, options.columns) {
        let line = line.strip_prefix('✓').map_or_else(
            || paint(&line, "1", options.colour),
            |rest| {
                format!(
                    "{}{}",
                    paint("✓", "32", options.colour),
                    paint(rest, "1", options.colour)
                )
            },
        );
        let _ = writeln!(output, "{line}");
    }
    let repository = if repo.host == "github.com" || repo.host.is_empty() {
        single_line(&repo.name)
    } else {
        single_line(&format!("{}/{}", repo.host, repo.name))
    };
    let metadata = if repository.is_empty() {
        orientation(stack_up).to_owned()
    } else {
        format!("{repository} · {}", orientation(stack_up))
    };
    write_lines(&mut output, &metadata, 2, options, "2");
    output.push('\n');
    if commits.is_empty() {
        write_lines(
            &mut output,
            "Nothing selected for publication.",
            2,
            options,
            "2",
        );
    }
    let number_width = commits
        .iter()
        .map(|c| c.pr_number.to_string().len() + 1)
        .max()
        .unwrap_or(0);
    let glyph_width = statuses
        .iter()
        .map(|status| status.glyph().width())
        .max()
        .unwrap_or(0);
    for (commit, status) in commits.iter().zip(statuses) {
        write_entry(
            &mut output,
            commit,
            repo,
            status,
            (number_width, glyph_width),
            options,
        );
    }
    output.push('\n');
    output
}

fn write_entry(
    output: &mut String,
    commit: &Commit,
    repo: &Repository,
    status: Status,
    widths: (usize, usize),
    options: Options,
) {
    let (number_width, glyph_width) = widths;
    let indent = 2.min(options.columns.saturating_sub(1));
    let prefix_width = indent + glyph_width + 2 + number_width + 2;
    let inline = options.columns.saturating_sub(prefix_width) >= 24;
    let title_indent = if inline {
        prefix_width
    } else {
        4.min(options.columns.saturating_sub(1))
    };
    let title = single_line(&commit.title);
    let has_title = !title.trim().is_empty();
    let number = format!("#{}", commit.pr_number);
    let destination = options
        .hyperlinks
        .then(|| hyperlinks::destination(repo, commit.pr_number))
        .flatten();
    let styled_number = paint(&number, "36", options.colour);
    let label = destination.as_ref().map_or_else(
        || styled_number.clone(),
        |url| hyperlinks::link(&styled_number, url),
    );
    let padding = " ".repeat(number_width.saturating_sub(number.len()));
    let glyph_padding = " ".repeat(glyph_width.saturating_sub(status.glyph().width()));
    let _ = write!(
        output,
        "{:indent$}{}{glyph_padding}  {padding}{label}",
        "",
        paint(status.glyph(), status.colour(), options.colour)
    );
    let lines = wrap(&title, options.columns.saturating_sub(title_indent));
    if !inline || !has_title {
        output.push('\n');
    }
    for (index, line) in lines.iter().filter(|_| has_title).enumerate() {
        if inline && index == 0 {
            let _ = writeln!(output, "  {line}");
        } else {
            let _ = writeln!(output, "{:title_indent$}{line}", "");
        }
    }
    if destination.is_none() {
        let url = single_line(&format!(
            "https://{}/{}/pull/{}",
            repo.host, repo.name, commit.pr_number
        ));
        let _ = writeln!(
            output,
            "{:title_indent$}{}",
            "",
            paint(&url, "2", options.colour)
        );
    }
}

fn write_lines(output: &mut String, text: &str, indent: usize, options: Options, code: &str) {
    let indent = indent.min(options.columns.saturating_sub(1));
    for line in wrap(text, options.columns.saturating_sub(indent)) {
        let _ = writeln!(
            output,
            "{:indent$}{}",
            "",
            paint(&line, code, options.colour)
        );
    }
}

/// Preserve indivisible graphemes even when the terminal is narrower than one glyph.
pub(crate) fn wrap(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        if !line.is_empty() && used + 1 + word.width() > columns {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if !line.is_empty() {
            line.push(' ');
            used += 1;
        }
        for grapheme in word.graphemes(true) {
            if !line.is_empty() && used + grapheme.width() > columns {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push_str(grapheme);
            used += grapheme.width();
        }
    }
    // The final word leaves a non-empty line; an empty title retains one empty line.
    lines.push(line);
    lines
}
