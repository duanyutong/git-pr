//! Publication observations taken from the resolved push destination.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

use super::{Git, policy};

pub(super) fn lease_args(
    commits: &[&crate::model::Commit],
    leases: Option<&BTreeMap<String, Option<String>>>,
) -> Result<Option<Vec<String>>> {
    leases
        .map(|leases| {
            commits
                .iter()
                .map(|commit| {
                    let reference = commit.remote_ref();
                    let expected = leases.get(reference).context("missing publication lease")?;
                    Ok(format!(
                        "--force-with-lease=refs/heads/{reference}:{}",
                        expected.as_deref().unwrap_or_default()
                    ))
                })
                .collect()
        })
        .transpose()
}

impl Git<'_> {
    pub(crate) fn recovery_destination(&self) -> Result<String> {
        let destinations = self.commands.git(&[
            "remote",
            "get-url",
            "--push",
            "--all",
            "--",
            &self.repo.remote,
        ])?;
        let mut lines = destinations.lines();
        let destination = lines
            .next()
            .context("missing publication push destination")?;
        if lines.next().is_some() {
            bail!("publication requires a single verified push destination");
        }
        let repository = policy::parse_remote_patterns(
            &format!("remote {destination}"),
            &policy::REMOTE_PATTERNS,
        )?;
        if !repository.host.eq_ignore_ascii_case(&self.repo.host)
            || !repository.name.eq_ignore_ascii_case(&self.repo.name)
        {
            bail!("publication push destination does not match the observed GitHub repository");
        }
        Ok(destination.to_owned())
    }

    pub(crate) fn recovery_refs(
        &self,
        destination: &str,
        references: &[String],
    ) -> Result<BTreeMap<String, Option<String>>> {
        let mut result = BTreeMap::new();
        for chunk in references.chunks(128) {
            let refs: Vec<_> = chunk
                .iter()
                .map(|reference| format!("refs/heads/{reference}"))
                .collect();
            let mut args = vec!["ls-remote", "--refs", "--", destination];
            args.extend(refs.iter().map(String::as_str));
            let response = self.commands.git(&args)?;
            for (reference, full) in chunk.iter().zip(&refs) {
                let mut found = None;
                for line in response.lines() {
                    let (oid, name) = line
                        .split_once('\t')
                        .context("malformed recovery ref observation")?;
                    if name == full {
                        if !policy::valid_oid(oid) || found.is_some() {
                            bail!("invalid or duplicate recovery ref {full:?}");
                        }
                        found = Some(oid.to_owned());
                    }
                }
                result.insert(reference.clone(), found);
            }
        }
        Ok(result)
    }
}
