//! Complete, invocation-scoped observations of relevant pull requests.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

use super::{GitHub, Ordering, guard};
use crate::model::PullRequest;

const BATCH_SIZE: usize = 25;

#[derive(Debug)]
pub(crate) struct PublicationDependant {
    pub number: u64,
    pub head: String,
    pub base: String,
    pub hash: String,
}

fn dependant_identity(node: &Value) -> Result<(u64, &str)> {
    let number = node["number"]
        .as_u64()
        .filter(|n| *n != 0)
        .context("missing dependant PR identity")?;
    let head = node["headRefName"]
        .as_str()
        .filter(|head| !head.is_empty())
        .context("missing dependant PR head")?;
    Ok((number, head))
}

#[derive(Debug, Deserialize)]
struct Repository {
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct RestReference {
    #[serde(rename = "ref")]
    name: String,
    #[serde(deserialize_with = "Option::deserialize")]
    repo: Option<Repository>,
}

#[derive(Debug, Deserialize)]
struct RestObservation {
    number: u64,
    node_id: String,
    #[serde(deserialize_with = "crate::model::pull_request::nullable_body")]
    body: Arc<str>,
    title: String,
    labels: Vec<crate::model::pull_request::Label>,
    state: String,
    merged: bool,
    merged_at: Option<String>,
    #[serde(default)]
    draft: Option<bool>,
    head: RestReference,
    base: RestReference,
    stack: Option<crate::model::pull_request::NativeStack>,
    #[serde(default, deserialize_with = "crate::model::pull_request::auto_merge")]
    auto_merge: Option<bool>,
}

pub(super) fn decode_rest(
    bytes: &[u8],
    repository: &str,
    creation: Option<(&str, &str)>,
) -> Result<PullRequest> {
    let pr: RestObservation =
        serde_json::from_slice(bytes).context("incomplete or invalid PR observation")?;
    if pr.number == 0
        || pr.node_id.is_empty()
        || pr.head.name.is_empty()
        || pr.base.name.is_empty()
        || !matches!(pr.state.as_str(), "open" | "closed")
    {
        bail!("incomplete PR identity or content observation");
    }
    if pr
        .base
        .repo
        .as_ref()
        .is_none_or(|repo| !repo.full_name.eq_ignore_ascii_case(repository))
        || pr
            .head
            .repo
            .as_ref()
            .is_some_and(|repo| !repo.full_name.eq_ignore_ascii_case(repository))
    {
        bail!("PR observation returned an unexpected repository identity; expected {repository}");
    }
    if let Some((head, base)) = creation
        && (pr.head.name != head
            || pr.base.name != base
            || pr.head.repo.is_none()
            || pr.state != "open"
            || pr.merged
            || pr.merged_at.is_some())
    {
        bail!(
            "PR creation returned an unexpected identity; expected open head {head:?} and base {base:?}"
        );
    }
    Ok(PullRequest {
        number: pr.number,
        node_id: pr.node_id,
        body: pr.body,
        title: pr.title,
        labels: pr.labels,
        state: pr.state,
        merged: pr.merged,
        merged_at: pr.merged_at,
        draft: pr.draft,
        head: crate::model::pull_request::Reference { name: pr.head.name },
        base: crate::model::pull_request::Reference { name: pr.base.name },
        stack: pr.stack,
        auto_merge: pr.auto_merge,
    })
}

/// Commit associations deliberately omit content which their consumers never use.
#[derive(Debug)]
pub(super) struct Association {
    pub number: u64,
    pub state: String,
    pub head: String,
    pub head_oid: String,
}

pub(super) type Associations = Arc<[Association]>;

#[derive(Debug, Deserialize)]
struct AssociationObservation {
    #[serde(default)]
    number: u64,
    #[serde(default)]
    state: String,
    head: AssociationHead,
}

#[derive(Debug, Deserialize)]
struct AssociationHead {
    #[serde(default)]
    sha: String,
    #[serde(default, rename = "ref")]
    name: String,
    #[serde(deserialize_with = "Option::deserialize")]
    repo: Option<Repository>,
}

#[derive(Debug, Default)]
struct Cursor {
    after: Option<String>,
    seen: HashSet<String>,
}

impl Cursor {
    fn advance(&mut self, connection: &Value) -> Result<()> {
        self.after = if connection["pageInfo"]["hasNextPage"]
            .as_bool()
            .context("missing page information")?
        {
            let cursor = connection["pageInfo"]["endCursor"]
                .as_str()
                .filter(|cursor| !cursor.is_empty())
                .context("missing pagination cursor")?;
            if !self.seen.insert(cursor.to_owned()) {
                bail!("pagination cursor did not advance");
            }
            Some(cursor.to_owned())
        } else {
            None
        };
        Ok(())
    }
}

#[derive(Debug)]
struct Observation {
    acquired: Instant,
    pr: PullRequest,
    labels: Cursor,
    stack_known: bool,
    recovery: Option<crate::model::pull_request::RecoveryPullRequest>,
}

#[derive(Debug)]
struct HeadObservation<'a> {
    acquired: Instant,
    reference: &'a String,
    cursor: Cursor,
    prs: Vec<Observation>,
    seen: HashSet<u64>,
}

impl HeadObservation<'_> {
    fn append(
        &mut self,
        connection: &mut Value,
        repo: &str,
        with_stack: bool,
        recovery: Option<&HashSet<u64>>,
        acquired: Instant,
    ) -> Result<()> {
        self.acquired = if self.cursor.after.is_none() {
            acquired
        } else {
            self.acquired.min(acquired)
        };
        let Value::Array(nodes) = connection
            .get_mut("nodes")
            .context("incomplete PR observation")?
            .take()
        else {
            bail!("incomplete PR observation");
        };
        // A concurrent update can move a PR between pages; keep its first observation.
        self.prs.extend(
            decode_nodes(nodes, repo, self.reference, with_stack, recovery, acquired)?
                .into_iter()
                .filter(|observation| self.seen.insert(observation.pr.number)),
        );
        self.cursor.advance(connection)
    }
}

impl GitHub<'_> {
    pub(crate) fn summaries(
        &self,
        numbers: &[u64],
    ) -> Vec<crate::model::pull_request::PullRequestSummary> {
        use crate::model::pull_request::{Lifecycle, PullRequestSummary};
        let mut seen = HashSet::new();
        let numbers: Vec<_> = numbers
            .iter()
            .copied()
            .filter(|n| *n != 0 && seen.insert(*n))
            .collect();
        let mut summaries = std::collections::BTreeMap::new();
        {
            let prs = self.session.prs();
            let times = guard(&self.session.pr_times);
            for number in &numbers {
                if let Some(pr) = prs.get(number)
                    && times.get(number).is_some_and(|time| self.fresh(time))
                {
                    summaries.insert(*number, pr.summary());
                }
            }
        }
        let missing: Vec<_> = numbers
            .iter()
            .copied()
            .filter(|n| !summaries.contains_key(n))
            .collect();
        for chunk in missing.chunks(BATCH_SIZE) {
            match self.summary_batch(chunk) {
                Ok(observed) => summaries.extend(observed.into_iter().map(|pr| (pr.number, pr))),
                Err(error) => self.commands.warn(&format!(
                    "could not verify PR details for the completion listing; retained rows are shown as unverified: {error:#}"
                )),
            }
        }
        numbers
            .into_iter()
            .map(|number| {
                summaries.remove(&number).unwrap_or(PullRequestSummary {
                    number,
                    title: None,
                    lifecycle: Lifecycle::Unknown,
                })
            })
            .collect()
    }

    fn summary_batch(
        &self,
        numbers: &[u64],
    ) -> Result<Vec<crate::model::pull_request::PullRequestSummary>> {
        use crate::model::pull_request::{Lifecycle, PullRequestSummary};
        use std::fmt::Write as _;
        let (owner, name) = self
            .config
            .repo
            .name
            .split_once('/')
            .context("invalid repository identity")?;
        let mut declarations = String::from("$owner:String!,$name:String!");
        let mut fields = String::new();
        let mut variables = json!({"owner": owner, "name": name});
        for (i, number) in numbers.iter().enumerate() {
            let number =
                i32::try_from(*number).context("PR number exceeds the GraphQL integer range")?;
            let _ = write!(declarations, ",$n{i}:Int!");
            let _ = write!(
                fields,
                "p{i}:pullRequest(number:$n{i}){{number title state merged isDraft}} "
            );
            variables[format!("n{i}")] = json!(number);
        }
        let response: Value = serde_json::from_slice(&self.request_url(
            "POST", &format!("https://api.{}/graphql", self.config.repo.host),
            Some(&json!({"query":format!("query({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"),"variables":variables})),
        )?.body).context("invalid PR summary response")?;
        if let Some(errors) = response.get("errors") {
            bail!("failed to observe PR summaries: {errors}");
        }
        numbers
            .iter()
            .enumerate()
            .map(|(i, &number)| {
                let pr = &response["data"]["repository"][format!("p{i}")];
                if pr["number"] != number {
                    bail!("missing or unexpected PR summary identity #{number}");
                }
                let state = pr["state"]
                    .as_str()
                    .context("missing PR state")?
                    .to_ascii_lowercase();
                let lifecycle =
                    Lifecycle::observed(&state, pr["merged"].as_bool(), pr["isDraft"].as_bool());
                if lifecycle == Lifecycle::Unknown {
                    bail!("incomplete PR lifecycle observation for #{number}");
                }
                Ok(PullRequestSummary {
                    number,
                    title: Some(pr["title"].as_str().context("missing PR title")?.to_owned()),
                    lifecycle,
                })
            })
            .collect()
    }

    pub(crate) fn merged_prs(&self, numbers: &[u64]) -> Result<HashSet<u64>> {
        let mut merged = HashSet::new();
        let mut missing = Vec::new();
        let mut seen = HashSet::new();
        {
            let prs = self.session.prs();
            let times = guard(&self.session.pr_times);
            for &number in numbers {
                if !seen.insert(number) {
                    continue;
                }
                if let Some(pr) = prs.get(&number)
                    && times.get(&number).is_some_and(|time| self.fresh(time))
                {
                    if pr.is_merged() {
                        merged.insert(number);
                    }
                } else {
                    missing.push(number);
                }
            }
        }
        if missing.is_empty() {
            return Ok(merged);
        }
        let (owner, name) = self
            .config
            .repo
            .name
            .split_once('/')
            .context("invalid repository identity")?;
        for chunk in missing.chunks(BATCH_SIZE) {
            let mut declarations = String::from("$owner:String!,$name:String!");
            let mut fields = String::new();
            let mut variables = json!({"owner":owner,"name":name});
            for (i, &number) in chunk.iter().enumerate() {
                use std::fmt::Write as _;
                let number =
                    i32::try_from(number).context("PR number exceeds the GraphQL integer range")?;
                let _ = write!(declarations, ",$n{i}:Int!");
                let _ = write!(fields, "p{i}:pullRequest(number:$n{i}){{number merged}} ");
                variables[format!("n{i}")] = json!(number);
            }
            let query =
                format!("query({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}");
            let url = format!("https://api.{}/graphql", self.config.repo.host);
            let response: Value = serde_json::from_slice(
                &self
                    .request_url(
                        "POST",
                        &url,
                        Some(&json!({"query":query,"variables":variables})),
                    )?
                    .body,
            )
            .context("invalid GitHub GraphQL observation response")?;
            if let Some(errors) = response.get("errors") {
                bail!("failed to observe historical PR status: {errors}");
            }
            for (i, &number) in chunk.iter().enumerate() {
                let pr = &response["data"]["repository"][format!("p{i}")];
                if number == 0 || pr["number"] != number {
                    bail!("missing or unexpected historical PR identity #{number}");
                }
                if pr["merged"]
                    .as_bool()
                    .context("missing historical PR merge status")?
                {
                    merged.insert(number);
                }
            }
        }
        Ok(merged)
    }

    /// Transitions require exclusive ownership of their affected PR relationships.
    pub(crate) fn check_publication_dependants(
        &self,
        references: &[String],
        selected: &std::collections::HashMap<u64, (&str, &str)>,
    ) -> Result<()> {
        self.check_publication_dependants_with_budget(references, selected, None)
    }

    pub(crate) fn check_publication_dependants_with_budget(
        &self,
        references: &[String],
        selected: &std::collections::HashMap<u64, (&str, &str)>,
        started: Option<Instant>,
    ) -> Result<()> {
        self.visit_publication_dependants(references, started, |reference, node| {
            let (number, head) = dependant_identity(node)?;
            if selected.get(&number) != Some(&(head, reference)) {
                bail!(
                    "unsafe publication: open PR #{number} (head {head:?}) depends on branch {reference:?} outside the verified selection. Updating that branch can indirectly merge or retarget the PR; include its complete stack or arrange a separately verified transition before retrying"
                );
            }
            Ok(())
        })
    }

    pub(crate) fn publication_dependants(
        &self,
        references: &[String],
        selected: &std::collections::HashMap<u64, (&str, &str)>,
    ) -> Result<Vec<PublicationDependant>> {
        let mut outside = Vec::new();
        self.visit_publication_dependants(references, None, |reference, node| {
            let (number, head) = dependant_identity(node)?;
            if let Some(expected) = selected.get(&number) {
                if *expected != (head, reference) {
                    bail!("publication dependant PR #{number} changed its selected head or base");
                }
            } else {
                let hash = node["headRefOid"].as_str()
                    .filter(|hash| crate::git::policy::valid_oid(hash))
                    .with_context(|| format!("cannot verify indirect merge for PR #{number} outside the verified selection: missing or invalid head object"))?;
                outside.push(PublicationDependant {
                    number, head: head.into(), base: reference.into(), hash: hash.into(),
                });
            }
            Ok(())
        })?;
        Ok(outside)
    }

    fn visit_publication_dependants(
        &self,
        references: &[String],
        started: Option<Instant>,
        mut visit: impl FnMut(&str, &Value) -> Result<()>,
    ) -> Result<()> {
        let (owner, name) = self
            .config
            .repo
            .name
            .split_once('/')
            .context("invalid repository identity")?;
        for chunk in references.chunks(BATCH_SIZE) {
            let mut pending: Vec<_> = chunk
                .iter()
                .map(|reference| (reference, Cursor::default()))
                .collect();
            while !pending.is_empty() {
                let mut declarations = String::from("$owner:String!,$name:String!");
                let mut fields = String::new();
                let mut variables = json!({"owner":owner,"name":name});
                for (i, (reference, cursor)) in pending.iter().enumerate() {
                    use std::fmt::Write as _;
                    let _ = write!(declarations, ",$b{i}:String!,$c{i}:String");
                    let _ = write!(
                        fields,
                        "p{i}:pullRequests(baseRefName:$b{i},states:[OPEN],first:100,after:$c{i}){{nodes{{number headRefName headRefOid}} pageInfo{{hasNextPage endCursor}}}} "
                    );
                    variables[format!("b{i}")] = json!(reference);
                    variables[format!("c{i}")] = json!(cursor.after);
                }
                let query = format!(
                    "query({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"
                );
                let response: Value = serde_json::from_slice(
                    &self
                        .request_url_with_budget(
                            "POST",
                            &format!("https://api.{}/graphql", self.config.repo.host),
                            Some(&json!({"query":query,"variables":variables})),
                            started,
                        )?
                        .body,
                )
                .context("invalid publication dependant observation")?;
                if let Some(errors) = response.get("errors") {
                    bail!("failed to observe PRs depending on published branches: {errors}");
                }
                for (i, (reference, cursor)) in pending.iter_mut().enumerate() {
                    let connection = &response["data"]["repository"][format!("p{i}")];
                    let nodes = connection["nodes"]
                        .as_array()
                        .context("incomplete publication dependant observation")?;
                    for node in nodes {
                        visit(reference, node)?;
                    }
                    cursor.advance(connection)?;
                }
                pending.retain(|(_, cursor)| cursor.after.is_some());
            }
        }
        Ok(())
    }

    pub(crate) fn observe(&self, references: &[String]) -> Result<()> {
        self.observe_mode(references, None, None).map(|_| ())
    }

    pub(super) fn observe_with_budget(
        &self,
        references: &[String],
        started: Instant,
    ) -> Result<()> {
        self.observe_mode(references, None, Some(started))
            .map(|_| ())
    }

    /// Refresh identity, content and selected recovery metadata in the same observation.
    pub(crate) fn observe_recovery(
        &self,
        references: &[String],
        numbers: &[u64],
    ) -> Result<Vec<crate::model::pull_request::RecoveryPullRequest>> {
        self.observe_recovery_with_budget(references, numbers, self.now())
            .map(|(prs, _)| prs)
    }

    pub(crate) fn observe_recovery_with_budget(
        &self,
        references: &[String],
        numbers: &[u64],
        started: Instant,
    ) -> Result<(
        Vec<crate::model::pull_request::RecoveryPullRequest>,
        Instant,
    )> {
        let selected = numbers.iter().copied().collect();
        let mut attempt = 0;
        loop {
            self.invalidate_heads(references);
            let (mut recovered, acquired) =
                self.observe_mode(references, Some(&selected), Some(started))?;
            let prs = numbers
                .iter()
                .map(|number| {
                    recovered
                        .remove(number)
                        .with_context(|| format!("missing recovery observation for PR #{number}"))
                })
                .collect::<Result<Vec<_>>>()?;
            if self.fresh(&acquired) {
                return Ok((prs, acquired));
            }
            if attempt == super::limits::RETRIES {
                return Err(super::limits::Exhausted(
                    "combined PR observation repeatedly expired during acquisition".into(),
                )
                .into());
            }
            attempt += 1;
        }
    }

    fn observe_mode(
        &self,
        references: &[String],
        recovery: Option<&HashSet<u64>>,
        recovery_started: Option<Instant>,
    ) -> Result<(
        BTreeMap<u64, crate::model::pull_request::RecoveryPullRequest>,
        Instant,
    )> {
        let mut recovered = BTreeMap::new();
        let mut acquired = None;
        // Hold ownership while fetching: parallel consumers cannot start duplicate lookups.
        let mut heads = self.session.heads();
        self.expire_heads(&mut heads, references);
        let mut seen = HashSet::new();
        let missing: Vec<_> = references
            .iter()
            .filter(|r| !r.is_empty() && !heads.contains_key(*r) && seen.insert(r.as_str()))
            .collect();
        let (owner, name) = self
            .config
            .repo
            .name
            .split_once('/')
            .context("invalid repository identity")?;
        for chunk in missing.chunks(BATCH_SIZE) {
            let mut observations: Vec<_> = chunk
                .iter()
                .map(|reference| HeadObservation {
                    acquired: self.now(),
                    reference,
                    cursor: Cursor::default(),
                    prs: Vec::new(),
                    seen: HashSet::new(),
                })
                .collect();
            let mut pending: Vec<_> = (0..observations.len()).collect();
            while !pending.is_empty() {
                let (mut response, with_stack, acquired) = self.observe_chunk(
                    &pending
                        .iter()
                        .map(|&i| &observations[i])
                        .collect::<Vec<_>>(),
                    owner,
                    name,
                    recovery.is_some(),
                    recovery_started,
                )?;
                for (alias, &index) in pending.iter().enumerate() {
                    let head = &mut observations[index];
                    let connection = response
                        .get_mut("data")
                        .and_then(|data| data.get_mut("repository"))
                        .and_then(|repo| repo.get_mut(format!("p{alias}")))
                        .context("incomplete PR observation")?;
                    head.append(
                        connection,
                        &self.config.repo.name,
                        with_stack,
                        recovery,
                        acquired,
                    )?;
                }
                pending.retain(|&i| observations[i].cursor.after.is_some());
            }
            self.complete_labels(
                observations
                    .iter_mut()
                    .flat_map(|head| &mut head.prs)
                    .collect(),
                owner,
                name,
                recovery_started,
            )?;
            for head in observations {
                acquired =
                    Some(acquired.map_or(head.acquired, |time: Instant| time.min(head.acquired)));
                let mut numbers = Vec::new();
                for observation in head.prs {
                    let number = observation.pr.number;
                    numbers.push(number);
                    if let Some(pr) = observation.recovery {
                        recovered.insert(number, pr);
                    }
                    self.observed(observation.pr, observation.acquired);
                    if !observation.stack_known {
                        guard(&self.session.unknown_stacks).insert(number);
                    }
                }
                heads.insert(head.reference.clone(), numbers);
                guard(&self.session.head_times).insert(head.reference.clone(), head.acquired);
            }
        }
        drop(heads);
        Ok((recovered, acquired.unwrap_or_else(|| self.now())))
    }

    fn expire_heads(&self, heads: &mut BTreeMap<String, Vec<u64>>, references: &[String]) {
        let mut times = guard(&self.session.head_times);
        for reference in references {
            if times.get(reference).is_some_and(|time| !self.fresh(time)) {
                times.remove(reference);
                heads.remove(reference);
            }
        }
        drop(times);
    }

    fn observe_chunk(
        &self,
        chunk: &[&HeadObservation<'_>],
        owner: &str,
        name: &str,
        recovery: bool,
        recovery_started: Option<Instant>,
    ) -> Result<(Value, bool, Instant)> {
        let mut with_stack = !self.session.stack_field_unsupported.load(Ordering::Relaxed);
        loop {
            let mut declarations = String::from("$owner:String!,$name:String!");
            let mut fields = String::new();
            let mut variables = json!({"owner":owner,"name":name});
            let recovery_fields = if recovery {
                super::recovery::HEAD_RECOVERY_FIELDS
            } else {
                ""
            };
            let stack = if with_stack { "stack{number}" } else { "" };
            for (i, head) in chunk.iter().enumerate() {
                use std::fmt::Write as _;
                let _ = write!(declarations, ",$h{i}:String!");
                let after = head
                    .cursor
                    .after
                    .as_ref()
                    .map_or_else(String::new, |cursor| {
                        let _ = write!(declarations, ",$c{i}:String!");
                        variables[format!("c{i}")] = json!(cursor);
                        format!(",after:$c{i}")
                    });
                let _ = write!(
                    fields,
                    "p{i}:pullRequests(headRefName:$h{i},first:100{after},orderBy:{{field:UPDATED_AT,direction:DESC}}){{nodes{{id number title body state merged isDraft headRefName baseRefName headRepository{{nameWithOwner}} autoMergeRequest{{enabledAt}} labels(first:100){{nodes{{name}} pageInfo{{hasNextPage endCursor}}}} {stack} {recovery_fields}}} pageInfo{{hasNextPage endCursor}}}} "
                );
                variables[format!("h{i}")] = json!(head.reference);
            }
            let operation = if recovery {
                "query RecoveryHeads"
            } else {
                "query"
            };
            let query = format!(
                "{operation}({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"
            );
            let url = format!("https://api.{}/graphql", self.config.repo.host);
            let response = self.request_url_with_budget(
                "POST",
                &url,
                Some(&json!({"query":query,"variables":variables})),
                recovery_started,
            )?;
            let acquired = response.acquired;
            let response: Value = serde_json::from_slice(&response.body)
                .context("invalid GitHub GraphQL observation response")?;
            let Some(errors) = response.get("errors") else {
                return Ok((response, with_stack, acquired));
            };
            let unsupported = errors.as_array().is_some_and(|errors| {
                !errors.is_empty()
                    && errors.iter().all(|error| {
                        error["message"].as_str().is_some_and(|message| {
                            message.contains("stack")
                                && (message.contains("doesn’t exist")
                                    || message.contains("doesn't exist")
                                    || message.contains("Cannot query field"))
                        })
                    })
            });
            if !with_stack || !unsupported {
                bail!("failed to observe pull requests: {errors}");
            }
            self.session
                .stack_field_unsupported
                .store(true, Ordering::Relaxed);
            with_stack = false;
        }
    }

    fn complete_labels(
        &self,
        mut pending: Vec<&mut Observation>,
        owner: &str,
        name: &str,
        recovery_started: Option<Instant>,
    ) -> Result<()> {
        pending.retain(|observation| observation.labels.after.is_some());
        while !pending.is_empty() {
            for chunk in pending.chunks_mut(BATCH_SIZE) {
                let mut declarations = String::from("$owner:String!,$name:String!");
                let mut fields = String::new();
                let mut variables = json!({"owner":owner,"name":name});
                for (i, observation) in chunk.iter().enumerate() {
                    use std::fmt::Write as _;
                    let number = i32::try_from(observation.pr.number)
                        .context("PR number exceeds the GraphQL integer range")?;
                    let _ = write!(declarations, ",$n{i}:Int!,$c{i}:String!");
                    let _ = write!(
                        fields,
                        "p{i}:pullRequest(number:$n{i}){{number labels(first:100,after:$c{i}){{nodes{{name}} pageInfo{{hasNextPage endCursor}}}}}} "
                    );
                    variables[format!("n{i}")] = json!(number);
                    variables[format!("c{i}")] = json!(observation.labels.after);
                }
                let query = format!(
                    "query({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"
                );
                let url = format!("https://api.{}/graphql", self.config.repo.host);
                let response = self.request_url_with_budget(
                    "POST",
                    &url,
                    Some(&json!({"query":query,"variables":variables})),
                    recovery_started,
                )?;
                let acquired = response.acquired;
                let response: Value = serde_json::from_slice(&response.body)
                    .context("invalid GitHub label observation response")?;
                if let Some(errors) = response.get("errors") {
                    bail!("failed to observe PR labels: {errors}");
                }
                for (i, observation) in chunk.iter_mut().enumerate() {
                    observation.acquired = observation.acquired.min(acquired);
                    let pr = &response["data"]["repository"][format!("p{i}")];
                    if pr["number"] != observation.pr.number {
                        bail!("unexpected PR identity in label page");
                    }
                    let labels = &pr["labels"];
                    observation.pr.labels.extend(
                        serde_json::from_value::<Vec<crate::model::pull_request::Label>>(
                            labels["nodes"].clone(),
                        )
                        .context("invalid label page")?,
                    );
                    observation.labels.advance(labels)?;
                }
            }
            pending.retain(|observation| observation.labels.after.is_some());
        }
        Ok(())
    }

    pub(crate) fn identity_heads(&self, hash: &str) -> Result<Vec<String>> {
        Ok(self
            .commit_prs(hash)?
            .iter()
            .filter(|pr| pr.head_oid == hash)
            .map(|pr| pr.head.clone())
            .collect())
    }

    pub(super) fn commit_prs(&self, hash: &str) -> Result<Associations> {
        let mut cache = guard(&self.session.commit_prs);
        if let Some((time, prs)) = cache.get(hash)
            && self.fresh(time)
        {
            return Ok(Arc::clone(prs));
        }
        let started = self.now();
        let (prs, acquired) = match self.pages(&format!("commits/{hash}/pulls")) {
            Err(error)
                if error
                    .downcast_ref::<crate::github::http::HttpStatusError>()
                    .is_some_and(|e| {
                        // Unpublished local commits may be reported as either status.
                        matches!(
                            e.status,
                            reqwest::StatusCode::NOT_FOUND
                                | reqwest::StatusCode::UNPROCESSABLE_ENTITY
                        ) && e.body.contains("No commit found")
                    }) =>
            {
                Ok((Vec::new(), started))
            }
            result => result,
        }?;
        let prs: Associations = prs.into();
        cache.insert(hash.into(), (acquired, Arc::clone(&prs)));
        drop(cache);
        Ok(prs)
    }

    fn pages(&self, path: &str) -> Result<(Vec<Association>, Instant)> {
        let mut all = Vec::new();
        let mut seen = HashSet::new();
        let mut included = HashSet::new();
        let mut page = 1_u64;
        let mut acquired = self.now();
        loop {
            let response = self.request_url(
                "GET",
                &self.repo_url(&format!("{path}?per_page=100&page={page}")),
                None,
            )?;
            acquired = if page == 1 {
                response.acquired
            } else {
                acquired.min(response.acquired)
            };
            let nodes: Vec<AssociationObservation> = serde_json::from_slice(&response.body)
                .with_context(|| {
                    format!("invalid commit PR observation for {path}, page {page}")
                })?;
            let complete = nodes.len() < 100;
            let mut progress = false;
            for pr in nodes {
                progress |= seen.insert(pr.number);
                if pr
                    .head
                    .repo
                    .as_ref()
                    .is_none_or(|repo| !repo.full_name.eq_ignore_ascii_case(&self.config.repo.name))
                {
                    continue;
                }
                if pr.number == 0
                    || pr.head.name.is_empty()
                    || !matches!(pr.state.as_str(), "open" | "closed")
                {
                    bail!("incomplete commit PR identity");
                }
                if included.insert(pr.number) {
                    all.push(Association {
                        number: pr.number,
                        state: pr.state,
                        head: pr.head.name,
                        head_oid: pr.head.sha,
                    });
                }
            }
            if complete {
                return Ok((all, acquired));
            }
            if !progress {
                bail!(
                    "commit PR pagination did not advance for {path} at page {page}; refusing repeated results"
                );
            }
            if page >= 100 {
                bail!("commit PR discovery exceeded its page limit; absence remains unknown");
            }
            page += 1;
        }
    }
}

fn decode_nodes(
    nodes: Vec<Value>,
    repo: &str,
    reference: &str,
    stack_known: bool,
    recovery: Option<&HashSet<u64>>,
    acquired: Instant,
) -> Result<Vec<Observation>> {
    let mut prs = Vec::new();
    for mut node in nodes {
        let stack_known = stack_known && node.get("stack").is_some();
        if !repository_matches(node.get("headRepository"), "nameWithOwner", repo)? {
            continue;
        }
        if ["body", "title", "merged"]
            .iter()
            .any(|key| node.get(key).is_none())
        {
            bail!("incomplete PR identity or content observation");
        }
        let recovery = if recovery.is_some_and(|selected| {
            node["number"]
                .as_u64()
                .is_some_and(|number| selected.contains(&number))
        }) {
            let pr = super::recovery::decode_recovery(&node)?;
            super::policy::validate_recovery_observation(&pr, pr.number, repo)?;
            Some(pr)
        } else {
            None
        };
        let mut labels = Cursor::default();
        labels.advance(&node["labels"])?;
        normalise_node(&mut node);
        let pr: PullRequest = serde_json::from_value(node).context("invalid PR observation")?;
        if pr.number == 0
            || pr.node_id.is_empty()
            || pr.head.name != reference
            || !matches!(pr.state.as_str(), "open" | "closed" | "merged")
        {
            bail!("incomplete PR identity or content observation");
        }
        prs.push(Observation {
            acquired,
            pr,
            labels,
            stack_known,
            recovery,
        });
    }
    Ok(prs)
}

/// Move response values into REST-shaped fields without copying description text.
pub(super) fn normalise_node(node: &mut Value) {
    if let Some(draft) = node
        .as_object_mut()
        .and_then(|fields| fields.remove("isDraft"))
    {
        node["draft"] = draft;
    }
    if let Some(request) = node
        .as_object_mut()
        .and_then(|fields| fields.remove("autoMergeRequest"))
    {
        node["auto_merge"] = request;
    }
    node["node_id"] = node["id"].take();
    node["head"] =
        Value::Object(std::iter::once(("ref".into(), node["headRefName"].take())).collect());
    node["base"] =
        Value::Object(std::iter::once(("ref".into(), node["baseRefName"].take())).collect());
    // REST-only metadata is not part of this GraphQL observation.
    let _ = node
        .as_object_mut()
        .map(|fields| fields.remove("merged_at"));
    node["labels"] = node["labels"]["nodes"].take();
    node["state"] = node["state"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .into();
}

fn repository_matches(repository: Option<&Value>, key: &str, expected: &str) -> Result<bool> {
    let repository = repository.context("missing PR head repository")?;
    if repository.is_null() {
        return Ok(false);
    }
    let name = repository[key]
        .as_str()
        .context("missing PR head repository identity")?;
    Ok(name.eq_ignore_ascii_case(expected))
}
