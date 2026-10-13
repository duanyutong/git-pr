//! Bounded content publication with observation before uncertain writes are retried.

use super::{CONTENT_LIMIT, GitHub, ensure_content_size, limits, policy};
use crate::model::{Commit, PullRequest};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    sync::{Arc, atomic::Ordering},
};

#[derive(Debug)]
struct Rejected;
impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GitHub rejected the content update; correct the reported permission or validation error before retrying")
    }
}
impl std::error::Error for Rejected {}

struct PreparedContent {
    observed: Arc<PullRequest>,
    body: Arc<str>,
    input: Vec<u8>,
    size: usize,
}

impl GitHub<'_> {
    fn content_batch(&self) -> usize {
        match self.session.content_batch.load(Ordering::Relaxed) {
            0 => limits::CONTENT_BATCH,
            batch => batch,
        }
    }

    fn recover_content(
        &self,
        mut changes: Vec<Change<'_>>,
        render: &mut dyn FnMut(&Commit, &PullRequest) -> Result<Option<String>>,
        mut error: anyhow::Error,
    ) -> Result<()> {
        let numbers = changes
            .iter()
            .map(|c| format!("#{}", c.before.number))
            .collect::<Vec<_>>()
            .join(", ");
        let _activity = self.commands.activity(
            &format!("Checking uncertain description updates: {numbers}"),
            None,
        );
        let mut attempt = 0;
        loop {
            if let Some(status) = error.downcast_ref::<crate::github::http::HttpStatusError>()
                && !matches!(status.status.as_u16(), 408 | 500..=599)
                && !error.is::<limits::Throttled>()
            {
                return Err(error).with_context(|| {
                    format!(
                        "PR content update failed for {numbers}; no unsafe replay was attempted"
                    )
                });
            }
            if error.is::<limits::Exhausted>() || error.is::<Rejected>() {
                return Err(error).with_context(|| {
                    format!("PR content updates remain unconfirmed for {numbers}")
                });
            }
            changes.retain(|change| !change.acknowledged);
            if changes.is_empty() {
                return Ok(());
            }
            let deferred = error.downcast_ref::<limits::RefreshAfterWait>();
            if let Some(deferred) = deferred {
                self.wait_for(deferred.delay, deferred.reason.label())?;
            }
            if let Some(throttled) = error.downcast_ref::<limits::Throttled>() {
                let delay = if throttled.backoff {
                    limits::backoff(attempt, true)
                } else {
                    throttled.delay
                };
                self.session.dispatch().throttle(self.now(), delay);
            }
            self.refresh_content(&mut changes, render).with_context(|| format!("could not establish the outcome of PR content updates for {numbers}; automatic recovery stopped"))?;
            if changes.is_empty() {
                return Ok(());
            }
            let outstanding = changes
                .iter()
                .map(|c| format!("#{}", c.before.number))
                .collect::<Vec<_>>()
                .join(", ");
            if attempt == limits::RETRIES {
                return Err(error).with_context(|| format!("PR content recovery exhausted after {} attempt(s); outstanding PRs: {outstanding}. Completed changes are retained; inspect these descriptions before retrying", attempt + 1));
            }
            let batch = self.content_batch();
            let batch = if deferred.is_some() {
                batch
            } else {
                (batch / 2).max(1)
            };
            self.session.content_batch.store(batch, Ordering::Relaxed);
            if deferred.is_none() && !error.is::<limits::Throttled>() {
                self.wait_for(
                    limits::backoff(attempt, false),
                    &format!(
                        "Recovering descriptions for {outstanding}; retry {}/{}",
                        attempt + 1,
                        limits::RETRIES
                    ),
                )?;
            }
            let mut failure = None;
            for chunk in changes.chunks_mut(batch) {
                if let Err(cause) = self.send_recovery(chunk) {
                    failure = Some(cause);
                    break;
                }
            }
            match failure {
                Some(cause) => error = cause,
                None => return Ok(()),
            }
            attempt += 1;
        }
    }

    fn refresh_content(
        &self,
        changes: &mut Vec<Change<'_>>,
        render: &mut dyn FnMut(&Commit, &PullRequest) -> Result<Option<String>>,
    ) -> Result<()> {
        let heads = changes
            .iter()
            .map(|c| c.before.head.name.clone())
            .collect::<Vec<_>>();
        self.invalidate_heads(&heads);
        self.observe(&heads)?;
        for change in changes.iter_mut() {
            let current = self
                .publication_pr(&change.before.head.name)?
                .context("PR disappeared during content recovery")?;
            if current.number != change.before.number
                || current.node_id != change.before.node_id
                || current.base.name != change.before.base.name
            {
                bail!(
                    "PR #{} identity or base changed during content recovery; refusing to update a different relationship",
                    change.before.number
                );
            }
            if current.title != change.before.title && current.title != change.after.title {
                bail!(
                    "PR #{} title changed during content recovery; preserving the concurrent edit",
                    current.number
                );
            }
            let body = render(change.commit, &current)
                .with_context(|| {
                    format!(
                        "failed to prepare PR #{} description during recovery",
                        current.number
                    )
                })?
                .map_or_else(|| Arc::clone(&current.body), Arc::from);
            let mut desired = Arc::clone(&current);
            let pr = Arc::make_mut(&mut desired);
            pr.title.clone_from(&change.commit.title);
            pr.body = body;
            if current.title == desired.title && current.body == desired.body {
                self.remember_update(Arc::clone(&current));
            }
            change.before = current;
            change.after = desired;
        }
        changes.retain(|change| {
            change.before.title != change.after.title || change.before.body != change.after.body
        });
        Ok(())
    }

    fn send_recovery(&self, changes: &mut [Change<'_>]) -> Result<()> {
        use std::fmt::Write as _;
        let mut declarations = Vec::new();
        let mut fields = String::new();
        let mut variables = Vec::new();
        let mut bytes = 256;
        for change in changes.iter() {
            let pr = &change.after;
            let alias = change.alias;
            let input = json!({"pullRequestId":pr.node_id,"title":pr.title,"body":pr.body})
                .to_string()
                .into_bytes();
            bytes += input.len() + 256;
            if bytes > CONTENT_LIMIT {
                return Err(limits::Exhausted(
                    "recovered PR content exceeds the 1 MiB request budget".into(),
                )
                .into());
            }
            declarations.push(format!("$p{alias}:UpdatePullRequestInput!"));
            let _ = write!(
                fields,
                "p{alias}:updatePullRequest(input:$p{alias}){{pullRequest{{number}}}} "
            );
            variables.push((alias, input));
        }
        self.dispatch_content(&declarations, &fields, variables, changes)
    }
}

#[derive(Debug)]
struct Change<'a> {
    alias: usize,
    commit: &'a Commit,
    before: Arc<PullRequest>,
    after: Arc<PullRequest>,
    acknowledged: bool,
}

impl GitHub<'_> {
    pub(crate) fn update_content(
        &self,
        changes: &[&Commit],
        render: &mut dyn FnMut(&Commit, &PullRequest) -> Result<Option<String>>,
    ) -> Result<()> {
        let mut pending = changes;
        let mut prefetched = 0;
        let mut progress = self.commands.progress("Descriptions", changes.len());
        let mut deferred: Option<PreparedContent> = None;
        while !pending.is_empty() {
            progress.targets(pending.iter().take(25).map(|commit| commit.pr_number));
            let batch_size = self.content_batch();
            if prefetched == 0
                || !super::guard(&self.session.pr_times)
                    .get(&pending[0].pr_number)
                    .is_some_and(super::fresh)
            {
                prefetched = pending.len().min(25);
                self.refresh_prs(
                    &pending[..prefetched]
                        .iter()
                        .map(|c| c.pr_number)
                        .collect::<Vec<_>>(),
                )?;
            }
            let mut variables = Vec::new();
            let mut declarations = Vec::new();
            let mut fields = String::new();
            let mut observations = Vec::new();
            let mut consumed = 0;
            let mut payload_bytes = 256;
            for (i, commit) in pending.iter().take(prefetched).enumerate() {
                if observations.len() == batch_size {
                    break;
                }
                let number = commit.pr_number;
                progress.targets(std::iter::once(number));
                let title = &commit.title;
                let mut pr = self.get(number)?;
                let before = Arc::clone(&pr);
                policy::require_open(&pr)?;
                // Render from this observation: expiry may have acquired newer human text.
                let (body, prepared) = Self::content_body(commit, &pr, deferred.take(), render)?;
                if pr.title == *title && (Arc::ptr_eq(&pr.body, &body) || pr.body == body) {
                    consumed += 1;
                    progress.advance(1);
                    continue;
                }
                if pr.node_id.is_empty() {
                    self.update_rest_content(commit, before, body, render)?;
                    consumed += 1;
                    progress.advance(1);
                    continue;
                }
                // Reserve bounded query overhead as well as JSON-escaped UTF-8 payload bytes.
                let (input, size) = Self::encode_content(&pr.node_id, title, &body, prepared);
                if payload_bytes + size > CONTENT_LIMIT {
                    if observations.is_empty() {
                        bail!("PR content exceeds the 1 MiB request budget");
                    }
                    deferred = Some(PreparedContent {
                        observed: pr,
                        body,
                        input,
                        size,
                    });
                    break;
                }
                let metadata = Arc::make_mut(&mut pr);
                metadata.title.clone_from(title);
                metadata.body = body;
                payload_bytes += size;
                consumed += 1;
                declarations.push(format!("$p{i}:UpdatePullRequestInput!"));
                variables.push((i, input));
                let _ = write!(
                    fields,
                    "p{i}:updatePullRequest(input:$p{i}){{pullRequest{{number}}}} "
                );
                observations.push(Change {
                    alias: i,
                    commit,
                    before,
                    after: pr,
                    acknowledged: false,
                });
            }
            pending = &pending[consumed..];
            prefetched -= consumed;
            if observations.is_empty() {
                continue;
            }
            progress.targets(observations.iter().map(|change| change.before.number));
            let dispatched = observations.len();
            if let Err(error) =
                self.dispatch_content(&declarations, &fields, variables, &mut observations)
            {
                self.recover_content(observations, render, error)?;
            }
            progress.advance(dispatched);
        }
        Ok(())
    }

    fn encode_content(
        node_id: &str,
        title: &str,
        body: &str,
        prepared: Option<PreparedContent>,
    ) -> (Vec<u8>, usize) {
        prepared.map_or_else(
            || {
                #[cfg(test)]
                crate::tests::unit::support::CONTENT_ENCODINGS.update(|n| n + 1);
                let input = json!({"pullRequestId":node_id,"title":title,"body":body})
                    .to_string()
                    .into_bytes();
                let size = input.len() + 256;
                (input, size)
            },
            |p| (p.input, p.size),
        )
    }

    fn content_body(
        commit: &Commit,
        pr: &Arc<PullRequest>,
        deferred: Option<PreparedContent>,
        render: &mut dyn FnMut(&Commit, &PullRequest) -> Result<Option<String>>,
    ) -> Result<(Arc<str>, Option<PreparedContent>)> {
        match deferred {
            Some(prepared) if Arc::ptr_eq(&prepared.observed, pr) || prepared.observed == *pr => {
                Ok((Arc::clone(&prepared.body), Some(prepared)))
            }
            _ => Ok((
                render(commit, pr)
                    .with_context(|| format!("failed to prepare PR #{} description", pr.number))?
                    .map_or_else(|| Arc::clone(&pr.body), Arc::from),
                None,
            )),
        }
    }

    fn update_rest_content(
        &self,
        commit: &Commit,
        before: Arc<PullRequest>,
        body: Arc<str>,
        render: &mut dyn FnMut(&Commit, &PullRequest) -> Result<Option<String>>,
    ) -> Result<()> {
        let encoded = json!({"title":commit.title,"body":body.as_ref()})
            .to_string()
            .into_bytes();
        #[cfg(test)]
        crate::tests::unit::support::CONTENT_ENCODINGS.update(|n| n + 1);
        ensure_content_size(&encoded)?;
        let result = self.request_url_encoded(
            "PATCH",
            &self.repo_url(&format!("pulls/{}", before.number)),
            Some(&encoded),
            1,
            true,
        );
        let mut after = Arc::clone(&before);
        let metadata = Arc::make_mut(&mut after);
        metadata.title.clone_from(&commit.title);
        metadata.body = body;
        if let Err(error) = result {
            self.recover_content(
                vec![Change {
                    alias: 0,
                    commit,
                    before,
                    after,
                    acknowledged: false,
                }],
                render,
                error,
            )
        } else {
            self.remember_update(after);
            Ok(())
        }
    }

    fn dispatch_content(
        &self,
        declarations: &[String],
        fields: &str,
        variables: Vec<(usize, Vec<u8>)>,
        observations: &mut [Change<'_>],
    ) -> Result<()> {
        let query = format!("mutation({}){{{fields}}}", declarations.join(","));
        let url = format!("https://api.{}/graphql", self.config.repo.host);
        let mut payload = br#"{"query":"#.to_vec();
        payload.extend_from_slice(serde_json::Value::String(query).to_string().as_bytes());
        payload.extend_from_slice(br#","variables":{"#);
        for (position, (alias, input)) in variables.into_iter().enumerate() {
            if position != 0 {
                payload.push(b',');
            }
            payload.extend_from_slice(format!(r#""p{alias}":"#).as_bytes());
            payload.extend_from_slice(&input);
        }
        payload.extend_from_slice(b"}}");
        // The initial 256-byte allowance and 256 bytes per alias exceed this
        // envelope's fixed ASCII overhead; input bytes were already bounded.
        let response = self
            .request_url_encoded("POST", &url, Some(&payload), observations.len(), true)
            .inspect_err(|error| {
                if let Some(partial) = error.downcast_ref::<super::dispatch::PartialGraphql>() {
                    self.acknowledge_content(&partial.data, observations);
                }
            })?;
        let response: Value = serde_json::from_slice(&response.body).context(
            "invalid GraphQL response to PR content updates; completed updates may remain",
        )?;
        let incomplete = self.acknowledge_content(&response["data"], observations);
        if incomplete || response.get("errors").is_some() {
            let error = anyhow::anyhow!("incomplete PR content update: {}", response["errors"]);
            if response["errors"].as_array().is_some_and(|errors| {
                errors.iter().any(|error| {
                    matches!(
                        error["type"].as_str(),
                        Some(
                            "FORBIDDEN"
                                | "NOT_FOUND"
                                | "UNPROCESSABLE"
                                | "UNPROCESSABLE_ENTITY"
                                | "BAD_USER_INPUT"
                                | "GRAPHQL_VALIDATION_FAILED"
                        )
                    )
                })
            }) {
                return Err(error.context(Rejected));
            }
            return Err(error);
        }
        Ok(())
    }

    fn acknowledge_content(&self, data: &Value, observations: &mut [Change<'_>]) -> bool {
        let mut incomplete = false;
        for change in observations {
            if data[format!("p{}", change.alias)]["pullRequest"]["number"] == change.after.number {
                change.acknowledged = true;
                self.remember_update(Arc::clone(&change.after));
            } else {
                incomplete = true;
            }
        }
        incomplete
    }
}

#[cfg(test)]
#[path = "../tests/unit/content_recovery.rs"]
mod unit;
