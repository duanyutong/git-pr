//! Test support excluded from production coverage.

use anyhow::{Result, bail};

use crate::commands::Commands;

pub(crate) fn change_id(commands: &Commands, hash: &str) -> Result<String> {
    let output = commands.jj_data(&[
        "--ignore-working-copy",
        "log",
        "-r",
        hash,
        "--no-graph",
        "-T",
        "change_id",
    ])?;
    match output.lines().rev().map(str::trim).find(|s| !s.is_empty()) {
        Some(id) => Ok(id.into()),
        None => bail!("failed to parse change ID from jj output: {output}"),
    }
}
