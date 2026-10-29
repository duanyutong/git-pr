#[cfg(test)]
#[path = "../tests/git_helpers.rs"]
mod test_helpers;

pub(crate) mod policy;
mod publication;
mod snapshot;

use policy::{CommitDisposition, short_hash};

use std::collections::HashSet;

use anyhow::{Context, Result, bail};

use crate::{
    commands::Commands,
    jj,
    model::{Commit, Repository, shorten_title},
    stack::selection::{self, RefSync},
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Git<'a> {
    pub commands: &'a Commands,
    pub repo: &'a Repository,
}

pub(crate) fn resolve_ref(commands: &Commands, reference: &str) -> Result<String> {
    commands.git(&["rev-parse", "--verify", &format!("{reference}^{{commit}}")])
}

pub(crate) fn resolve_refs(commands: &Commands, references: &[String]) -> Result<Vec<String>> {
    let mut identities = Vec::with_capacity(references.len());
    let mut remaining = references;
    while !remaining.is_empty() {
        let mut bytes = 0;
        let count = remaining
            .iter()
            .take(128)
            .take_while(|reference| {
                bytes += reference.len() + 10;
                bytes <= 64 * 1024
            })
            .count();
        if count == 0 {
            bail!("revision exceeds the 64 KiB argument budget");
        }
        let (chunk, rest) = remaining.split_at(count);
        remaining = rest;
        let revisions: Vec<_> = chunk
            .iter()
            .map(|reference| format!("{reference}^{{commit}}"))
            .collect();
        let mut args = vec!["rev-parse", "--revs-only", "--end-of-options"];
        args.extend(revisions.iter().map(String::as_str));
        let output = commands.git_data(&args, &[])?;
        let hashes: Vec<_> = output.lines().collect();
        if hashes.len() != chunk.len()
            || hashes.iter().any(|hash| {
                !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|c| c.is_ascii_hexdigit())
            })
        {
            bail!("incomplete or invalid resolved commit identities");
        }
        identities.extend(hashes.into_iter().map(str::to_owned));
    }
    Ok(identities)
}

#[derive(Debug)]
pub(crate) struct StackObservation {
    pub(crate) commits: Vec<Commit>,
    pub(crate) depths: std::collections::HashMap<String, usize>,
    pub(crate) tip_hash: String,
    pub(crate) base_hash: String,
}

impl Git<'_> {
    pub(crate) fn head(&self) -> Result<String> {
        if self.repo.jj {
            self.commands
                .jj_data(&[
                    "--ignore-working-copy",
                    "log",
                    "-r",
                    "@-",
                    "--no-graph",
                    "-T",
                    "commit_id",
                ])
                .context("ERROR: failed to resolve jj @-")
        } else {
            Ok("HEAD".into())
        }
    }

    pub(crate) fn clean(&self) -> Result<bool> {
        if self.repo.jj {
            let status = self.commands.jj_data(&["--ignore-working-copy", "log", "-r", "@", "--no-graph", "-T", "if(empty, \"EMPTY\", \"NONEMPTY\") ++ \"|\" ++ if(description, \"HAS-DESC\", \"NO-DESC\")"])
                .context("could not inspect the Jujutsu working copy")?;
            if !matches!(
                status.as_str(),
                "EMPTY|NO-DESC" | "EMPTY|HAS-DESC" | "NONEMPTY|HAS-DESC" | "NONEMPTY|NO-DESC"
            ) {
                bail!(
                    "invalid Jujutsu working-copy status; expected emptiness and description markers"
                );
            }
            return Ok(jj::policy::clean_status(&status));
        }
        self.commands
            .git(&["status", "--porcelain=v1", "--untracked-files=no"])
            .map(|s| s.is_empty())
            .context("could not inspect working-tree changes")
    }

    pub(crate) fn observe_stack(
        &self,
        base: &str,
        tip: &str,
        include_working_copy: bool,
    ) -> Result<StackObservation> {
        let (snapshot, base_hash) = self.read_stack(base, tip)?;
        let snapshot::Snapshot {
            mut commits,
            trees,
            parents,
        } = snapshot;
        let depths = snapshot::linear_depths(&commits, &parents);
        if self.repo.jj {
            jj::fill_change_ids(self.commands, &mut commits)?;
        }
        let mut tip_hash = commits
            .first()
            .map_or_else(|| tip.to_owned(), |c| c.hash.clone());
        commits.reverse();
        if include_working_copy
            && self.repo.jj
            && let Some(commit) = jj::working_copy(self.commands)?
        {
            tip_hash.clone_from(&commit.hash);
            commits.push(commit);
        }
        self.validate_commits(&mut commits, &trees, &parents)?;
        Ok(StackObservation {
            commits,
            depths,
            tip_hash,
            base_hash,
        })
    }

    /// Description context requires commit metadata, without rewrite-recovery identities.
    pub(crate) fn description_commits(&self, base: &str, tip: &str) -> Result<Vec<Commit>> {
        let (snapshot, _) = self.read_stack(base, tip)?;
        let snapshot::Snapshot {
            mut commits,
            trees,
            parents,
        } = snapshot;
        commits.reverse();
        self.validate_commits(&mut commits, &trees, &parents)?;
        Ok(commits)
    }

    fn read_history(&self, base: &str, tip: &str) -> Result<snapshot::Snapshot> {
        let output = self
            .commands
            .git_data(
                &[
                    "log",
                    snapshot::FORMAT,
                    "--no-decorate",
                    "--no-color",
                    "--encoding=UTF-8",
                    "--no-show-signature",
                    &format!("{base}..{tip}"),
                ],
                &[],
            )
            .with_context(|| format!("failed to find common ancestor for {base} and {tip}"))?;
        snapshot::parse(&output)
    }

    /// Branch publication retains merge commits and their original stored objects.
    pub(crate) fn branch_history(&self, base: &str, tip: &str) -> Result<Vec<Commit>> {
        Ok(self.read_history(base, tip)?.commits)
    }

    fn read_stack(&self, base: &str, tip: &str) -> Result<(snapshot::Snapshot, String)> {
        let mut snapshot = self.read_history(base, tip)?;
        snapshot.reject_merges()?;
        let base_tree =
            self.commands
                .git(&["show", "--no-show-signature", "-s", "--format=%H %T", base])?;
        let mut fields = base_tree.split_whitespace();
        let (Some(oid), Some(tree), None) = (fields.next(), fields.next(), fields.next()) else {
            bail!("invalid base commit/tree observation for {base}");
        };
        snapshot.trees.insert(oid.into(), tree.into());
        Ok((snapshot, oid.into()))
    }

    fn validate_commits(
        &self,
        commits: &mut Vec<Commit>,
        trees: &std::collections::HashMap<String, String>,
        parents: &std::collections::HashMap<String, Vec<String>>,
    ) -> Result<()> {
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        let mut retained = 0;
        let mut conflicted_ancestor = false;
        for index in 0..commits.len() {
            // Empty rows may be omitted from presentation, but remain reachable
            // from their descendants and must retain their publication barrier.
            conflicted_ancestor |= commits[index].conflicted;
            commits[index].conflicted = conflicted_ancestor;
            let commit = &commits[index];
            let known_empty = parents
                .get(&commit.hash)
                .filter(|p| p.len() == 1)
                .and_then(|p| trees.get(&p[0]))
                .zip(trees.get(&commit.hash))
                .map(|(parent, tree)| parent == tree);
            let empty = match known_empty {
                Some(empty) => empty,
                None => self
                    .commands
                    .git(&[
                        "diff-tree",
                        "--no-commit-id",
                        "--name-only",
                        "-r",
                        &commit.hash,
                    ])
                    .with_context(|| {
                        format!(
                            "could not inspect file changes in commit {}",
                            commit.short_hash()
                        )
                    })?
                    .is_empty(),
            };
            let keep = match policy::classify_commit(&commit.title, empty) {
                CommitDisposition::Keep => true,
                CommitDisposition::SkipUntitledEmpty => {
                    warnings.push(format!(
                        "⚠️  commit {} has empty title and no file changes, skipping",
                        commit.short_hash()
                    ));
                    false
                }
                CommitDisposition::RejectUntitled => {
                    errors.push(format!(
                        "❌ commit {} has empty title but contains file changes (fix required)",
                        commit.short_hash()
                    ));
                    false
                }
                CommitDisposition::SkipEmpty => {
                    warnings.push(format!(
                        "⚠️  commit {} {:?} has no file changes, skipping",
                        commit.short_hash(),
                        shorten_title(&commit.title)
                    ));
                    false
                }
            };
            if keep {
                // Both positions are within the original vector; retained never
                // exceeds index. Keep order without allocating a second stack.
                if retained != index {
                    commits.swap(retained, index);
                }
                retained += 1;
            }
        }
        commits.truncate(retained);
        for warning in warnings {
            self.commands.out(&warning);
        }
        for error in &errors {
            self.commands.out(error);
        }
        if !errors.is_empty() {
            bail!("validation failed, please fix the commits above");
        }
        Ok(())
    }

    pub(crate) fn validate_remote_ref(reference: &str) -> Result<()> {
        if reference.trim() != reference {
            bail!("remote ref has leading or trailing whitespace");
        }
        if reference.starts_with("(HEAD detached ") {
            bail!("remote ref is Git's detached-HEAD display placeholder, not a branch");
        }
        if !policy::valid_branch_name(reference) {
            bail!("invalid branch name {reference:?}");
        }
        Ok(())
    }

    pub(crate) fn push_with_leases(
        &self,
        commits: &[&Commit],
        leases: Option<&std::collections::BTreeMap<String, Option<String>>>,
    ) -> Result<()> {
        for commit in commits {
            policy::protect_trunk(commit.remote_ref(), &self.repo.trunk)?;
        }
        let mut remaining = commits;
        self.commands
            .terminal
            .progress("Branches", 0, commits.len());
        while !remaining.is_empty() {
            // Leave substantial room for the environment and fixed arguments on
            // platforms with a small ARG_MAX; oversized individual refs fail explicitly.
            let mut bytes = 0;
            let count = remaining
                .iter()
                .take(128)
                .take_while(|c| {
                    bytes += c.hash.len() + c.remote_ref().len() + 14;
                    if leases.is_some() {
                        bytes += c.remote_ref().len() + 100;
                    }
                    bytes <= 64 * 1024
                })
                .count();
            if count == 0 {
                bail!("publication refspec exceeds the 64 KiB argument budget");
            }
            let (chunk, rest) = remaining.split_at(count);
            remaining = rest;
            let specs: Vec<_> = chunk
                .iter()
                .map(|c| format!("{}:refs/heads/{}", c.hash, c.remote_ref()))
                .collect();
            let lease_args = publication::lease_args(chunk, leases)?;
            let mut args = vec!["push", "--porcelain", "--no-follow-tags"];
            if let Some(lease_args) = &lease_args {
                args.extend(lease_args.iter().map(String::as_str));
            } else {
                args.push("-f");
            }
            args.extend(["--", &self.repo.remote]);
            args.extend(specs.iter().map(String::as_str));
            self.record_publication(chunk, false);
            if let Err(error) = self.commands.git(&args) {
                let refs: Vec<_> = chunk
                    .iter()
                    .map(|c| format!("refs/heads/{}", c.remote_ref()))
                    .collect();
                let verification_context = || {
                    format!(
                        "publication failed and its remote result could not be verified; some refs may already have changed. Original push failure: {error:#}"
                    )
                };
                // Git resolves push URLs and pushInsteadOf rules; the remote name
                // alone would make ls-remote inspect the fetch destination.
                let destinations = self
                    .commands
                    .git(&[
                        "remote",
                        "get-url",
                        "--push",
                        "--all",
                        "--",
                        &self.repo.remote,
                    ])
                    .with_context(verification_context)?;
                if destinations.is_empty() {
                    bail!(
                        "no push destinations could be verified. {}",
                        verification_context()
                    );
                }
                for destination in destinations.lines() {
                    let mut query = vec!["ls-remote", "--refs", "--", destination];
                    query.extend(refs.iter().map(String::as_str));
                    let observation = self
                        .commands
                        .git(&query)
                        .with_context(verification_context)?;
                    let failed: Vec<_> = chunk
                        .iter()
                        .zip(&refs)
                        .filter(|(c, r)| !selection::remote_ref_matches(&observation, r, &c.hash))
                        .map(|(c, _)| c.remote_ref())
                        .collect();
                    if !failed.is_empty() {
                        bail!(
                            "ERROR: failed to push {} branch(es): {}\n{error:#}",
                            failed.len(),
                            failed.join(", ")
                        );
                    }
                }
            }
            self.record_publication(chunk, true);
            self.commands.terminal.progress(
                "Branches",
                commits.len() - remaining.len(),
                commits.len(),
            );
        }
        Ok(())
    }

    fn record_publication(&self, commits: &[&Commit], confirmed: bool) {
        for commit in commits {
            let operation = crate::model::outcome::Operation::Branch(commit.remote_ref().into());
            if confirmed {
                self.commands.outcomes.confirm(operation);
            } else {
                self.commands.outcomes.start(operation);
            }
        }
    }

    pub(crate) fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let args = ["merge-base", "--is-ancestor", ancestor, descendant].map(str::to_owned);
        let output = self
            .commands
            .runner
            .execute("git", &args, &self.commands.cwd, &self.commands.env)
            .with_context(|| {
                format!("could not compare ancestry of {ancestor} and {descendant}")
            })?;
        if output.status == 1 && output.text.is_empty() {
            return Ok(false);
        }
        output
            .success()
            .map(|_| true)
            .with_context(|| format!("could not compare ancestry of {ancestor} and {descendant}"))
    }

    pub(crate) fn stack_tip(&self, hash: &str) -> Result<String> {
        let output = self.commands.git(&[
            "for-each-ref",
            "--contains",
            hash,
            "--format=%(objectname)",
            "refs/heads/",
        ])?;
        let tips = crate::model::dedup(output.lines().map(str::trim).filter(|s| *s != hash));
        let leaf = selection::pick_stack_leaf(&tips, &|batch| {
            let mut args = vec!["merge-base", "--independent"];
            args.extend(batch.iter().map(String::as_str));
            let output = self.commands.git(&args)?;
            let roots = output.lines().map(str::to_owned).collect::<Vec<_>>();
            if roots.is_empty() || roots.iter().any(|root| !batch.contains(root)) {
                bail!("invalid independent commit identities");
            }
            Ok(roots)
        })?;
        Ok(leaf.unwrap_or_else(|| hash.into()))
    }

    pub(crate) fn sync_refs(&self, commits: &[Commit]) {
        if !commits.iter().any(|c| !c.skip) {
            return;
        }
        if self.repo.jj
            && let Err(error) = self.commands.jj(&["git", "import"])
        {
            self.commands.warn(&format!(
                "could not import published Git refs into Jujutsu: {error:#}; bookmarks are unchanged"
            ));
            return;
        }
        let checked_out: HashSet<_> = if self.repo.jj {
            HashSet::new()
        } else {
            let worktrees = match self.commands.git(&["worktree", "list", "--porcelain"]) {
                Ok(output) => output,
                Err(error) => {
                    self.commands.warn(&format!(
                        "could not inspect checked-out worktrees: {error:#}\n  the push succeeded; local branches are unchanged"
                    ));
                    return;
                }
            };
            worktrees
                .lines()
                .filter_map(|l| {
                    l.trim()
                        .strip_prefix("branch refs/heads/")
                        .map(str::to_owned)
                })
                .collect()
        };
        let branches = match self.commands.git(&[
            "for-each-ref",
            "--format=%(refname:strip=2)|%(objectname)",
            "refs/heads/",
        ]) {
            Ok(output) => output,
            Err(error) => {
                self.commands.warn(&format!(
                    "could not inspect local refs: {error:#}; local branches are unchanged"
                ));
                return;
            }
        };
        let branches: std::collections::HashMap<_, _> = branches
            .lines()
            .filter_map(|l| l.rsplit_once('|'))
            .collect();
        let bookmarks = if self.repo.jj {
            match jj::bookmarks(self.commands) {
                Ok(bookmarks) => bookmarks,
                Err(error) => {
                    self.commands.warn(&format!(
                        "could not inspect Jujutsu bookmarks: {error:#}; local refs are unchanged"
                    ));
                    return;
                }
            }
        } else {
            std::collections::HashMap::new()
        };
        let mut moved = false;
        for commit in commits
            .iter()
            .filter(|c| !c.skip && !c.remote_ref().is_empty())
        {
            if self.repo.jj {
                moved |= self.sync_bookmark(commit, &bookmarks, &branches);
                continue;
            }
            self.sync_local_branch(commit, &branches, &checked_out);
        }
        if moved {
            match self.commands.jj(&["git", "export"]) {
                Ok(output) if output.contains("Failed to export") => self.commands.warn(&format!(
                    "jj reported incomplete bookmark export; the push succeeded, but some local Git refs may be stale\n{output}"
                )),
                Err(e) => self.commands.warn(&format!(
                    "jj could not export every bookmark to a git ref: {e:#}"
                )),
                Ok(_) => {}
            }
        }
    }

    fn sync_local_branch(
        &self,
        commit: &Commit,
        branches: &std::collections::HashMap<&str, &str>,
        checked_out: &HashSet<String>,
    ) {
        let name = commit
            .publication
            .as_ref()
            .and_then(|binding| binding.anchor.as_deref())
            .unwrap_or_else(|| commit.remote_ref());
        let Some(&local) = branches.get(name) else {
            return;
        };
        let ancestor = match if local == commit.hash {
            Ok(false)
        } else {
            self.is_ancestor(local, &commit.hash)
        } {
            Ok(ancestor) => ancestor,
            Err(error) => {
                self.commands.warn(&format!(
                    "{error:#}; the push succeeded, but local branch {name} is unchanged"
                ));
                return;
            }
        };
        match selection::decide_ref_sync(local, &commit.hash, ancestor, false) {
            RefSync::Nothing => return,
            RefSync::Unrelated => {
                self.commands.warn(&format!("local branch {name} points at {}, which is not an ancestor of the pushed {};\n  leaving it alone (a later non-forced push of it will be rejected)", short_hash(local), commit.short_hash()));
                return;
            }
            RefSync::Move => {}
        }
        if checked_out.contains(name) {
            self.commands.warn(&format!(
                "local branch {name} is checked out at {}, not the pushed {};\n  leaving it alone",
                short_hash(local),
                commit.short_hash()
            ));
            return;
        }
        if let Err(error) = self.commands.git(&[
            "update-ref",
            &format!("refs/heads/{name}"),
            &commit.hash,
            local,
        ]) {
            self.commands.warn(&format!("could not move local branch {name} onto the pushed commit {}: {error:#}\n  the push succeeded; the local branch is stale", commit.short_hash()));
        }
    }

    fn sync_bookmark(
        &self,
        commit: &Commit,
        bookmarks: &std::collections::HashMap<String, jj::Bookmark>,
        branches: &std::collections::HashMap<&str, &str>,
    ) -> bool {
        let name = commit
            .publication
            .as_ref()
            .and_then(|binding| binding.anchor.as_deref())
            .unwrap_or_else(|| commit.remote_ref());
        let Some(bookmark) = bookmarks.get(name) else {
            return false;
        };
        if bookmark.conflicted || bookmark.targets.len() > 1 {
            self.commands.warn(&format!("jj bookmark {name} is conflicted; leaving it alone — resolve it with `jj bookmark set {name} -r <rev>`"));
            return false;
        }
        let Some((local, change)) = bookmark.targets.first() else {
            return false;
        };
        if *local == commit.hash {
            return branches.get(name).is_some_and(|oid| **oid != commit.hash);
        }
        let same_change = !change.is_empty() && *change == commit.change_id;
        let ancestor = if same_change {
            false
        } else {
            match self.is_ancestor(local, &commit.hash) {
                Ok(ancestor) => ancestor,
                Err(error) => {
                    self.commands.warn(&format!(
                        "{error:#}; the push succeeded, but jj bookmark {name} is unchanged"
                    ));
                    return false;
                }
            }
        };
        if selection::decide_ref_sync(local, &commit.hash, ancestor, same_change)
            == RefSync::Unrelated
        {
            self.commands.warn(&format!("jj bookmark {name} points at {}, a different change than the pushed {};\n  leaving it alone (a later non-forced push of it will be rejected)", short_hash(local), commit.short_hash()));
            return false;
        }
        if let Err(error) = self.commands.jj(&[
            "bookmark",
            "set",
            name,
            "-r",
            &commit.hash,
            "--allow-backwards",
        ]) {
            self.commands.warn(&format!("could not move jj bookmark {name} onto the pushed commit {}: {error:#}\n  the push succeeded; the local bookmark is stale", commit.short_hash()));
            return false;
        }
        true
    }
}
