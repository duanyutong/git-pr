//! Git owns configuration parsing, scope reporting and atomic file updates.

use super::{Attempt, Binding, Head, parse_review, review_url};
use crate::{commands::Commands, model::Repository};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

pub(crate) const PATTERN: &str = r"^(branch\..*\.(gitprhead|gitprpullrequest|gitprchangeid|github-pr-owner-number|remote|merge)|git-pr\.pendingcreate)$";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub scope: String,
    pub origin: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Records(pub BTreeMap<String, Vec<Record>>);

impl Records {
    pub(crate) fn read(commands: &Commands) -> Result<Self> {
        Self::read_pattern(commands, PATTERN, false)
    }

    fn read_key(commands: &Commands, key: &str) -> Result<Self> {
        Self::read_pattern(
            commands,
            &format!("^{}$", regex::escape(&canonical_key(key))),
            false,
        )
    }

    fn read_pattern(commands: &Commands, pattern: &str, local_only: bool) -> Result<Self> {
        let mut args = vec!["config", "--null", "--show-scope", "--show-origin"];
        if local_only {
            args.extend(["--local", "--no-includes"]);
        } else {
            args.push("--includes");
        }
        args.extend(["--get-regexp", pattern]);
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let output = commands
            .runner
            .execute_data("git", &args, &commands.cwd, &commands.env, &[])
            .context("failed to observe publication configuration")?;
        if output.status == 1 && output.stdout.is_empty() && output.stderr.is_empty() {
            return Ok(Self::default());
        }
        if output.status != 0 {
            bail!(
                "publication configuration query failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self::decode(
            &String::from_utf8(output.stdout).context("publication configuration is not UTF-8")?,
        )
    }

    pub(crate) fn decode(output: &str) -> Result<Self> {
        let data = output
            .strip_suffix('\0')
            .context("incomplete publication configuration output")?;
        let fields: Vec<_> = data.split('\0').collect();
        if !fields.len().is_multiple_of(3) {
            bail!("incomplete publication configuration record");
        }
        let mut records = Self::default();
        for fields in fields.as_chunks::<3>().0 {
            let (key, value) = fields[2].split_once('\n').unwrap_or((fields[2], ""));
            records.0.entry(key.into()).or_default().push(Record {
                scope: fields[0].into(),
                origin: fields[1].into(),
                value: value.into(),
            });
        }
        Ok(records)
    }

    pub(crate) fn claim(&self, anchor: &str, field: &str) -> Result<Option<&str>> {
        self.single(&format!("branch.{anchor}.{field}"))
    }

    pub(crate) fn hint(&self, key: &str) -> Option<&str> {
        self.0.get(key)?.last().map(|record| record.value.as_str())
    }

    pub(crate) fn single(&self, key: &str) -> Result<Option<&str>> {
        let Some(records) = self.0.get(key) else {
            return Ok(None);
        };
        let mut value = None;
        for record in records {
            if record.scope != "local" {
                bail!(
                    "{key} at {} has unsupported {} scope; publication identity must be repository-local",
                    record.origin,
                    record.scope
                );
            }
            if value.is_some_and(|v| !equivalent(key, v, &record.value)) {
                bail!(
                    "conflicting definitions of {key} at {}; explicitly reconcile their origins",
                    record.origin
                );
            }
            value = Some(record.value.as_str());
        }
        Ok(value)
    }

    pub(crate) fn anchors(&self) -> impl Iterator<Item = &str> {
        self.0.keys().filter_map(|key| {
            key.strip_prefix("branch.")?
                .rsplit_once('.')
                .map(|(anchor, _)| anchor)
        })
    }

    pub(crate) fn attempts(&self, repo: &Repository) -> Result<Vec<Attempt>> {
        self.0
            .get("git-pr.pendingcreate")
            .into_iter()
            .flatten()
            .map(|record| {
                if record.scope != "local" {
                    bail!(
                        "git-pr.pendingCreate must be repository-local ({})",
                        record.origin
                    );
                }
                Attempt::parse(&record.value, repo)
            })
            .collect()
    }

    pub(crate) fn head(&self, anchor: &str, repo: &Repository) -> Result<Option<Head>> {
        self.claim(anchor, "gitprhead")?
            .map(|s| Head::parse(s, repo))
            .transpose()
    }

    pub(crate) fn review(&self, anchor: &str, repo: &Repository) -> Result<Option<u64>> {
        self.claim(anchor, "gitprpullrequest")?
            .map(|s| parse_review(s, repo))
            .transpose()
    }

    pub(crate) fn relocation(
        &self,
        commands: &Commands,
        anchor: &str,
    ) -> Result<Vec<(String, String)>> {
        // Git reports origins relative to its repository context, which need
        // not be the caller's directory or its canonical filesystem spelling.
        // A direct local query establishes which definitions Git will edit.
        let local = Self::read_pattern(commands, PATTERN, true)?;
        let mut values = Vec::new();
        for field in ["gitprhead", "gitprpullrequest", "gitprchangeid"] {
            let key = format!("branch.{anchor}.{field}");
            if let Some(value) = self.single(&key)? {
                for record in self.0.get(&key).into_iter().flatten() {
                    if !local
                        .0
                        .get(&key)
                        .is_some_and(|definitions| definitions.contains(record))
                    {
                        bail!(
                            "cannot relocate {key} from {}; explicitly move the association at its origin",
                            record.origin
                        );
                    }
                }
                values.push((key, value.into()));
            }
        }
        Ok(values)
    }
}

pub(crate) fn assignments(
    repo: &Repository,
    binding: &Binding,
    change: &str,
    review: u64,
) -> Vec<(String, String)> {
    let Some(anchor) = &binding.anchor else {
        return Vec::new();
    };
    let mut values = vec![(format!("branch.{anchor}.gitPrHead"), binding.head.value())];
    if repo.jj && !change.is_empty() {
        values.push((format!("branch.{anchor}.gitPrChangeId"), change.into()));
    }
    if review != 0 {
        values.push((
            format!("branch.{anchor}.gitPrPullRequest"),
            review_url(repo, review),
        ));
    }
    values
}

pub(crate) fn persist_observed(
    commands: &Commands,
    records: &Records,
    values: &[(String, String)],
) -> Result<()> {
    for (key, value) in values {
        match records.single(&canonical_key(key))? {
            Some(existing) if equivalent(key, existing, value) => {}
            Some(_) => bail!(
                "publication configuration changed at {key}; refusing to overwrite its association"
            ),
            None => insert(commands, key, value)?,
        }
    }
    Ok(())
}

pub(crate) fn persist(
    commands: &Commands,
    repo: &Repository,
    binding: &Binding,
    change: &str,
    review: u64,
) -> Result<()> {
    for (key, value) in assignments(repo, binding, change, review) {
        insert(commands, &key, &value)?;
    }
    Ok(())
}

fn canonical_key(key: &str) -> String {
    let mut key = key.to_owned();
    if let Some(i) = key.rfind('.') {
        key[i + 1..].make_ascii_lowercase();
    }
    key
}

fn equivalent(key: &str, left: &str, right: &str) -> bool {
    let key = canonical_key(key);
    if key.ends_with(".gitprhead") {
        if let (Some((left_repo, left_ref)), Some((right_repo, right_ref))) =
            (left.split_once(' '), right.split_once(' '))
        {
            return left_repo.eq_ignore_ascii_case(right_repo) && left_ref == right_ref;
        }
    } else if key.ends_with(".gitprpullrequest") {
        return left.eq_ignore_ascii_case(right);
    }
    left == right
}

pub(crate) fn insert(commands: &Commands, key: &str, value: &str) -> Result<()> {
    let query = canonical_key(key);
    let records = Records::read_key(commands, key)?;
    match records.single(&query)? {
        Some(existing) if equivalent(key, existing, value) => return Ok(()),
        Some(_) => bail!(
            "publication configuration changed at {key}; refusing to overwrite its association"
        ),
        None => {}
    }
    commands.git(&["config", "--local", "--add", key, value])?;
    let observed = Records::read_key(commands, key)?;
    if !observed
        .single(&query)?
        .is_some_and(|observed| equivalent(key, observed, value))
    {
        bail!("publication configuration write could not be verified at {key}");
    }
    Ok(())
}

pub(crate) fn begin(commands: &Commands, repo: &Repository, attempt: &Attempt) -> Result<()> {
    if Records::read_key(commands, "git-pr.pendingCreate")?
        .attempts(repo)?
        .iter()
        .any(|pending| pending.head == attempt.head)
    {
        bail!(
            "PR creation for {:?} has an unresolved attempt; verify its outcome before explicitly clearing git-pr.pendingCreate",
            attempt.head.reference
        );
    }
    commands.git(&[
        "config",
        "--local",
        "--add",
        "git-pr.pendingCreate",
        &attempt.value(),
    ])?;
    if !Records::read_key(commands, "git-pr.pendingCreate")?
        .attempts(repo)?
        .contains(attempt)
    {
        bail!("pending creation record could not be verified");
    }
    Ok(())
}

pub(crate) fn finish(commands: &Commands, repo: &Repository, reference: &str) -> Result<()> {
    for attempt in Records::read_key(commands, "git-pr.pendingCreate")?
        .attempts(repo)?
        .iter()
        .filter(|a| a.head.reference == reference)
    {
        commands.git(&["config", "--local", "--fixed-value", "--unset-all", "git-pr.pendingCreate", &attempt.value()])
            .context("PR exists, but its pending creation record could not be removed; retain the PR and retry")?;
    }
    Ok(())
}

pub(crate) fn retire(commands: &Commands, values: &[(String, String)]) -> Result<()> {
    for (key, value) in values {
        let records = Records::read_key(commands, key)?;
        match records.single(key)? {
            None => continue,
            Some(observed) if observed == value => {}
            Some(_) => bail!("former association {key} changed during relocation"),
        }
        commands.git(&[
            "config",
            "--local",
            "--fixed-value",
            "--unset-all",
            key,
            value,
        ])?;
        if Records::read_key(commands, key)?.single(key)?.is_some() {
            bail!("former association {key} could not be retired; inspect its origins");
        }
    }
    Ok(())
}
