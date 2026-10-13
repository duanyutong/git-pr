use std::collections::HashSet;

use serde_json::{Value, json};

use crate::{
    cli::output,
    github, jj,
    model::trailers,
    model::{self, Commit},
    stack::description::{self, HistoryEntry, Style},
    stack::selection,
};

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}
fn strings(value: &Value) -> Vec<String> {
    array(value).iter().map(|v| text(v).to_owned()).collect()
}
fn boolean(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
}
fn number(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

fn commit(value: &Value) -> Commit {
    Commit {
        conflicted: false,
        hash: text(&value["Hash"]).into(),
        change_id: text(&value["ChangeID"]).into(),
        date: value["Date"]
            .as_str()
            .filter(|s| !s.starts_with("0001-"))
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            }),
        title: text(&value["Title"]).into(),
        message: text(&value["Message"]).into(),
        author_name: text(&value["AuthorName"]).into(),
        author_email: text(&value["AuthorEmail"]).into(),
        attrs: array(&value["Attrs"])
            .iter()
            .map(|a| (text(&a[0]).into(), text(&a[1]).into()))
            .collect(),
        pr_number: number(&value["PRNumber"]),
        skip: boolean(&value["Skip"]),
        newly_created: boolean(&value["NewlyCreated"]),
        base_blocked: boolean(&value["BaseBlocked"]),
        metadata_updated: false,
        base_updated: boolean(&value["BaseUpdated"]),
    }
}

fn commits(value: &Value) -> Vec<Commit> {
    array(value).iter().map(commit).collect()
}
fn commit_value(commit: &Commit) -> Value {
    json!({"Hash":commit.hash,"ChangeID":commit.change_id,"Date":commit.date.map_or_else(||"0001-01-01T00:00:00Z".into(),|d|d.to_rfc3339_opts(chrono::SecondsFormat::AutoSi,true)),
        "AuthorName":commit.author_name,"AuthorEmail":commit.author_email,"Title":commit.title,"Message":commit.message,"Attrs":commit.attrs,
        "PRNumber":commit.pr_number,"Skip":commit.skip,"NewlyCreated":commit.newly_created,"BaseBlocked":commit.base_blocked,"BaseUpdated":commit.base_updated})
}

fn normalise(value: Value) -> Value {
    match value {
        Value::Null => json!([]),
        Value::Array(items) => Value::Array(items.into_iter().map(normalise).collect()),
        Value::Object(items) => {
            Value::Object(items.into_iter().map(|(k, v)| (k, normalise(v))).collect())
        }
        value => value,
    }
}

#[test]
fn reference_goldens() {
    let mut count = 0;
    for line in include_str!("../../../tests/fixtures/reference.jsonl").lines() {
        let fixture: Value = serde_json::from_str(line).unwrap();
        let input = &fixture["input"];
        let function = text(&fixture["function"]);
        let actual = evaluate(function, input);
        // Adoption now preserves an unmanaged description, including with a commit message.
        let expected = if fixture["case"] == "TestGeneratePRBodyWithCommitMessage" {
            json!(
                "Old PR description from GitHub UI\n\n---\n<!-- git-pr-stack-start -->\n* ◻️ #101\n* ◻️ #102\n<!-- git-pr-stack-end -->"
            )
        } else {
            fixture["output"].clone()
        };
        assert_eq!(
            normalise(actual),
            normalise(expected),
            "{}: {function}",
            text(&fixture["case"])
        );
        count += 1;
    }
    assert!(count >= 312);
}

fn evaluate(function: &str, input: &Value) -> Value {
    match function {
        "matchWildcard" => json!(
            selection::DraftPatterns::new(&[text(&input["pattern"]).to_owned()])
                .unwrap()
                .matches(text(&input["text"]))
        ),
        "matchAnyPattern" => json!(
            selection::DraftPatterns::new(&strings(&input["patterns"]))
                .unwrap()
                .matches(text(&input["text"]))
        ),
        "parseCommaList" => json!(model::comma_list(text(input))),
        "formatKey" => json!(trailers::format_key(text(input))),
        "shortHash" => json!(crate::git::policy::short_hash(text(input))),
        "shortenTitle" => json!(model::shorten_title(text(input))),
        "generatePRBody" => json!(description::generate_body(
            &commit(&input["commit"]).message,
            text(&input["body"]),
            text(&input["stack"]),
            text(&input["template"])
        )),
        "generateStackInfo" => {
            let history: Vec<_> = array(&input["history"])
                .iter()
                .map(|h| HistoryEntry {
                    number: number(&h["Number"]),
                    merged: boolean(&h["IsMerged"]),
                })
                .collect();
            let merged: HashSet<_> = input["merged"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .filter(|(_, v)| boolean(v))
                .map(|(k, _)| k.parse::<u64>().unwrap())
                .collect();
            json!(crate::tests::legacy::generate_stack(
                &commits(&input["commits"]),
                &commit(&input["current"]),
                &history,
                &merged,
                Style {
                    stack_up: boolean(&input["reverse"]),
                    emoji: boolean(&input["emoji"]),
                    host: text(&input["host"]),
                    repo: text(&input["repo"])
                }
            ))
        }
        "extractPRHistoryFromStackInfo" => json!(
            description::extract_history(text(input))
                .iter()
                .map(|h| json!({"Number":h.number,"IsMerged":h.merged}))
                .collect::<Vec<_>>()
        ),
        "extractPRNumbersFromStackInfo" => json!(
            description::extract_history(text(input))
                .iter()
                .map(|h| h.number)
                .collect::<Vec<_>>()
        ),
        "formatPROutput" => json!(crate::tests::legacy::format_pr(
            text(&input["template"]),
            &commit(&input["commit"]),
            text(&input["host"]),
            text(&input["repo"])
        )),
        "dedupStrings" => json!(model::dedup(array(input).iter().map(text))),
        "selectPRNumberForHeadRef" => json!(github::policy::select_pr(
            &serde_json::from_value::<Vec<model::PullRequest>>(input["prs"].clone()).unwrap(),
            text(&input["reference"])
        )),
        "selectLocalBranchForCommit" => json!(
            selection::local_branch(
                &strings(&input["lines"]).join("\n"),
                &commit(&input["commit"]).hash
            )
            .unwrap_or_default()
        ),
        "remoteRefMatches" => json!(selection::remote_ref_matches(
            text(&input["output"]),
            text(&input["reference"]),
            text(&input["hash"])
        )),
        "decideRefSync" => json!(match selection::decide_ref_sync(
            text(&input["local"]),
            text(&input["pushed"]),
            boolean(&input["ff"]),
            boolean(&input["same"])
        ) {
            selection::RefSync::Nothing => 0,
            selection::RefSync::Move => 1,
            selection::RefSync::Unrelated => 2,
        }),
        "ghStackReason" => json!(crate::tests::legacy::stack_reason(text(input))),
        "isStackPartiallyUnstacked" => json!(text(input).contains("remain stacked")),
        "isBaseChangeBlockedByStack" => json!(text(input).contains("part of a stack")),
        "isGhStackMissing" | "isStackNeedsRebuild" | "isStackPushRejected" => {
            let error = text(&input["error"]);
            let combined = format!("{}{error}", text(&input["output"]));
            let expected = match function {
                "isGhStackMissing" => crate::tests::legacy::StackFailure::Missing,
                "isStackNeedsRebuild" => crate::tests::legacy::StackFailure::Rebuild,
                _ => crate::tests::legacy::StackFailure::PushRejected,
            };
            json!(
                !error.is_empty()
                    && crate::tests::legacy::classify_stack_error(&combined) == expected
            )
        }
        "parseLogs" => match crate::tests::legacy::parse_logs(text(input)) {
            Ok(v) => json!({"value":v.iter().map(commit_value).collect::<Vec<_>>(),"error":""}),
            Err(e) => json!({"value":null,"error":e.to_string()}),
        },
        "parseTrailers" => {
            let strings = strings(input);
            let (m, a) = trailers::parse(&strings.iter().map(String::as_str).collect::<Vec<_>>());
            json!({"message":m,"attrs":a})
        }
        "parseJJWorkingCopy" => match jj::policy::parse_working_copy(
            text(&input["check"]),
            text(&input["info"]),
            text(&input["user"]),
            text(&input["email"]),
        ) {
            Ok(v) => json!({"value":v.as_ref().map(commit_value),"error":""}),
            Err(e) => json!({"value":null,"error":e.to_string()}),
        },
        "resolveOutputFormat" => match output::resolve_format(text(input)) {
            Ok(v) => json!({"value":v,"error":""}),
            Err(e) => json!({"value":"","error":e.to_string()}),
        },
        "FullMessage" => json!(commit(input).full_message()),
        "orderSelection" => match selection::order_selection(
            &commits(&input["stack"]),
            &strings(&input["selected"]),
        ) {
            Ok((lo, hi)) => json!({"lo":lo,"hi":hi,"error":""}),
            Err(e) => json!({"lo":0,"hi":0,"error":e.to_string()}),
        },
        "findPrevNonSkipped" => crate::tests::legacy::previous_non_skipped(
            &commits(&input["stack"]),
            &commit(&input["target"]).hash,
        )
        .map_or(Value::Null, commit_value),
        "validateRemoteRefsBeforePush" => json!(selection::missing_refs(&commits(input))),
        "pickStackLeaf" => {
            let tips = strings(&input["tips"]);
            let leaf = selection::pick_stack_leaf(&tips, &|batch| {
                Ok(batch
                    .iter()
                    .filter(|a| {
                        !batch
                            .iter()
                            .any(|b| a != &b && boolean(&input["edges"][format!("{a}|{b}")]))
                    })
                    .cloned()
                    .collect())
            })
            .unwrap();
            json!({"leaf":leaf.as_deref().unwrap_or(""),"ok":leaf.is_some()||tips.is_empty()})
        }
        "resolveGitHubStackEnabled" => {
            let values = array(&input["settings"]);
            let mut options = values.iter().map(Value::as_bool);
            let cli = options.next().flatten();
            let env = options.next().flatten();
            let git = options.next().flatten();
            json!(crate::config::settings::github_stack(
                boolean(&input["no"]),
                cli,
                env.map(|b| if b { "1" } else { "0" }),
                git
            ))
        }
        "markUnselected" => {
            let mut cms = commits(&input["commits"]);
            let selected = input["selected"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, v)| boolean(v))
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>();
            let _ = selection::mark_unselected(&mut cms, &selected);
            json!(cms.iter().map(commit_value).collect::<Vec<_>>())
        }
        "recoverRangeTip" => {
            let trace = std::cell::RefCell::new(Vec::new());
            let result = selection::recover_tip(
                boolean(&input["jj"]),
                text(&input["id"]),
                text(&input["head"]),
                usize::try_from(number(&input["depth"])).unwrap(),
                |id| {
                    trace.borrow_mut().push(format!("change:{id}"));
                    if id == "id" {
                        anyhow::bail!("boom")
                    }
                    Ok("atipcommithash00000000000000000000000000".into())
                },
                |head, depth| {
                    trace.borrow_mut().push(format!("depth:{head}:{depth}"));
                    Ok("depthrecoveredhash0000000000000000000000".into())
                },
            );
            match result {
                Ok(value) => json!({"value":value,"error":"","trace":trace.into_inner()}),
                Err(e) => json!({"value":"","error":e.to_string(),"trace":trace.into_inner()}),
            }
        }
        other => panic!("Unmapped reference function: {other}"),
    }
}
