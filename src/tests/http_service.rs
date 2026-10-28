//! Stateful protocol fixture shared by in-memory and disposable-repository tests.

use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) fn respond(
    prs: &mut BTreeMap<u64, Value>,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: Option<&Value>,
) -> Result<Value> {
    let payload = body.cloned().unwrap_or(Value::Null);
    if path == "graphql" {
        if payload["query"].as_str().unwrap().starts_with("mutation") {
            let mut data = json!({});
            for (key, input) in payload["variables"].as_object().unwrap() {
                let number: u64 = input["pullRequestId"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("PR_")
                    .parse()
                    .unwrap();
                let pr = prs.get_mut(&number).unwrap();
                for field in ["title", "body"] {
                    if let Some(value) = input.get(field) {
                        pr[field] = value.clone();
                    }
                }
                data[key] = json!({"pullRequest":{"number":number}});
            }
            return Ok(json!({"data":data}));
        }
        let mut repository = json!({});
        for (key, reference) in payload["variables"].as_object().unwrap() {
            if let Some(index) = key.strip_prefix('b') {
                repository[format!("p{index}")] = json!({"nodes":prs.values().filter(|p| p["base"]["ref"] == *reference && p["state"] == "open").map(|p| json!({"number":p["number"],"headRefName":p["head"]["ref"]})).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false}});
                continue;
            }
            if let Some(index) = key
                .strip_prefix('n')
                .filter(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
            {
                repository[format!("p{index}")] = prs.get(&reference.as_u64().unwrap()).map_or(Value::Null, |pr| json!({"number":pr["number"], "title":pr["title"], "state":pr["state"].as_str().unwrap_or("open").to_uppercase(), "isDraft":pr["draft"].as_bool().unwrap_or(false), "merged":pr["merged"].as_bool().unwrap_or(false) || pr["merged_at"].is_string()}));
                continue;
            }
            let Some(index) = key.strip_prefix('h') else {
                continue;
            };
            let nodes: Vec<_> = prs.values().filter(|p| p["head"]["ref"] == *reference).map(|p| json!({"number":p["number"],"id":p.get("node_id").cloned().unwrap_or_else(||json!(format!("PR_{}",p["number"]))),"title":p["title"].as_str().unwrap_or(""),"body":p["body"].as_str().unwrap_or(""),"state":p["state"].as_str().unwrap_or("open").to_uppercase(),"merged":p.get("merged").unwrap_or(&json!(false)),"isDraft":p["draft"].as_bool().unwrap_or(false),"headRefName":p["head"]["ref"],"baseRefName":p["base"]["ref"],"headRepository":{"nameWithOwner":"org/repo"},"autoMergeRequest":if p["auto_merge"].is_object() {json!({"enabledAt":"2026-10-27T00:00:00Z"})} else {p["auto_merge"].clone()},"stack":p["stack"],"labels":{"nodes":p["labels"].as_array().into_iter().flatten().map(|l| if l.is_string(){json!({"name":l})} else {l.clone()}).collect::<Vec<_>>(),"pageInfo":{"hasNextPage":false}}})).collect();
            repository[format!("p{index}")] =
                json!({"nodes":nodes,"pageInfo":{"hasNextPage":false}});
        }
        return Ok(json!({"data":{"repository":repository}}));
    }
    if path == "pulls" && method == "POST" {
        let number = prs.keys().max().copied().unwrap_or(100) + 1;
        let pr = json!({"number":number,"node_id":format!("PR_{number}"),"title":payload["title"],"body":payload["body"],"head":{"ref":payload["head"],"repo":{"full_name":"org/repo"}},"base":{"ref":payload["base"],"repo":{"full_name":"org/repo"}},"draft":payload["draft"],"state":"open","merged":false,"auto_merge":null,"labels":[]});
        prs.insert(number, pr.clone());
        return Ok(pr);
    }
    if path == "pulls" {
        let url = reqwest::Url::parse(&format!("http://localhost/?{}", query.unwrap_or("")))?;
        let values: BTreeMap<_, _> = url.query_pairs().collect();
        let head = values
            .get("head")
            .and_then(|h| h.strip_prefix("org:"))
            .unwrap_or("");
        return Ok(json!(
            prs.values()
                .filter(|p| p["head"]["ref"] == head
                    && (values.get("state").is_none_or(|s| s == "all")
                        || p["state"] == values["state"].as_ref()))
                .collect::<Vec<_>>()
        ));
    }
    if let Some(number) = path.strip_prefix("pulls/") {
        let pr = prs
            .get_mut(&number.parse()?)
            .ok_or_else(|| anyhow::anyhow!("not found"))?;
        if method == "PATCH" {
            assert!(payload.get("draft").is_none());
            if payload.get("base").is_some() && (pr["state"] == "closed" || pr["merged"] == true) {
                bail!("Cannot change the base branch of a closed pull request.");
            }
            if payload.get("base").is_some() && pr["stack"].is_object() {
                bail!("pull request is part of a stack");
            }
            for field in ["body", "title"] {
                if let Some(value) = payload.get(field) {
                    pr[field] = value.clone();
                }
            }
            if let Some(base) = payload.get("base") {
                pr["base"]["ref"] = base.clone();
            }
        }
        let mut observed = pr.clone();
        observed
            .as_object_mut()
            .unwrap()
            .entry("auto_merge")
            .or_insert(Value::Null);
        return Ok(observed);
    }
    if path.starts_with("commits/") {
        return Ok(json!(
            prs.values()
                .cloned()
                .map(|mut pr| {
                    pr["head"]["repo"] = json!({"full_name":"org/repo"});
                    pr
                })
                .collect::<Vec<_>>()
        ));
    }
    if let Some(suffix) = path.strip_prefix("issues/") {
        let (number, suffix) = suffix.split_once("/labels").unwrap();
        let pr = prs.get_mut(&number.parse()?).unwrap();
        if !pr["labels"].is_array() {
            pr["labels"] = json!([]);
        }
        let labels = pr["labels"].as_array_mut().unwrap();
        if method == "POST" {
            for label in payload["labels"].as_array().unwrap() {
                if !labels
                    .iter()
                    .any(|existing| label_name(existing).eq_ignore_ascii_case(label_name(label)))
                {
                    labels.push(label.clone());
                }
            }
        } else if method == "DELETE" {
            let url = reqwest::Url::parse(&format!(
                "http://localhost/?name={}",
                suffix.trim_start_matches('/')
            ))?;
            let label = url.query_pairs().next().unwrap().1;
            labels.retain(|value| !label_name(value).eq_ignore_ascii_case(&label));
        }
        return Ok(json!(
            labels
                .iter()
                .map(|l| if l.is_string() {
                    json!({"name":l})
                } else {
                    l.clone()
                })
                .collect::<Vec<_>>()
        ));
    }
    if path == "stacks" || path.starts_with("stacks/") {
        let number = path
            .split('/')
            .nth(1)
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(9);
        if path.ends_with("/unstack") {
            for pr in prs.values_mut() {
                pr["stack"] = Value::Null;
            }
            return Ok(Value::Null);
        }
        if method == "POST" {
            for number in payload["pull_requests"].as_array().unwrap() {
                let number = number.as_u64().unwrap();
                if prs[&number]["auto_merge"].is_object() {
                    return Err(crate::github::http::HttpStatusError {
                        url: format!("https://api.github.com/repos/org/repo/{path}"),
                        status: reqwest::StatusCode::UNPROCESSABLE_ENTITY,
                        body: json!({"message":format!("Pull request #{number} has auto-merge enabled"),"status":"422"}).to_string(),
                        quota: crate::github::limits::Quota::default(),
                    }.into());
                }
            }
            for number in payload["pull_requests"].as_array().unwrap() {
                prs.get_mut(&number.as_u64().unwrap()).unwrap()["stack"] = json!({"number":9});
            }
        }
        return Ok(
            json!({"number":number,"base":{"ref":"main"},"pull_requests":prs.values().filter(|p| p["stack"]["number"] == number).collect::<Vec<_>>() }),
        );
    }
    bail!("unexpected fixture request: {method} {path}")
}

fn label_name(value: &Value) -> &str {
    value.as_str().or_else(|| value["name"].as_str()).unwrap()
}
