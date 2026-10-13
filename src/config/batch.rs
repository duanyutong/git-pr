//! Batched Git configuration queries; Git owns parsing, conversion and scope ordering.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use crate::commands::Commands;

#[derive(Default)]
pub(super) struct Values(BTreeMap<String, String>);

impl Values {
    pub(super) fn read(commands: &Commands, keys: &[&str], kind: Option<&str>) -> Result<Self> {
        Self::read_patterns(commands, keys, kind, &[])
    }

    pub(super) fn read_patterns(
        commands: &Commands,
        keys: &[&str],
        kind: Option<&str>,
        patterns: &[&str],
    ) -> Result<Self> {
        if keys.is_empty() && patterns.is_empty() {
            return Ok(Self::default());
        }
        let pattern = format!(
            "^({})$",
            keys.iter()
                .map(|key| regex::escape(&key.to_ascii_lowercase()))
                .chain(patterns.iter().map(|pattern| (*pattern).to_owned()))
                .collect::<Vec<_>>()
                .join("|")
        );
        let mut args = vec!["config".to_owned(), "--null".into()];
        if let Some(kind) = kind {
            args.push(format!("--type={kind}"));
        }
        args.extend(["--get-regexp".into(), pattern]);
        let output = commands
            .runner
            .execute_data("git", &args, &commands.cwd, &commands.env, &[])
            .context("failed to execute Git configuration query")?;
        if output.status == 1 && output.stdout.is_empty() && output.stderr.is_empty() {
            return Ok(Self::default());
        }
        if output.status != 0 {
            if let Some(kind) = kind {
                // Preserve Git's per-setting conversion failures and diagnostics
                // when a typed batch cannot be read successfully.
                return Self::scalars(commands, keys, kind);
            }
            bail!(
                "failed to read Git configuration: exit code {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self::decode(
            &String::from_utf8(output.stdout)
                .context("Git configuration output is not valid UTF-8")?,
        )
    }

    fn scalars(commands: &Commands, keys: &[&str], kind: &str) -> Result<Self> {
        let mut values = BTreeMap::new();
        for key in keys {
            if let Some(value) = super::git_value(
                commands,
                &["config", &format!("--type={kind}"), "--get", key],
            )? {
                values.insert(key.to_ascii_lowercase(), value);
            }
        }
        Ok(Self(values))
    }

    fn decode(output: &str) -> Result<Self> {
        let mut values = BTreeMap::new();
        for record in output
            .strip_suffix('\0')
            .context("incomplete Git configuration records")?
            .split('\0')
        {
            let (key, value) = record.split_once('\n').unwrap_or((record, ""));
            // --null frames values containing newlines; output order is Git's scope order.
            values.insert(key.to_owned(), value.trim().to_owned());
        }
        Ok(Self(values))
    }

    pub(super) fn get(&self, key: &str) -> Option<&str> {
        // Section and variable names are case-insensitive; subsections are not.
        let mut key = key.to_owned();
        if let Some(first) = key.find('.') {
            key[..first].make_ascii_lowercase();
        }
        if let Some(last) = key.rfind('.') {
            key[last + 1..].make_ascii_lowercase();
        }
        self.0.get(&key).map(String::as_str)
    }

    pub(super) fn needs_publication_branch(&self) -> bool {
        let has_default = self.0.contains_key("remote.pushdefault");
        self.0.keys().any(|key| {
            key.starts_with("branch.")
                && (key.ends_with(".pushremote") || (!has_default && key.ends_with(".remote")))
        })
    }

    pub(super) fn string(&self, key: &str) -> String {
        self.get(key).unwrap_or_default().to_owned()
    }

    pub(super) fn boolean(&self, key: &str) -> Option<bool> {
        self.get(key).map(|value| value == "true")
    }
}

#[cfg(test)]
#[path = "../tests/unit/configuration.rs"]
mod unit;
