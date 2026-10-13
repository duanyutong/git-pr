//! Native credential-store boundaries.

use super::CredentialStore;
#[cfg(target_os = "macos")]
use super::keychain::decode_keychain;
use crate::commands::Commands;
#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use anyhow::bail;

#[derive(Debug)]
pub(crate) struct SystemCredentials;

impl CredentialStore for SystemCredentials {
    fn get(&self, service: &str, commands: &Commands) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            // Disable command logging: the command output is the credential itself.
            let mut commands = commands.clone();
            commands.verbose = false;
            decode_keychain(&commands.run(
                "/usr/bin/security",
                &["find-generic-password", "-s", service, "-wa", ""],
            )?)
        }
        #[cfg(target_os = "linux")]
        {
            let _ = commands;
            let service_api = dbus_secret_service::SecretService::connect(
                dbus_secret_service::EncryptionType::Plain,
            )?;
            let collection = service_api.get_collection_by_alias("login")?;
            collection.unlock()?;
            let items = collection.search_items(std::collections::HashMap::from([
                ("username", ""),
                ("service", service),
            ]))?;
            let item = items.first().context("credential not found")?;
            Ok(String::from_utf8(item.get_secret()?)?)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (service, commands);
            bail!("credential store unavailable on this platform")
        }
    }
}
