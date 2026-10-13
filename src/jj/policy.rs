//! Deterministic interpretation of Jujutsu working-copy values.

use anyhow::{Context, Result, bail};

use crate::{model::Commit, model::trailers};

pub(super) fn parse_conflict(marker: &str) -> Result<bool> {
    match marker {
        "false" => Ok(false),
        "true" => Ok(true),
        _ => bail!("invalid Jujutsu conflict marker {marker:?}; expected true or false"),
    }
}

pub(crate) fn clean_status(output: &str) -> bool {
    matches!(
        output.trim(),
        "EMPTY|NO-DESC" | "EMPTY|HAS-DESC" | "NONEMPTY|HAS-DESC"
    )
}

pub(crate) fn parse_working_copy(
    check: &str,
    info: &str,
    author_name: &str,
    author_email: &str,
) -> Result<Option<Commit>> {
    let parts: Vec<_> = check
        .trim()
        .lines()
        .last()
        .unwrap_or("")
        .split('|')
        .collect();
    if parts.len() != 2 || parts[0] == "EMPTY" || parts[1] != "HAS-DESC" {
        return Ok(None);
    }
    let info = info.trim();
    let first = info.lines().next().unwrap_or("");
    let parts: Vec<_> = first.splitn(3, '|').collect();
    if parts.len() != 3 {
        bail!("unexpected jj @ output: {first}");
    }
    let body = &info[parts[0].len() + parts[1].len() + 2..];
    let (title, body) = body.split_once('\n').unwrap_or((body, ""));
    let (message, attrs) = trailers::parse(&body.split('\n').collect::<Vec<_>>());
    Ok(Some(Commit {
        hash: parts[1].into(),
        change_id: parts[0].into(),
        title: title.trim().into(),
        message,
        attrs,
        author_email: author_email.into(),
        author_name: author_name.into(),
        ..Commit::default()
    }))
}

/// Bound both argument cardinality and command bytes before invoking Jujutsu.
pub(crate) fn revision_query(revisions: &[String]) -> Result<(String, String, usize)> {
    let mut revset = String::new();
    let mut template = String::from("commit_id ++ \"\\0\"");
    let mut count = 0;
    for revision in revisions.iter().take(128) {
        let literal = template_literal(revision);
        let membership = format!(" ++ if(self.contained_in({literal}), \"{count},\", \"\")");
        if revset.len() + template.len() + revision.len() + membership.len() + 32 > 65_536 {
            break;
        }
        if count != 0 {
            revset.push_str(" | ");
        }
        revset.push('(');
        revset.push_str(revision);
        revset.push(')');
        template.push_str(&membership);
        count += 1;
    }
    if count == 0 {
        bail!("Jujutsu revision exceeds the bounded batch command size");
    }
    template.push_str(" ++ \"\\n\"");
    Ok((revset, template, count))
}

/// Jujutsu accepts its own escapes, rather than JSON's Unicode escape syntax.
pub(crate) fn template_literal(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    for character in text.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            '\0' => literal.push_str("\\0"),
            _ => literal.push(character),
        }
    }
    literal.push('"');
    literal
}

/// Preserve individual cardinality and caller order, including revision aliases.
pub(crate) fn parse_resolutions(output: &str, revisions: &[String]) -> Result<Vec<String>> {
    let mut resolved = vec![None; revisions.len()];
    for line in output.lines() {
        let (hash, members) = line
            .split_once('\0')
            .context("invalid Jujutsu resolution record")?;
        if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid Jujutsu commit identity in resolution record");
        }
        let members = members
            .strip_suffix(',')
            .context("incomplete Jujutsu argument membership")?;
        for member in members.split(',') {
            let index = member
                .parse::<usize>()
                .context("invalid Jujutsu argument identity")?;
            let slot = resolved
                .get_mut(index)
                .context("unexpected Jujutsu argument identity")?;
            if slot.replace(hash.to_owned()).is_some() {
                bail!(
                    "Jujutsu revision {:?} matched multiple commits",
                    revisions[index]
                );
            }
        }
    }
    revisions
        .iter()
        .zip(resolved)
        .map(|(revision, hash)| {
            hash.with_context(|| format!("Jujutsu revision {revision:?} matched no commits"))
        })
        .collect()
}
