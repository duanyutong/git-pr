#[cfg(test)]
pub(crate) use crate::tests::model_helpers::{format_key, serialise};

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
