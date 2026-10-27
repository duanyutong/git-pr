use std::{collections::BTreeMap, ffi::OsString};

use anyhow::{Context, Result};

use crate::{cli::Flags, model::comma_list};

pub(crate) fn unicode_environment(
    values: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::new();
    for (key, value) in values {
        let Ok(key) = key.into_string() else {
            continue;
        };
        let value = value.into_string();
        if key == "HOME" || key.starts_with("GIT_PR_") {
            let value = value.ok().with_context(|| {
                format!("ERROR: environment variable {key} must be valid UTF-8")
            })?;
            environment.insert(key, value);
        } else if let Ok(value) = value {
            environment.insert(key, value);
        }
    }
    Ok(environment)
}

pub(crate) fn enabled(
    flags: &Flags,
    env: &BTreeMap<String, String>,
    name: &str,
    variable: &str,
) -> bool {
    flags.boolean(name) || env.get(variable).is_some_and(|s| s == "1")
}

pub(crate) fn fallback<'a>(
    flag: &'a str,
    env: &'a BTreeMap<String, String>,
    variable: &str,
) -> &'a str {
    if flag.is_empty() {
        env.get(variable).map_or("", String::as_str)
    } else {
        flag
    }
}

pub(crate) fn github_stack(
    no_stack: bool,
    cli: Option<bool>,
    env: Option<&str>,
    git: Option<bool>,
) -> bool {
    !no_stack
        && cli
            .or_else(|| env.map(|s| s == "1" || s.eq_ignore_ascii_case("true")))
            .or(git)
            .unwrap_or(false)
}

pub(crate) fn labels(cli: &[String], configured: &str) -> Vec<String> {
    if cli.is_empty() {
        comma_list(configured)
    } else {
        cli.to_vec()
    }
}

pub(crate) fn preview_limit(value: Option<&str>) -> Result<usize> {
    value.map_or(Ok(20), |value| {
        value
            .parse::<usize>()
            .context("git-pr.previewLimit must be a non-negative integer (0 shows all commits)")
    })
}
