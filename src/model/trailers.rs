#[cfg(test)]
use std::fmt::Write as _;

#[cfg(test)]
use crate::model::REMOTE_REF;

pub(crate) fn parse(lines: &[&str]) -> (String, Vec<(String, String)>) {
    let text = lines.join("\n");
    let text = text.trim();
    let lines: Vec<_> = text.lines().collect();
    let mut attrs = Vec::new();
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().rev() {
        if let Some((key, value)) = parse_attr(line) {
            attrs.push((key, value));
            end = i;
        } else {
            if !line.trim().is_empty() {
                return (text.into(), Vec::new());
            }
            end = i;
            break;
        }
    }
    if attrs.is_empty() {
        return (text.into(), attrs);
    }
    (lines[..end].join("\n").trim().into(), attrs)
}

fn parse_attr(line: &str) -> Option<(String, String)> {
    let (key, value) = line.trim_start_matches([' ', '\t']).split_once(':')?;
    let key = key.trim_end();
    if key.is_empty() || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return None;
    }
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some((key.to_ascii_lowercase(), value.into()))
}

#[cfg(test)]
pub(crate) fn format_key(key: &str) -> String {
    key.to_lowercase()
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
pub(crate) fn serialise(title: &str, message: &str, attrs: &[(String, String)]) -> String {
    let mut text = format!("{title}\n\n");
    if !message.is_empty() {
        let _ = writeln!(text, "{message}\n");
    }
    // Internal attributes put the winning occurrence first. On disk the final
    // occurrence wins, so retain that precedence within each sorted key group.
    let mut attrs: Vec<_> = attrs.iter().rev().collect();
    attrs.sort_by(|a, b| (a.0 == REMOTE_REF, &a.0).cmp(&(b.0 == REMOTE_REF, &b.0)));
    let width = attrs.iter().map(|a| a.0.len()).max().unwrap_or(0);
    for (key, value) in attrs {
        let _ = writeln!(text, "{:>width$}: {value}", format_key(key));
    }
    text.trim().into()
}
