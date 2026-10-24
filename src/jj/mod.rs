pub(crate) mod policy;

use policy::parse_working_copy;

use anyhow::{Context, Result, bail};

use crate::{commands::Commands, model::Commit};

pub(crate) fn resolve_revision(commands: &Commands, revision: &str) -> Result<String> {
    let output = commands.jj_data(&[
        "--ignore-working-copy",
        "log",
        "-r",
        revision,
        "--no-graph",
        "-T",
        "commit_id ++ \"\\n\"",
    ])?;
    let lines: Vec<_> = output
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    match lines.as_slice() {
        [] => bail!("jj revision {revision:?} matched no commits"),
        [hash] => Ok((*hash).into()),
        _ => bail!(
            "jj revision {revision:?} matched {} commits; expected exactly one",
            lines.len()
        ),
    }
}

/// Resolve each argument independently within a bounded, immutable observation.
pub(crate) fn resolve_revisions(commands: &Commands, revisions: &[String]) -> Result<Vec<String>> {
    if revisions.is_empty() {
        return Ok(Vec::new());
    }
    let operation = commands.jj_data(&[
        "--ignore-working-copy",
        "op",
        "log",
        "--limit",
        "1",
        "--no-graph",
        "-T",
        "id",
    ])?;
    let mut hashes = Vec::with_capacity(revisions.len());
    let mut remaining = revisions;
    while !remaining.is_empty() {
        let (revset, template, count) = policy::revision_query(remaining)?;
        let output = commands.jj_data(&[
            "--at-operation",
            &operation,
            "log",
            "-r",
            &revset,
            "--no-graph",
            "-T",
            &template,
        ])?;
        hashes.extend(policy::parse_resolutions(&output, &remaining[..count])?);
        remaining = &remaining[count..];
    }
    Ok(hashes)
}

#[cfg(test)]
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

pub(crate) fn working_copy(commands: &Commands) -> Result<Option<Commit>> {
    let data = commands.jj_data(&["--ignore-working-copy", "log", "-r", "@", "--no-graph", "-T", "conflict ++ \"\\0\" ++ if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\") ++ \"\\0\" ++ author.name() ++ \"\\0\" ++ author.email() ++ \"\\0\" ++ change_id ++ \"|\" ++ commit_id ++ \"|\" ++ description"])?;
    let fields: Vec<_> = data.splitn(5, '\0').collect();
    let [conflict, check, author_name, author_email, info] = fields.as_slice() else {
        anyhow::bail!("incomplete working-copy observation");
    };
    let conflicted = policy::parse_conflict(conflict)?;
    parse_working_copy(check, info, author_name, author_email)
        .map(|commit| {
            commit.map(|mut commit| {
                commit.conflicted = conflicted;
                commit
            })
        })
        .context("failed to read Jujutsu working-copy metadata")
}

/// Read all change identities without repeatedly snapshotting the working copy.
pub(crate) fn fill_change_ids(commands: &Commands, commits: &mut [Commit]) -> Result<()> {
    if commits.is_empty() {
        return Ok(());
    }
    let operation = commands.jj_data(&[
        "--ignore-working-copy",
        "op",
        "log",
        "--limit",
        "1",
        "--no-graph",
        "-T",
        "id",
    ])?;
    for chunk in commits.chunks_mut(128) {
        let revisions = chunk
            .iter()
            .map(|c| c.hash.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        let output = commands.jj_data(&[
            "--at-operation",
            &operation,
            "log",
            "-r",
            &revisions,
            "--no-graph",
            "-T",
            "commit_id ++ \" \" ++ change_id ++ \" \" ++ conflict ++ \"\\n\"",
        ])?;
        let identities: std::collections::HashMap<_, _> = output
            .lines()
            .map(|line| {
                let (hash, metadata) = line
                    .split_once(' ')
                    .context("missing Jujutsu change identity")?;
                let (change, conflict) = metadata
                    .split_once(' ')
                    .context("missing Jujutsu conflict marker")?;
                Ok((hash, (change, policy::parse_conflict(conflict)?)))
            })
            .collect::<Result<_>>()?;
        for commit in chunk {
            let &(change, conflicted) = identities
                .get(commit.hash.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing Jujutsu identity for {}", commit.hash))?;
            commit.change_id = change.to_owned();
            commit.conflicted = conflicted;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct Bookmark {
    pub conflicted: bool,
    pub targets: Vec<(String, String)>,
}

pub(crate) fn bookmarks(
    commands: &Commands,
) -> Result<std::collections::HashMap<String, Bookmark>> {
    let output = commands.jj_data(&["--ignore-working-copy", "bookmark", "list", "-T", "name ++ \"\\0\" ++ if(conflict, \"1\", \"0\") ++ \"\\0\" ++ added_targets.map(|c| c.commit_id() ++ \" \" ++ c.change_id()).join(\",\") ++ \"\\n\""])?;
    let mut bookmarks = std::collections::HashMap::new();
    for line in output.lines() {
        let mut fields = line.splitn(3, '\0');
        let name = fields.next().unwrap_or("");
        let conflicted = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid bookmark observation"))?
            == "1";
        let targets = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing bookmark targets"))?
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|target| {
                let (oid, change) = target
                    .split_once(' ')
                    .ok_or_else(|| anyhow::anyhow!("invalid bookmark identity"))?;
                Ok((oid.to_owned(), change.to_owned()))
            })
            .collect::<Result<Vec<_>>>()?;
        bookmarks.insert(
            name.into(),
            Bookmark {
                conflicted,
                targets,
            },
        );
    }
    Ok(bookmarks)
}
