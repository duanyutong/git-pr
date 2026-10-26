//! Complete, invocation-scoped observations of relevant pull requests.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{GitHub, Ordering, guard};
use crate::model::PullRequest;

const BATCH_SIZE: usize = 25;

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
    head: RestReference,
    base: RestReference,
    stack: Option<crate::model::pull_request::NativeStack>,
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
        head: crate::model::pull_request::Reference { name: pr.head.name },
        base: crate::model::pull_request::Reference { name: pr.base.name },
        stack: pr.stack,
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
    pr: PullRequest,
    labels: Cursor,
    stack_known: bool,
}

#[derive(Debug)]
struct HeadObservation<'a> {
    reference: &'a String,
    cursor: Cursor,
    prs: Vec<Observation>,
    seen: HashSet<u64>,
}

impl GitHub<'_> {
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

    /// Updating a base branch also affects PRs outside the selected local stack.
    pub(crate) fn check_publication_dependants(
        &self,
        references: &[String],
        selected: &std::collections::HashMap<u64, (&str, &str)>,
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
                        "p{i}:pullRequests(baseRefName:$b{i},states:[OPEN],first:100,after:$c{i}){{nodes{{number headRefName}} pageInfo{{hasNextPage endCursor}}}} "
                    );
                    variables[format!("b{i}")] = json!(reference);
                    variables[format!("c{i}")] = json!(cursor.after);
                }
                let query = format!(
                    "query({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"
                );
                let response: Value = serde_json::from_slice(
                    &self
                        .request_url(
                            "POST",
                            &format!("https://api.{}/graphql", self.config.repo.host),
                            Some(&json!({"query":query,"variables":variables})),
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
                        let number = node["number"]
                            .as_u64()
                            .filter(|n| *n != 0)
                            .context("missing dependant PR identity")?;
                        let head = node["headRefName"]
                            .as_str()
                            .filter(|h| !h.is_empty())
                            .context("missing dependant PR head")?;
                        if selected.get(&number) != Some(&(head, reference.as_str())) {
                            bail!(
                                "unsafe publication: open PR #{number} (head {head:?}) depends on branch {reference:?} outside the verified selection. Updating that branch can indirectly merge or retarget the PR; include its complete stack or arrange a separately verified transition before retrying"
                            );
                        }
                    }
                    cursor.advance(connection)?;
                }
                pending.retain(|(_, cursor)| cursor.after.is_some());
            }
        }
        Ok(())
    }

    pub(crate) fn observe(&self, references: &[String]) -> Result<()> {
        // Hold ownership while fetching: parallel consumers cannot start duplicate lookups.
        let mut heads = self.session.heads();
        {
            let mut times = guard(&self.session.head_times);
            for reference in references {
                if times.get(reference).is_some_and(|time| !self.fresh(time)) {
                    times.remove(reference);
                    heads.remove(reference);
                }
            }
            drop(times);
        }
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
                    reference,
                    cursor: Cursor::default(),
                    prs: Vec::new(),
                    seen: HashSet::new(),
                })
                .collect();
            let mut pending: Vec<_> = (0..observations.len()).collect();
            while !pending.is_empty() {
                let (mut response, with_stack) = self.observe_chunk(
                    &pending
                        .iter()
                        .map(|&i| &observations[i])
                        .collect::<Vec<_>>(),
                    owner,
                    name,
                )?;
                for (alias, &index) in pending.iter().enumerate() {
                    let head = &mut observations[index];
                    let connection = response
                        .get_mut("data")
                        .and_then(|data| data.get_mut("repository"))
                        .and_then(|repo| repo.get_mut(format!("p{alias}")))
                        .context("incomplete PR observation")?;
                    let Value::Array(nodes) = connection
                        .get_mut("nodes")
                        .context("incomplete PR observation")?
                        .take()
                    else {
                        bail!("incomplete PR observation");
                    };
                    // A concurrent update can move a PR between pages; keep its first observation.
                    head.prs.extend(
                        decode_nodes(nodes, &self.config.repo.name, head.reference, with_stack)?
                            .into_iter()
                            .filter(|observation| head.seen.insert(observation.pr.number)),
                    );
                    head.cursor.advance(connection)?;
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
            )?;
            for head in observations {
                let mut numbers = Vec::new();
                for observation in head.prs {
                    let number = observation.pr.number;
                    numbers.push(number);
                    self.observed(observation.pr);
                    if !observation.stack_known {
                        guard(&self.session.unknown_stacks).insert(number);
                    }
                }
                heads.insert(head.reference.clone(), numbers);
                guard(&self.session.head_times).insert(head.reference.clone(), self.now());
            }
        }
        drop(heads);
        Ok(())
    }

    fn observe_chunk(
        &self,
        chunk: &[&HeadObservation<'_>],
        owner: &str,
        name: &str,
    ) -> Result<(Value, bool)> {
        let mut with_stack = !self.session.stack_field_unsupported.load(Ordering::Relaxed);
        loop {
            let mut declarations = String::from("$owner:String!,$name:String!");
            let mut fields = String::new();
            let mut variables = json!({"owner":owner,"name":name});
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
                    "p{i}:pullRequests(headRefName:$h{i},first:100{after},orderBy:{{field:UPDATED_AT,direction:DESC}}){{nodes{{id number title body state merged headRefName baseRefName headRepository{{nameWithOwner}} labels(first:100){{nodes{{name}} pageInfo{{hasNextPage endCursor}}}} {stack}}} pageInfo{{hasNextPage endCursor}}}} "
                );
                variables[format!("h{i}")] = json!(head.reference);
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
            let Some(errors) = response.get("errors") else {
                return Ok((response, with_stack));
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
                let response: Value = serde_json::from_slice(
                    &self
                        .request_url(
                            "POST",
                            &url,
                            Some(&json!({"query":query,"variables":variables})),
                        )?
                        .body,
                )
                .context("invalid GitHub label observation response")?;
                if let Some(errors) = response.get("errors") {
                    bail!("failed to observe PR labels: {errors}");
                }
                for (i, observation) in chunk.iter_mut().enumerate() {
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
        let prs = match self.pages(&format!("commits/{hash}/pulls")) {
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
                Ok(Vec::new())
            }
            result => result,
        }?;
        let prs: Associations = prs.into();
        cache.insert(hash.into(), (self.now(), Arc::clone(&prs)));
        drop(cache);
        Ok(prs)
    }

    fn pages(&self, path: &str) -> Result<Vec<Association>> {
        let mut all = Vec::new();
        let mut seen = HashSet::new();
        let mut included = HashSet::new();
        let mut page = 1_u64;
        loop {
            let bytes = self.request("GET", &format!("{path}?per_page=100&page={page}"), None)?;
            let nodes: Vec<AssociationObservation> =
                serde_json::from_slice(&bytes).with_context(|| {
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
                return Ok(all);
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
) -> Result<Vec<Observation>> {
    let mut prs = Vec::new();
    for mut node in nodes {
        if !repository_matches(node.get("headRepository"), "nameWithOwner", repo)? {
            continue;
        }
        if ["body", "title", "merged"]
            .iter()
            .any(|key| node.get(key).is_none())
        {
            bail!("incomplete PR identity or content observation");
        }
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
            pr,
            labels,
            stack_known,
        });
    }
    Ok(prs)
}

/// Move response values into REST-shaped fields without copying description text.
pub(super) fn normalise_node(node: &mut Value) {
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
