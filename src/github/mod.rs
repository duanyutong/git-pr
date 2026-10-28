#[cfg(test)]
#[path = "../tests/github_helpers.rs"]
mod test_helpers;

mod content;
mod dispatch;
pub(crate) mod http;
pub(crate) mod limits;
mod native;
mod observation;
pub(crate) mod policy;
mod recovery;

use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub(crate) struct Session {
    prs: Mutex<BTreeMap<u64, Arc<PullRequest>>>,
    heads: Mutex<BTreeMap<String, Vec<u64>>>,
    commit_prs: Mutex<BTreeMap<String, (Instant, observation::Associations)>>,
    dispatch: Mutex<limits::Schedule>,
    content_batch: std::sync::atomic::AtomicUsize,
    updated: Mutex<HashSet<u64>>,
    pr_times: Mutex<BTreeMap<u64, Instant>>,
    head_times: Mutex<BTreeMap<String, Instant>>,
    stack_field_unsupported: AtomicBool,
    unknown_stacks: Mutex<HashSet<u64>>,
}

impl Session {
    // Cache entries are inserted only after complete decoding. Unwinding leaves each
    // map structurally valid; the worker barrier propagates the original failure.
    fn prs(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Arc<PullRequest>>> {
        guard(&self.prs)
    }
    fn heads(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u64>>> {
        guard(&self.heads)
    }
    fn dispatch(&self) -> std::sync::MutexGuard<'_, limits::Schedule> {
        guard(&self.dispatch)
    }
}
fn guard<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

use policy::select_pr;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    commands::Commands,
    config::Config,
    github::http::{Response, Transport},
    model::{Commit, PullRequest},
};

#[derive(Debug)]
pub(crate) struct GitHub<'a> {
    pub config: &'a Config,
    pub session: Session,
    pub clock: Option<&'a dyn crate::commands::Clock>,
    pub commands: &'a Commands,
    pub transport: &'a dyn Transport,
    pub template: &'a str,
}

impl GitHub<'_> {
    fn fresh(&self, observed: &Instant) -> bool {
        self.now().saturating_duration_since(*observed) <= Duration::from_secs(30)
    }

    pub(crate) fn was_updated(&self, number: u64) -> bool {
        guard(&self.session.updated).contains(&number)
    }

    fn remember_update(&self, pr: Arc<PullRequest>) {
        guard(&self.session.updated).insert(pr.number);
        self.remember(pr);
    }
    pub(crate) fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Vec<u8>> {
        let url = self.repo_url(path);
        self.request_url(method, &url, body)
            .map(|response| response.body)
    }

    fn repo_url(&self, path: &str) -> String {
        format!(
            "https://api.{}/repos/{}/{path}",
            self.config.repo.host, self.config.repo.name
        )
    }

    fn request_url(&self, method: &str, url: &str, body: Option<&Value>) -> Result<Response> {
        let mutation = method != "GET"
            && (!url.ends_with("/graphql")
                || body
                    .and_then(|b| b["query"].as_str())
                    .is_some_and(|q| q.starts_with("mutation")));
        let encoded = body.map(|value| value.to_string().into_bytes());
        self.request_url_encoded(
            method,
            url,
            encoded.as_deref(),
            usize::from(mutation),
            false,
        )
    }

    pub(crate) fn refresh_observations(&self, references: &[String]) -> Result<()> {
        // Refresh only known, expired heads. Discovery owns absent observations.
        let expired: Vec<_> = {
            let times = guard(&self.session.head_times);
            references
                .iter()
                .filter(|reference| times.get(*reference).is_some_and(|time| !self.fresh(time)))
                .cloned()
                .collect()
        };
        self.observe(&expired)
    }

    /// Publication can change PR state even when the previous observation is young.
    pub(crate) fn invalidate_heads(&self, references: &[String]) {
        let mut heads = self.session.heads();
        let mut times = guard(&self.session.head_times);
        let mut pr_times = guard(&self.session.pr_times);
        for reference in references {
            if let Some(numbers) = heads.remove(reference) {
                for number in numbers {
                    pr_times.remove(&number);
                }
            }
            times.remove(reference);
        }
        drop(pr_times);
        drop(times);
        drop(heads);
    }

    pub(crate) fn publication_pr(&self, reference: &str) -> Result<Option<Arc<PullRequest>>> {
        let heads = self.session.heads();
        let numbers = heads
            .get(reference)
            .context("missing publication observation")?;
        let prs = self.session.prs();
        let mut open = numbers
            .iter()
            .filter_map(|n| prs.get(n))
            .filter(|pr| pr.state == "open");
        let selected = open.next();
        if open.next().is_some() {
            bail!("multiple open PRs use head {reference:?}; refusing ambiguous publication");
        }
        let selected = selected.or_else(|| numbers.first().and_then(|n| prs.get(n)));
        let selected = selected.cloned();
        drop(prs);
        drop(heads);
        if let Some(pr) = &selected {
            policy::require_open(pr)?;
        }
        Ok(selected)
    }

    pub(crate) fn get(&self, number: u64) -> Result<Arc<PullRequest>> {
        let mut prs = self.session.prs();
        if let Some(pr) = prs.get(&number)
            && guard(&self.session.pr_times)
                .get(&number)
                .is_some_and(|time| self.fresh(time))
        {
            return Ok(Arc::clone(pr));
        }
        let pr = observation::decode_rest(
            &self.request("GET", &format!("pulls/{number}"), None)?,
            &self.config.repo.name,
            None,
        )
        .with_context(|| format!("invalid GitHub response for PR #{number}"))?;
        if pr.number != number {
            bail!("PR observation returned an unexpected identity");
        }
        let pr = Arc::new(pr);
        prs.insert(number, Arc::clone(&pr));
        guard(&self.session.pr_times).insert(number, self.now());
        guard(&self.session.unknown_stacks).remove(&number);
        drop(prs);
        Ok(pr)
    }

    fn remember(&self, pr: Arc<PullRequest>) {
        let number = pr.number;
        self.session.prs().insert(number, pr);
        // A partial mutation does not renew the age of unrelated observed fields.
        guard(&self.session.pr_times)
            .entry(number)
            .or_insert_with(|| self.now());
    }

    fn observed(&self, pr: PullRequest) {
        let number = pr.number;
        self.remember(Arc::new(pr));
        guard(&self.session.pr_times).insert(number, self.now());
        guard(&self.session.unknown_stacks).remove(&number);
    }

    fn get_with_stack(&self, number: u64) -> Result<Arc<PullRequest>> {
        if guard(&self.session.unknown_stacks).contains(&number) {
            guard(&self.session.pr_times).remove(&number);
        }
        self.get(number)
    }

    pub(crate) fn find_head(&self, reference: &str, state: &str) -> Result<u64> {
        if reference.is_empty() {
            return Ok(0);
        }
        if let Some(number) = self.cached_head(reference, state) {
            return Ok(number);
        }
        self.observe(&[reference.to_owned()])?;
        Ok(self.observed_head(reference, state))
    }

    pub(crate) fn observed_head(&self, reference: &str, state: &str) -> u64 {
        let heads = self.session.heads();
        let prs = self.session.prs();
        let number = select_pr(
            heads
                .get(reference)
                .into_iter()
                .flatten()
                .filter_map(|n| prs.get(n).map(Arc::as_ref))
                .filter(|p| state == "all" || p.state == "open"),
            reference,
        );
        drop(prs);
        drop(heads);
        number
    }

    fn cached_head(&self, reference: &str, state: &str) -> Option<u64> {
        let heads = self.session.heads();
        let numbers = heads.get(reference)?;
        if !guard(&self.session.head_times)
            .get(reference)
            .is_some_and(|time| self.fresh(time))
        {
            return None;
        }
        let prs = self.session.prs();
        let number = select_pr(
            numbers
                .iter()
                .filter_map(|n| prs.get(n).map(Arc::as_ref))
                .filter(|p| state == "all" || p.state == "open"),
            reference,
        );
        drop(prs);
        drop(heads);
        Some(number)
    }

    pub(crate) fn cached_find(&self, commit: &Commit) -> Option<u64> {
        if commit.pr_number != 0 {
            Some(commit.pr_number)
        } else {
            self.cached_head(commit.remote_ref(), "open")
        }
    }

    pub(crate) fn base_is_current(&self, commit: &Commit, base: &str) -> bool {
        let prs = self.session.prs();
        prs.get(&commit.pr_number).is_some_and(|pr| {
            guard(&self.session.pr_times)
                .get(&commit.pr_number)
                .is_some_and(|time| self.fresh(time))
                && pr.state == "open"
                && !pr.is_merged()
                && pr.base.name == base
        })
    }

    pub(crate) fn find(&self, commit: &Commit) -> Result<u64> {
        if commit.pr_number != 0 {
            return Ok(commit.pr_number);
        }
        if !commit.remote_ref().is_empty() {
            return self.find_head(commit.remote_ref(), "open");
        }
        let prs = self.commit_prs(&commit.hash)?;
        let mut matching = prs.iter().filter(|p| p.state == "open");
        match (matching.next(), matching.next()) {
            (Some(pr), None) => Ok(pr.number),
            (None, _) => Ok(0),
            _ => bail!(
                "multiple open PRs contain commit {}; specify its Remote-Ref",
                commit.short_hash()
            ),
        }
    }

    pub(crate) fn lookup(&self, commit: &Commit) -> Result<u64> {
        if commit.pr_number != 0 {
            return Ok(commit.pr_number);
        }
        if commit.remote_ref().is_empty() {
            return Ok(0);
        }
        let number = self.find_head(commit.remote_ref(), "open")?;
        if number != 0 {
            return Ok(number);
        }
        let prs = self.commit_prs(&commit.hash)?;
        Ok(prs
            .iter()
            .find(|p| p.head == commit.remote_ref())
            .map_or(0, |p| p.number))
    }

    pub(crate) fn create(&self, commit: &mut Commit, base: &str) -> Result<()> {
        let body = crate::stack::description::initial_body(&commit.message, self.template)
            .with_context(|| format!("failed to prepare PR creation for {:?}", commit.title))?;
        let draft = policy::is_draft(
            &commit.title,
            self.config.draft,
            &self.config.draft_patterns,
        );
        self.commands.detail(&format!(
            "create {}pull request for {:?}",
            if draft { "draft " } else { "" },
            commit.title
        ));
        let payload = json!({"title":commit.title,"body":body,"head":commit.remote_ref(),"base":base,"draft":draft});
        let effect = crate::model::outcome::Operation::Creation(commit.remote_ref().into());
        self.commands.explain(&format!(
            "Creating PR for head {:?} with base {base:?}.",
            commit.remote_ref()
        ));
        self.commands.outcomes.start(effect.clone());
        let response = self.request("POST", "pulls", Some(&payload));
        let bytes = match response {
            Ok(bytes) => bytes,
            Err(error) => {
                // Creation can have completed before a response was lost. Observe before retrying.
                self.session.heads().remove(commit.remote_ref());
                let number = self.find_head(commit.remote_ref(), "open").with_context(|| format!(
                    "PR creation failed and its result could not be verified; a PR may already exist. Original creation failure: {error:#}"
                ))?;
                if number == 0 {
                    return Err(error);
                }
                commit.pr_number = number;
                self.commands.outcomes.confirm(effect);
                return Ok(());
            }
        };
        let pr = observation::decode_rest(
            &bytes,
            &self.config.repo.name,
            Some((commit.remote_ref(), base)),
        )
        .with_context(|| format!("invalid PR creation response for head {:?} and base {base:?}; a PR may already exist. Verify the remote PR before retrying submission", commit.remote_ref()))?;
        commit.pr_number = pr.number;
        commit.newly_created = true;
        self.commands.outcomes.confirm(effect);
        self.session
            .heads()
            .entry(commit.remote_ref().into())
            .or_default()
            .push(pr.number);
        self.observed(pr);
        guard(&self.session.head_times).insert(commit.remote_ref().into(), self.now());
        Ok(())
    }

    pub(crate) fn refresh_prs(&self, numbers: &[u64]) -> Result<()> {
        #[cfg(test)]
        crate::tests::unit::support::PR_REFRESH_PROBES.update(|n| n + numbers.len());
        let references: Vec<_> = {
            let prs = self.session.prs();
            let times = guard(&self.session.pr_times);
            numbers
                .iter()
                .filter(|number| !times.get(number).is_some_and(|time| self.fresh(time)))
                .filter_map(|number| prs.get(number))
                .map(|pr| pr.head.name.clone())
                .collect()
        };
        if references.is_empty() {
            return Ok(());
        }
        // A recently observed head can still contain an older PR observation.
        // Invalidate the head independently, then reuse the complete batched reader.
        {
            let mut heads = self.session.heads();
            for reference in &references {
                heads.remove(reference);
            }
        }
        self.observe(&references)
    }

    pub(crate) fn update_labels(&self, changes: &[(&Commit, bool)]) -> Result<()> {
        let mut progress = self.commands.progress("Labels", changes.len());
        for chunk in changes.chunks(25) {
            for (index, &(commit, tip)) in chunk.iter().enumerate() {
                progress.targets(std::iter::once(commit.pr_number));
                let (required, removed) = policy::label_changes(
                    commit,
                    &self.config.tags,
                    &self.config.labels,
                    &self.config.tip_labels,
                    tip,
                );
                if required.is_empty() && removed.is_empty() {
                    progress.advance(1);
                    continue;
                }
                let number = commit.pr_number;
                #[cfg(test)]
                crate::tests::unit::support::LABEL_FRESHNESS_CHECKS.update(|n| n + 1);
                let current = guard(&self.session.pr_times)
                    .get(&number)
                    .is_some_and(|time| self.fresh(time));
                // Slow preceding mutations can expire the next target; refresh its
                // remaining bounded group only when that target requires observation.
                if !current {
                    self.refresh_prs(
                        &chunk[index..]
                            .iter()
                            .map(|(c, _)| c.pr_number)
                            .collect::<Vec<_>>(),
                    )?;
                }
                let mut pr = self.get(number)?;
                let observed: std::collections::HashMap<_, _> = pr
                    .labels
                    .iter()
                    .map(|label| (label.name().to_ascii_lowercase(), label.name()))
                    .collect();
                let additions: Vec<_> = required
                    .into_iter()
                    .filter(|label| !observed.contains_key(&label.to_ascii_lowercase()))
                    .collect();
                let removals: Vec<_> = removed
                    .into_iter()
                    .filter_map(|label| {
                        observed
                            .get(&label.to_ascii_lowercase())
                            .map(|name| (*name).to_owned())
                    })
                    .collect();
                if !additions.is_empty() {
                    let effect = crate::model::outcome::Operation::Labels(number);
                    self.commands.outcomes.start(effect.clone());
                    self.request(
                        "POST",
                        &format!("issues/{number}/labels"),
                        Some(&json!({"labels":additions})),
                    )?;
                    self.commands.outcomes.confirm(effect);
                    Arc::make_mut(&mut pr).labels.extend(
                        additions
                            .into_iter()
                            .map(crate::model::pull_request::Label::Name),
                    );
                    self.remember_update(pr.clone());
                }
                for label in removals {
                    let encoded = percent_encoding::utf8_percent_encode(
                        &label,
                        percent_encoding::NON_ALPHANUMERIC,
                    );
                    let effect = crate::model::outcome::Operation::Labels(number);
                    self.commands.outcomes.start(effect.clone());
                    self.request("DELETE", &format!("issues/{number}/labels/{encoded}"), None)?;
                    self.commands.outcomes.confirm(effect);
                    Arc::make_mut(&mut pr)
                        .labels
                        .retain(|value| !value.name().eq_ignore_ascii_case(&label));
                    self.remember_update(pr.clone());
                }
                progress.advance(1);
            }
        }
        Ok(())
    }

    pub(crate) fn verify_bases(&self, changes: Vec<(&Commit, &str)>) -> Result<()> {
        let mut progress = self.commands.progress("Bases", changes.len());
        let mut remaining = changes.into_iter();
        loop {
            let chunk: Vec<_> = remaining.by_ref().take(25).collect();
            if chunk.is_empty() {
                return Ok(());
            }
            progress.targets(chunk.iter().map(|(commit, _)| commit.pr_number));
            self.refresh_prs(&chunk.iter().map(|(c, _)| c.pr_number).collect::<Vec<_>>())?;
            let verified = crate::commands::parallel_cached(
                chunk,
                |(commit, base)| self.base_is_current(commit, base).then_some(()),
                |(commit, base)| self.verify_base(commit, base),
            )?;
            progress.advance(verified.len());
        }
    }

    pub(crate) fn verify_base(&self, commit: &Commit, base: &str) -> Result<()> {
        if self.base_is_current(commit, base) {
            return Ok(());
        }
        policy::require_publication_base(self.get(commit.pr_number)?.as_ref(), base)
    }

    fn confirm(&self, prompt: &str) -> bool {
        self.config.auto_accept || self.commands.terminal.confirm(prompt)
    }

    pub(crate) fn warn_blocked(&self, commits: &[Commit]) {
        let blocked: Vec<_> = commits
            .iter()
            .filter(|c| c.base_blocked && !c.skip)
            .collect();
        if blocked.is_empty() {
            return;
        }
        let numbers = blocked
            .iter()
            .filter(|c| c.pr_number != 0)
            .map(|c| format!("#{}", c.pr_number))
            .collect::<Vec<_>>()
            .join(", ");
        let numbers = if numbers.is_empty() {
            "(unknown)"
        } else {
            &numbers
        };
        self.commands.warn(&format!("PR(s) {numbers} are in a GitHub Stack and could not be retargeted; their base is stale.\nFix with `gh stack modify`, or let git-pr rebuild the stack (accept the prompt / drop --no-stack)."));
    }
}

#[cfg(test)]
#[path = "../tests/unit/github_protocol.rs"]
mod unit;

const CONTENT_LIMIT: usize = 1024 * 1024;
fn ensure_content_size(payload: &[u8]) -> Result<()> {
    if payload.len() > CONTENT_LIMIT {
        bail!("PR content exceeds the 1 MiB request budget");
    }
    Ok(())
}
