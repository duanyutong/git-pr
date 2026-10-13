//! Parsed submission checkpoints shared by configuration and orchestration.

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Checkpoint {
    Validate,
    GetCommits,
    Rewrite,
    Push,
    PrCreate,
}

impl Checkpoint {
    pub(crate) fn parse(value: &str) -> Result<Option<Self>> {
        match value {
            "" => Ok(None),
            "validate" => Ok(Some(Self::Validate)),
            "get-commits" => Ok(Some(Self::GetCommits)),
            "rewrite" => Ok(Some(Self::Rewrite)),
            "push" => Ok(Some(Self::Push)),
            "pr-create" => Ok(Some(Self::PrCreate)),
            _ => bail!(
                "ERROR: invalid --stop-after {value:?}; must be one of: validate, get-commits, rewrite, push, pr-create"
            ),
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::GetCommits => "get-commits",
            Self::Rewrite => "rewrite",
            Self::Push => "push",
            Self::PrCreate => "pr-create",
        }
    }
}
