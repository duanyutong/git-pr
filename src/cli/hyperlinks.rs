//! Explicit hyperlink policy and inert, validated PR destinations.

use crate::model::Repository;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Auto,
    Always,
    Never,
}

impl Mode {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err(format!(
                "invalid hyperlink mode {value:?}; expected auto, always or never"
            )),
        }
    }

    /// Rich-output eligibility is enforced separately by the terminal boundary.
    pub(crate) const fn enabled(self) -> bool {
        match self {
            // OSC 8 has no portable capability query; eligible interactive output
            // uses links without relying on emulator names or connection metadata.
            Self::Auto | Self::Always => true,
            Self::Never => false,
        }
    }
}

pub(crate) fn destination(repo: &Repository, number: u64) -> Option<String> {
    if repo
        .host
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || "/\\?#@".contains(c))
        || repo.name.chars().any(char::is_control)
    {
        return None;
    }
    let (owner, name) = repo.name.split_once('/')?;
    if [owner, name]
        .iter()
        .any(|part| matches!(*part, "" | "." | ".."))
        || name.contains('/')
    {
        return None;
    }
    let mut url = reqwest::Url::parse(&format!("https://{}/", repo.host)).ok()?;
    let appended = url.path_segments_mut().ok().map(|mut segments| {
        segments.extend([owner, name, "pull", &number.to_string()]);
    });
    appended.map(|()| url.into())
}

pub(crate) fn link(label: &str, destination: &str) -> String {
    format!("\x1b]8;;{destination}\x1b\\{label}\x1b]8;;\x1b\\")
}
