//! Test fixture construction excluded from production coverage.

use std::fmt::Write as _;

use crate::model::{Commit, REMOTE_REF};

impl Commit {
    pub(crate) fn set_attr(&mut self, key: &str, value: &str) {
        if let Some((_, v)) = self.attrs.iter_mut().find(|(k, _)| k == key) {
            value.clone_into(v);
        } else {
            self.attrs.push((key.into(), value.into()));
            self.attrs.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }

    pub(crate) fn full_message(&self) -> String {
        crate::tests::unit::support::MESSAGE_SERIALISATIONS.update(|count| count + 1);
        serialise(&self.title, &self.message, &self.attrs)
    }
}

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
