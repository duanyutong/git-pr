use anyhow::{Result, bail};

use crate::model::Commit;

const TOKENS: [&str; 10] = [
    "{url}",
    "{number}",
    "{title}",
    "{hash}",
    "{shorthash}",
    "{repo}",
    "{host}",
    "{branch}",
    "{author}",
    "{email}",
];

const FIELDS: [&str; 10] = [
    "url",
    "number",
    "title",
    "hash",
    "shorthash",
    "repo",
    "host",
    "branch",
    "author",
    "email",
];

pub(crate) fn version(sha: Option<&str>, dirty: Option<&str>) -> String {
    format!(
        "git-pr version {} (git {}, dirty={}, target={}, rustc={}, built={})",
        env!("CARGO_PKG_VERSION"),
        sha.unwrap_or("unknown"),
        dirty.unwrap_or("unknown"),
        env!("VERGEN_CARGO_TARGET_TRIPLE"),
        env!("VERGEN_RUSTC_SEMVER"),
        env!("VERGEN_BUILD_TIMESTAMP")
    )
}

pub(crate) fn resolve_format(value: &str) -> Result<String> {
    let value = value.trim();
    match value {
        "url" => Ok("{url}".into()),
        "" | "url-title" => Ok("{url} {title}".into()),
        "markdown" => Ok("[#{number}]({url}) {title}".into()),
        _ if value.contains('{') => {
            let mut opening = None;
            for (index, character) in value.char_indices() {
                if character == '{' {
                    opening = Some(index + 1);
                }
                if character == '}'
                    && let Some(start) = opening.take()
                {
                    let field = &value[start..index];
                    if !field.is_empty()
                        && field
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                        && !FIELDS.contains(&field)
                    {
                        bail!(
                            "unknown placeholder {{{field}}} in --output template\n{}",
                            format_help()
                        );
                    }
                }
            }
            Ok(value.into())
        }
        _ => bail!("invalid --output {value:?}\n{}", format_help()),
    }
}

fn format_help() -> String {
    format!(
        "\nPresets:\n  markdown   [#{{number}}]({{url}}) {{title}}\n  url        {{url}}\n  url-title  {{url}} {{title}}  (default)\n\nOr a custom template using these placeholders:\n  {}\n\nExample:\n  git pr --output '{{shorthash}} #{{number}} {{title}}'\n",
        FIELDS.map(|s| format!("{{{s}}}")).join(", ")
    )
}

#[derive(Debug)]
pub(crate) struct Template<'a> {
    parts: Vec<Part<'a>>,
    needs_url: bool,
    needs_number: bool,
}
#[derive(Debug)]
enum Part<'a> {
    Literal(&'a str),
    Field(usize),
}
impl<'a> Template<'a> {
    pub(crate) fn new(template: &'a str) -> Self {
        let mut parts = Vec::new();
        let mut needs_url = false;
        let mut needs_number = false;
        let mut start = 0;
        let mut cursor = 0;
        while cursor < template.len() {
            if let Some((i, token)) = TOKENS
                .iter()
                .enumerate()
                .find(|(_, token)| template[cursor..].starts_with(**token))
            {
                if start < cursor {
                    parts.push(Part::Literal(&template[start..cursor]));
                }
                parts.push(Part::Field(i));
                needs_url |= i == 0;
                needs_number |= i == 1;
                cursor += token.len();
                start = cursor;
            } else {
                cursor += template[cursor..].chars().next().map_or(1, char::len_utf8);
            }
        }
        if start < template.len() {
            parts.push(Part::Literal(&template[start..]));
        }
        Self {
            parts,
            needs_url,
            needs_number,
        }
    }
    pub(crate) fn render(&self, commit: &Commit, host: &str, repo: &str) -> String {
        let url = if self.needs_url {
            format!("https://{host}/{repo}/pull/{}", commit.pr_number)
        } else {
            String::new()
        };
        let number = if self.needs_number {
            commit.pr_number.to_string()
        } else {
            String::new()
        };
        let replacements = [
            url.as_str(),
            &number,
            &commit.title,
            &commit.hash,
            commit.short_hash(),
            repo,
            host,
            commit.remote_ref(),
            &commit.author_name,
            &commit.author_email,
        ];
        let value = |part: &Part<'a>| match part {
            Part::Literal(text) => *text,
            Part::Field(i) => replacements[*i],
        };
        let capacity = self.parts.iter().map(|part| value(part).len()).sum();
        let mut output = String::with_capacity(capacity);
        for part in &self.parts {
            output.push_str(value(part));
        }
        output
    }
}
