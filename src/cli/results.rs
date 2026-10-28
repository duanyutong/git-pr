//! Pure layout of complete submission results, measured before terminal decoration.

use std::fmt::Write as _;

use unicode_width::UnicodeWidthStr as _;

use super::{
    hyperlinks,
    layout::{clip, complete_lines},
    progress::{paint, single_line},
};
use crate::model::{
    Commit, Repository,
    pull_request::{Lifecycle, PullRequestSummary},
};

const CREATED_GLYPH: &str = "+";
const UPDATED_GLYPH: &str = "~";
const UNCHANGED_GLYPH: &str = "=";

#[derive(Clone, Copy, Debug)]
pub(crate) struct Options<'a> {
    pub columns: usize,
    pub colour: bool,
    pub hyperlinks: bool,
    pub summaries: &'a [PullRequestSummary],
    pub use_icon_font: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Created,
    Updated,
    Unchanged,
    Historical,
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
            Self::Historical => "historical",
        }
    }

    const fn glyph(self) -> &'static str {
        match self {
            Self::Created => CREATED_GLYPH,
            Self::Updated => UPDATED_GLYPH,
            Self::Unchanged => UNCHANGED_GLYPH,
            Self::Historical => "·",
        }
    }

    const fn colour(self) -> &'static str {
        match self {
            Self::Created => "32",
            Self::Updated => "33",
            Self::Unchanged | Self::Historical => "2",
        }
    }
}

#[derive(Debug)]
struct Row<'a> {
    number: u64,
    title: &'a str,
    status: Status,
    lifecycle: Lifecycle,
}

fn rows<'a>(
    commits: &[&'a Commit],
    summaries: &'a [PullRequestSummary],
    stack_up: bool,
) -> Vec<Row<'a>> {
    let selected: std::collections::HashMap<_, _> =
        commits.iter().map(|c| (c.pr_number, *c)).collect();
    let mut observed: Vec<_> = summaries.iter().collect();
    if stack_up {
        observed.reverse();
    }
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<_> = observed
        .into_iter()
        .filter(|pr| seen.insert(pr.number))
        .map(|pr| {
            let commit = selected.get(&pr.number);
            Row {
                number: pr.number,
                title: pr.title.as_deref().unwrap_or_else(|| {
                    commit.map_or("PR details unavailable", |c| c.title.as_str())
                }),
                status: commit.map_or(Status::Historical, |c| Status::of(c)),
                lifecycle: pr.lifecycle,
            }
        })
        .collect();
    rows.extend(
        commits
            .iter()
            .filter(|c| c.pr_number == 0 || seen.insert(c.pr_number))
            .map(|commit| Row {
                number: commit.pr_number,
                title: &commit.title,
                status: Status::of(commit),
                lifecycle: Lifecycle::Unknown,
            }),
    );
    rows
}

const fn lifecycle_icon(lifecycle: Lifecycle, use_icon_font: bool) -> (&'static str, &'static str) {
    match lifecycle {
        Lifecycle::Open => (if use_icon_font { "\u{f407}" } else { "○" }, "32"),
        Lifecycle::Draft => (if use_icon_font { "\u{f4dd}" } else { "◌" }, "90"),
        Lifecycle::Merged => (if use_icon_font { "\u{f419}" } else { "✓" }, "35"),
        Lifecycle::Closed => (if use_icon_font { "\u{f4dc}" } else { "×" }, "31"),
        Lifecycle::Unknown => ("?", "33"),
    }
}

pub(crate) const fn orientation(stack_up: bool) -> &'static str {
    if stack_up { "tip first" } else { "tip last" }
}

pub(crate) fn render(
    commits: &[&Commit],
    repo: &Repository,
    stack_up: bool,
    options: Options<'_>,
) -> String {
    let rows = rows(commits, options.summaries, stack_up);
    let counts: Vec<_> = [
        Status::Created,
        Status::Updated,
        Status::Unchanged,
        Status::Historical,
    ]
    .into_iter()
    .filter_map(|status| {
        let count = rows.iter().filter(|row| row.status == status).count();
        (count != 0).then(|| format!("{count} {}", status.label()))
    })
    .collect();
    let heading = if counts.is_empty() {
        "Pull requests".into()
    } else {
        format!("✓ Pull requests · {}", counts.join(" · "))
    };
    let mut output = String::from("\n");
    let indent = 2.min(options.columns.saturating_sub(1));
    for line in complete_lines(&heading, options.columns.saturating_sub(indent)) {
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
        let _ = writeln!(output, "{:indent$}{line}", "");
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
    if rows.is_empty() {
        write_lines(
            &mut output,
            "Nothing selected for publication.",
            2,
            options,
            "2",
        );
    }
    let number_width = rows
        .iter()
        .map(|row| row.number.to_string().len() + 1)
        .max()
        .unwrap_or(0);
    let glyph_width = rows
        .iter()
        .map(|row| row.status.glyph().width())
        .max()
        .unwrap_or(0);
    for row in &rows {
        write_entry(&mut output, row, repo, (number_width, glyph_width), options);
    }
    output.push('\n');
    output
}

fn write_entry(
    output: &mut String,
    row: &Row<'_>,
    repo: &Repository,
    widths: (usize, usize),
    options: Options<'_>,
) {
    let status = row.status;
    let (icon, icon_colour) = lifecycle_icon(row.lifecycle, options.use_icon_font);
    let (number_width, glyph_width) = widths;
    let indent = 2.min(options.columns.saturating_sub(1));
    let prefix_width = indent + glyph_width + 2 + number_width + 2 + icon.width() + 2;
    let title = single_line(row.title);
    let number = format!("#{}", row.number);
    let destination = options
        .hyperlinks
        .then(|| hyperlinks::destination(repo, row.number))
        .flatten();
    let styled_number = paint(
        &number,
        if status == Status::Historical {
            "2"
        } else {
            "36"
        },
        options.colour,
    );
    let label = destination.as_ref().map_or_else(
        || styled_number.clone(),
        |url| hyperlinks::link(&styled_number, url),
    );
    let padding = " ".repeat(number_width.saturating_sub(number.len()));
    let glyph_padding = " ".repeat(glyph_width.saturating_sub(status.glyph().width()));
    // Compact spacing before sacrificing descriptive title cells; identities stay complete.
    let compact = options.columns < prefix_width;
    let prefix = if compact {
        format!(
            "{}{label}{}",
            paint(status.glyph(), status.colour(), options.colour),
            paint(icon, icon_colour, options.colour)
        )
    } else {
        format!(
            "{:indent$}{}{glyph_padding}  {padding}{label}  {}",
            "",
            paint(status.glyph(), status.colour(), options.colour),
            paint(icon, icon_colour, options.colour)
        )
    };
    let title_indent = if compact {
        status.glyph().width() + number.width() + icon.width() + 1
    } else {
        prefix_width
    };
    let title = clip(&title, options.columns.saturating_sub(title_indent));
    let title = if status == Status::Historical {
        paint(&title, "2", options.colour)
    } else {
        title
    };
    let separator = if compact { " " } else { "  " };
    let _ = writeln!(output, "{prefix}{separator}{title}");
    if destination.is_none() {
        let url = single_line(&format!(
            "https://{}/{}/pull/{}",
            repo.host, repo.name, row.number
        ));
        let _ = writeln!(
            output,
            "{:title_indent$}{}",
            "",
            paint(
                &clip(&url, options.columns.saturating_sub(title_indent)),
                "2",
                options.colour
            )
        );
    }
}

fn write_lines(output: &mut String, text: &str, indent: usize, options: Options<'_>, code: &str) {
    let indent = indent.min(options.columns.saturating_sub(1));
    for line in complete_lines(text, options.columns.saturating_sub(indent)) {
        let _ = writeln!(
            output,
            "{:indent$}{}",
            "",
            paint(&line, code, options.colour)
        );
    }
}
