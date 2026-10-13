#[cfg(any(target_os = "macos", test))]
pub(crate) mod keychain;

mod system;
pub(crate) use system::SystemCredentials;

use std::{collections::BTreeMap, fmt};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::commands::Commands;

#[derive(Clone, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct Secret(pub String);
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

pub(crate) trait CredentialStore: fmt::Debug + Send + Sync {
    fn get(&self, service: &str, commands: &Commands) -> Result<String>;
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Host {
    oauth_token: Secret,
}

pub(crate) fn load(
    hosts_path: &std::path::Path,
    host: &str,
    store: &dyn CredentialStore,
    commands: &Commands,
    files: &dyn crate::config::files::Files,
) -> Result<Secret> {
    let data = files.read(hosts_path).with_context(|| format!("ERROR: failed to load GitHub credential configuration at {}\nCheck file access and UTF-8 encoding. If this host is not configured, sign in using `gh auth login`", hosts_path.display()))?;
    // YAML diagnostics may quote the source line containing the credential.
    let mut hosts: BTreeMap<String, Option<Host>> = serde_saphyr::from_str(&data).map_err(|_| anyhow::anyhow!(
        "ERROR: invalid GitHub credential configuration at {}; expected a YAML mapping of hosts with string oauth_token values. Check the file syntax or sign in again with `gh auth login`; credential contents are omitted",
        hosts_path.display()
    ))?;
    let Some(Some(mut entry)) = hosts.remove(host) else {
        bail!(
            "ERROR: no GitHub config for host {host}\n\nHint: Check {}\nRun `gh auth login` and choose your GitHub host",
            hosts_path.display()
        );
    };
    if entry.oauth_token.0.is_empty() {
        entry.oauth_token.0 = store
            .get(&format!("gh:{host}"), commands)
            .with_context(|| format!("ERROR: failed to read the credential store for {host}; unlock the store or sign in again with `gh auth login`"))?;
    }
    if entry.oauth_token.0.is_empty() {
        bail!(
            "ERROR: no GitHub token found for host {host:?}\n\nHint: use github cli to login to your account:\n\n      gh auth login"
        );
    }
    Ok(entry.oauth_token)
}
