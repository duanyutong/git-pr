use std::fmt;

use crate::model::trailers;

pub(crate) const REMOTE_REF: &str = "remote-ref";

// Independent facts may coexist, including a deferred base edit marked as updated.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Commit {
    pub hash: String,
    pub change_id: String,
    pub date: Option<chrono::DateTime<chrono::Utc>>,
    pub author_name: String,
    pub author_email: String,
    pub title: String,
    pub message: String,
    pub attrs: Vec<(String, String)>,
    pub pr_number: u64,
    pub skip: bool,
    /// Conflict state of this commit or an observed local ancestor.
    pub conflicted: bool,
    pub newly_created: bool,
    pub base_blocked: bool,
    pub base_updated: bool,
    pub metadata_updated: bool,
}

impl Commit {
    pub(crate) fn short_hash(&self) -> &str {
        self.hash.get(..8).unwrap_or(&self.hash)
    }

    pub(crate) fn attr(&self, key: &str) -> &str {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v)
    }

    pub(crate) fn remote_ref(&self) -> &str {
        self.attr(REMOTE_REF)
    }

    pub(crate) fn set_attr(&mut self, key: &str, value: &str) {
        if let Some((_, v)) = self.attrs.iter_mut().find(|(k, _)| k == key) {
            value.clone_into(v);
        } else {
            self.attrs.push((key.into(), value.into()));
            self.attrs.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }

    pub(crate) fn tags(&self, defaults: &[String]) -> Vec<String> {
        dedup(
            defaults
                .iter()
                .map(|s| s.trim())
                .chain(self.attr("tags").split(',').map(str::trim)),
        )
    }

    pub(crate) fn full_message(&self) -> String {
        #[cfg(test)]
        crate::tests::unit::support::MESSAGE_SERIALISATIONS.update(|count| count + 1);
        trailers::serialise(&self.title, &self.message, &self.attrs)
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reference = self.remote_ref();
        let reference = if reference.is_empty() {
            String::new()
        } else {
            format!("({reference})")
        };
        write!(f, "{} {reference} {}", self.short_hash(), self.title)
    }
}

pub(crate) fn dedup<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|v| !v.is_empty() && seen.insert(*v))
        .map(str::to_owned)
        .collect()
}

pub(crate) fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn shorten_title(title: &str) -> String {
    if title.len() <= 36 {
        return title.into();
    }
    let end = title.floor_char_boundary(36);
    let prefix = &title[..end];
    prefix.rfind(' ').map_or_else(
        || format!("{prefix}..."),
        |i| format!("{} ...", &prefix[..i]),
    )
}
