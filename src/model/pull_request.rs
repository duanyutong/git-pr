//! Pull-request values exchanged between policy and service adapters.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct PullRequest {
    pub number: u64,
    pub node_id: String,
    #[serde(deserialize_with = "nullable_body")]
    pub body: Arc<str>,
    pub title: String,
    pub labels: Vec<Label>,
    pub state: String,
    pub merged: bool,
    pub merged_at: Option<String>,
    pub draft: Option<bool>,
    pub head: Reference,
    pub base: Reference,
    pub stack: Option<NativeStack>,
    /// None denotes missing evidence; an explicit API null denotes disabled auto-merge.
    #[serde(deserialize_with = "auto_merge")]
    pub auto_merge: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct Reference {
    #[serde(rename = "ref")]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct NativeStack {
    pub number: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Label {
    Name(String),
    Object { name: String },
}
impl Label {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Name(name) | Self::Object { name } => name,
        }
    }
}

pub(crate) fn nullable_body<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Arc<str>, D::Error> {
    Option::<Arc<str>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

pub(crate) fn auto_merge<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<bool>, D::Error> {
    Option::<serde_json::Value>::deserialize(deserializer).and_then(|request| {
        let Some(request) = request else {
            return Ok(Some(false));
        };
        if request["enabledAt"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
            || matches!(
                request["merge_method"].as_str(),
                Some("merge" | "squash" | "rebase")
            )
        {
            return Ok(Some(true));
        }
        Err(serde::de::Error::custom("invalid auto-merge observation"))
    })
}
impl PullRequest {
    pub(crate) const fn is_merged(&self) -> bool {
        self.merged || self.merged_at.is_some()
    }

    pub(crate) fn summary(&self) -> PullRequestSummary {
        PullRequestSummary {
            number: self.number,
            title: Some(self.title.clone()),
            lifecycle: Lifecycle::observed(&self.state, Some(self.is_merged()), self.draft),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Open,
    Draft,
    Merged,
    Closed,
    Unknown,
}

impl Lifecycle {
    pub(crate) fn observed(state: &str, merged: Option<bool>, draft: Option<bool>) -> Self {
        match (state, merged, draft) {
            ("open" | "closed" | "merged", Some(true), _) => Self::Merged,
            ("closed", Some(false), _) => Self::Closed,
            ("open", Some(false), Some(true)) => Self::Draft,
            ("open", Some(false), Some(false)) => Self::Open,
            _ => Self::Unknown,
        }
    }
}

/// Presentation evidence; never used to authorise publication or replace a PR.
#[derive(Clone, Debug)]
pub(crate) struct PullRequestSummary {
    pub number: u64,
    pub title: Option<String>,
    pub lifecycle: Lifecycle,
}

/// Complete metadata acquired only when publication requires a base transition.
/// Missing fields must not turn an uncertain observation into recovery authority.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct RecoveryPullRequest {
    pub number: u64,
    pub node_id: String,
    pub state: String,
    pub merged: bool,
    pub merged_at: Option<String>,
    pub head: RecoveryReference,
    pub base: RecoveryReference,
    pub merge_commit_sha: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub stack: Option<NativeStack>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct RecoveryReference {
    #[serde(rename = "ref")]
    pub name: String,
    pub sha: String,
    pub repo: RecoveryRepository,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct RecoveryRepository {
    pub full_name: String,
}
