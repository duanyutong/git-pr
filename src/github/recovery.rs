//! Fresh observations and verified mutations for PR base transitions.

use anyhow::{Context, Result, bail};
use serde_json::json;
use std::{collections::HashSet, fmt::Write as _, time::Instant};

use super::{GitHub, guard, policy};
use crate::model::pull_request::RecoveryPullRequest;

pub(super) const HEAD_RECOVERY_FIELDS: &str =
    "mergedAt mergeCommit{oid} headRefOid baseRefOid baseRepository{nameWithOwner}";

const RECOVERY_FIELDS: &str = "number id state merged mergedAt mergeCommit{oid} headRefName headRefOid headRepository{nameWithOwner} baseRefName baseRefOid baseRepository{nameWithOwner} stack{number}";

#[derive(Debug)]
struct RetargetAcknowledgement {
    next: Option<RecoveryPullRequest>,
    observed: Instant,
}

impl GitHub<'_> {
    /// Acquire complete recovery evidence in one bounded query per group.
    /// These observations deliberately bypass the ordinary content cache.
    #[cfg(test)]
    pub(crate) fn recovery_prs(&self, numbers: &[u64]) -> Result<Vec<RecoveryPullRequest>> {
        self.recovery_prs_at(numbers, None).map(|(prs, _)| prs)
    }

    fn recovery_prs_at(
        &self,
        numbers: &[u64],
        started: Option<Instant>,
    ) -> Result<(Vec<RecoveryPullRequest>, Instant)> {
        let mut seen = HashSet::new();
        let numbers: Vec<_> = numbers
            .iter()
            .copied()
            .filter(|number| seen.insert(*number))
            .collect();
        let mut observed = Vec::new();
        let mut acquired = self.now();
        for chunk in numbers.chunks(25) {
            let (owner, name) = self
                .config
                .repo
                .name
                .split_once('/')
                .context("invalid repository identity")?;
            let mut declarations = String::from("$owner:String!,$name:String!");
            let mut fields = String::new();
            let mut variables = json!({"owner":owner,"name":name});
            for (i, number) in chunk.iter().enumerate() {
                let number = i32::try_from(*number)
                    .context("PR number exceeds the GraphQL integer range")?;
                let _ = write!(declarations, ",$r{i}:Int!");
                let _ = write!(
                    fields,
                    "p{i}:pullRequest(number:$r{i}){{{RECOVERY_FIELDS}}} "
                );
                variables[format!("r{i}")] = json!(number);
            }
            let response = self.request_url_with_budget(
                "POST", &format!("https://api.{}/graphql", self.config.repo.host),
                Some(&json!({"query":format!("query({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"),"variables":variables})),
                started,
            )?;
            acquired = if observed.is_empty() {
                response.acquired
            } else {
                acquired.min(response.acquired)
            };
            let mut response: serde_json::Value = serde_json::from_slice(&response.body)
                .context("incomplete base recovery observation")?;
            if let Some(errors) = response.get("errors") {
                bail!("failed to observe publication transition: {errors}");
            }
            let repository = response
                .get_mut("data")
                .and_then(|data| data.get_mut("repository"))
                .and_then(serde_json::Value::as_object_mut)
                .context("missing recovery repository observation")?;
            for (i, number) in chunk.iter().enumerate() {
                let node = repository
                    .remove(&format!("p{i}"))
                    .context("missing recovery PR observation")?;
                let pr = decode_recovery(node).with_context(|| {
                    format!("incomplete base recovery observation for PR #{number}")
                })?;
                policy::validate_recovery_observation(&pr, *number, &self.config.repo.name)?;
                observed.push(pr);
            }
        }
        Ok((observed, acquired))
    }

    pub(crate) fn recovery_pr(&self, number: u64) -> Result<RecoveryPullRequest> {
        self.recovery_pr_with_budget(number, None)
    }

    pub(crate) fn recovery_pr_with_budget(
        &self,
        number: u64,
        started: Option<Instant>,
    ) -> Result<RecoveryPullRequest> {
        let mut value: serde_json::Value = serde_json::from_slice(
            &self
                .request_url_with_budget(
                    "GET",
                    &self.repo_url(&format!("pulls/{number}")),
                    None,
                    started,
                )?
                .body,
        )
        .context("incomplete base recovery observation")?;
        if value.get("stack").is_none() {
            // REST may omit membership; obtain explicit, fresh GraphQL evidence.
            let reference = value["head"]["ref"]
                .as_str()
                .context("missing recovery head reference")?
                .to_owned();
            let references = [reference];
            self.invalidate_heads(&references);
            started.map_or_else(
                || self.observe(&references),
                |started| self.observe_with_budget(&references, started),
            )?;
            let observed = self
                .session
                .heads()
                .get(&references[0])
                .filter(|numbers| numbers.contains(&number))
                .and_then(|_| self.session.prs().get(&number).cloned())
                .context("missing recovery membership observation")?;
            if guard(&self.session.unknown_stacks).contains(&number)
                || value["node_id"] != observed.node_id
                || value["head"]["ref"] != observed.head.name
                || value["base"]["ref"] != observed.base.name
                || value["state"] != observed.state
                || value["merged"] != observed.merged
            {
                bail!(
                    "incomplete or inconsistent recovery membership observation for PR #{number}"
                );
            }
            value["stack"] = json!(observed.stack);
        }
        let pr = serde_json::from_value(value).context("incomplete base recovery observation")?;
        policy::validate_recovery_observation(&pr, number, &self.config.repo.name)?;
        Ok(pr)
    }

    pub(crate) fn recovery_parent(&self, reference: &str) -> Result<RecoveryPullRequest> {
        let references = [reference.to_owned()];
        self.invalidate_heads(&references);
        self.observe(&references)?;
        let numbers = self
            .session
            .heads()
            .get(reference)
            .cloned()
            .unwrap_or_default();
        let [number] = numbers.as_slice() else {
            bail!(
                "obsolete base {reference:?} has missing or ambiguous PR history; refusing automatic base recovery"
            );
        };
        self.recovery_pr(*number)
    }

    pub(crate) fn retarget_recovery(
        &self,
        expected: &RecoveryPullRequest,
        base: &str,
    ) -> Result<()> {
        self.retarget_recoveries(&[(expected.clone(), base.to_owned())])
    }

    /// Each acknowledgement also observes the next precondition, without dispatching its mutation.
    pub(crate) fn retarget_recoveries(
        &self,
        changes: &[(RecoveryPullRequest, String)],
    ) -> Result<()> {
        let mut prefetched = None;
        for (index, (expected, base)) in changes.iter().enumerate() {
            let next = changes.get(index + 1).map(|(pr, _)| pr.number);
            let acknowledgement = self.retarget_one(expected, base, next, prefetched.take())?;
            guard(&self.session.updated).insert(expected.number);
            self.commands
                .outcomes
                .confirm(crate::model::outcome::Operation::Base(expected.number));
            prefetched = acknowledgement
                .next
                .map(|pr| (pr, acknowledgement.observed));
        }
        Ok(())
    }

    fn retarget_one(
        &self,
        expected: &RecoveryPullRequest,
        base: &str,
        next: Option<u64>,
        mut prefetched: Option<(RecoveryPullRequest, Instant)>,
    ) -> Result<RetargetAcknowledgement> {
        let started = self.now();
        let mut deferrals = 0;
        loop {
            let (current, acquired) = match prefetched
                .take()
                .filter(|(_, observed)| self.fresh(observed))
            {
                Some(observation) => observation,
                None => self.recovery_head(expected.number, started)?,
            };
            policy::same_recovery_head(expected, &current, &expected.base.name)?;
            Self::require_recoverable_membership(&current)?;
            let effect = crate::model::outcome::Operation::Base(expected.number);
            self.commands.outcomes.start(effect.clone());
            let response = self.send_retarget(expected, base, next, acquired, started);
            if response.as_ref().is_err_and(super::limits::rejected) {
                self.commands.outcomes.reject(effect.clone());
            }
            self.invalidate_heads(std::slice::from_ref(&expected.head.name));
            match response {
                Ok(acknowledgement) => return Ok(acknowledgement),
                Err(error) => {
                    if let Some((delay, reason)) = super::limits::deferral(&error, deferrals) {
                        // Renew evidence after an unsent attempt or definite rejection.
                        if deferrals == super::limits::RETRIES {
                            return Err(error.context(super::limits::Exhausted("base update exceeded its automatic deferral budget; no deferred mutation was sent".into())));
                        }
                        self.wait_for_recovery(delay, reason.label(), started)?;
                        let (actual, acquired) = self.recovery_head(expected.number, started)?;
                        if !error.is::<super::limits::RefreshAfterWait>()
                            && policy::same_recovery_head(expected, &actual, base).is_ok()
                        {
                            return Ok(RetargetAcknowledgement {
                                next: None,
                                observed: acquired,
                            });
                        }
                        prefetched = Some((actual, acquired));
                        deferrals += 1;
                        continue;
                    }
                    if super::limits::rejected(&error) {
                        return Err(error);
                    }
                    let observed = self.recovery_head(expected.number, started)
                        .and_then(|(actual, acquired)| {
                            policy::same_recovery_head(expected, &actual, base)?;
                            Ok(acquired)
                        })
                        .with_context(|| format!("base recovery for PR #{} could not be confirmed; retain any completed PR changes and inspect before retrying; original base update failure: {error:#}", expected.number))?;
                    return Ok(RetargetAcknowledgement {
                        next: None,
                        observed,
                    });
                }
            }
        }
    }

    fn throttle_retarget(&self, error: &anyhow::Error) {
        if let Some(throttled) = error.downcast_ref::<super::limits::Throttled>() {
            self.session
                .dispatch()
                .throttle(self.now(), throttled.delay);
        }
    }

    fn send_retarget(
        &self,
        expected: &RecoveryPullRequest,
        base: &str,
        next: Option<u64>,
        acquired: Instant,
        started: Instant,
    ) -> Result<RetargetAcknowledgement> {
        let mut variables = json!({"input":{"pullRequestId":expected.node_id,"baseRefName":base}});
        let (declaration, lookahead) = if let Some(number) = next {
            variables["next"] = json!(
                i32::try_from(number).context("PR number exceeds the GraphQL integer range")?
            );
            (
                ",$next:Int!",
                format!("repository{{next:pullRequest(number:$next){{{RECOVERY_FIELDS}}}}}"),
            )
        } else {
            ("", String::new())
        };
        let body = json!({"query":format!("mutation Retarget($input:UpdatePullRequestInput!{declaration}){{retarget:updatePullRequest(input:$input){{pullRequest{{{RECOVERY_FIELDS} {lookahead}}}}}}}"),"variables":variables}).to_string();
        let response = self.request_url_encoded(
            "POST",
            &format!("https://api.{}/graphql", self.config.repo.host),
            Some(body.as_bytes()),
            1,
            Some(acquired),
            Some(started),
        );
        match response {
            Ok(response) => {
                let mut value: serde_json::Value = serde_json::from_slice(&response.body)
                    .context("incomplete base update acknowledgement")?;
                let partial = value.get("errors").is_some();
                let data = value
                    .get_mut("data")
                    .map(serde_json::Value::take)
                    .unwrap_or_default();
                self.decode_retarget(data, expected, base, next, partial)
                    .map(|next| RetargetAcknowledgement {
                        next,
                        observed: response.acquired,
                    })
                    .map_err(|error| match value.get("errors") {
                        Some(errors) => {
                            error.context(format!("base update returned GraphQL errors: {errors}"))
                        }
                        None => error,
                    })
            }
            Err(mut error) => {
                self.throttle_retarget(&error);
                if let Some(partial) = error.downcast_mut::<super::dispatch::PartialGraphql>() {
                    let data = partial.data.take();
                    if let Ok(next) = self.decode_retarget(data, expected, base, next, true) {
                        return Ok(RetargetAcknowledgement {
                            next,
                            observed: partial.acquired,
                        });
                    }
                }
                Err(error)
            }
        }
    }

    fn decode_retarget(
        &self,
        mut data: serde_json::Value,
        expected: &RecoveryPullRequest,
        base: &str,
        next: Option<u64>,
        partial: bool,
    ) -> Result<Option<RecoveryPullRequest>> {
        let mut node = data
            .get_mut("retarget")
            .and_then(|data| data.get_mut("pullRequest"))
            .map(serde_json::Value::take)
            .context("missing base update acknowledgement")?;
        let next_node = node
            .get_mut("repository")
            .and_then(|repository| repository.get_mut("next"))
            .map(serde_json::Value::take);
        let actual = decode_recovery(node)?;
        policy::validate_recovery_observation(&actual, expected.number, &self.config.repo.name)?;
        policy::same_recovery_head(expected, &actual, base)?;
        // Ancillary errors cannot revoke a complete, validated mutation acknowledgement.
        Ok(next.filter(|_| !partial).and_then(|number| {
            next_node
                .and_then(|node| decode_recovery(node).ok())
                .filter(|pr| {
                    policy::validate_recovery_observation(pr, number, &self.config.repo.name)
                        .is_ok()
                })
        }))
    }

    fn recovery_head(
        &self,
        number: u64,
        started: Instant,
    ) -> Result<(RecoveryPullRequest, Instant)> {
        let (mut prs, acquired) = self.recovery_prs_at(&[number], Some(started))?;
        prs.pop()
            .map(|pr| (pr, acquired))
            .context("missing recovery observation")
    }
}

pub(super) fn decode_recovery<'de>(
    node: impl serde::Deserializer<'de, Error = serde_json::Error>,
) -> Result<RecoveryPullRequest> {
    use crate::model::pull_request::{NativeStack, RecoveryReference, RecoveryRepository};
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Observation {
        number: u64,
        id: String,
        state: String,
        merged: bool,
        #[serde(deserialize_with = "Option::deserialize")]
        merged_at: Option<String>,
        #[serde(deserialize_with = "Option::deserialize")]
        merge_commit: Option<Merge>,
        head_ref_name: String,
        head_ref_oid: String,
        head_repository: Repository,
        base_ref_name: String,
        base_ref_oid: String,
        base_repository: Repository,
        #[serde(deserialize_with = "Option::deserialize")]
        stack: Option<NativeStack>,
    }
    #[derive(Deserialize)]
    struct Repository {
        #[serde(rename = "nameWithOwner")]
        full_name: String,
    }
    #[derive(Deserialize)]
    struct Merge {
        oid: String,
    }
    let pr = Observation::deserialize(node)?;
    let state = match pr.state.as_str() {
        "OPEN" => "open",
        "CLOSED" | "MERGED" => "closed",
        _ => bail!("invalid recovery lifecycle"),
    };
    Ok(RecoveryPullRequest {
        number: pr.number,
        node_id: pr.id,
        state: state.into(),
        merged: pr.merged,
        merged_at: pr.merged_at,
        merge_commit_sha: pr.merge_commit.map(|commit| commit.oid),
        head: RecoveryReference {
            name: pr.head_ref_name,
            sha: pr.head_ref_oid,
            repo: RecoveryRepository {
                full_name: pr.head_repository.full_name,
            },
        },
        base: RecoveryReference {
            name: pr.base_ref_name,
            sha: pr.base_ref_oid,
            repo: RecoveryRepository {
                full_name: pr.base_repository.full_name,
            },
        },
        stack: pr.stack,
    })
}
