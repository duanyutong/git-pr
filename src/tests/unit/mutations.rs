//! Focused observable assertions for critical-policy mutation investigations.

use crate::{
    git::policy::protect_trunk,
    github::http::{HttpStatusError, Request, Response, Transport},
    github::{GitHub, Session},
    model::{Commit, REMOTE_REF},
    stack::selection,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use super::support::World;

fn commit(prefix: char, published: Option<&str>, skip: bool) -> Commit {
    Commit {
        hash: prefix.to_string().repeat(40),
        attrs: published.map_or_else(Vec::new, |reference| {
            vec![(REMOTE_REF.into(), reference.into())]
        }),
        skip,
        ..Commit::default()
    }
}

#[test]
fn missing_publication_refs_report_only_active_absences_in_stack_order() {
    let commits = [
        commit('1', Some("published/first"), false),
        commit('2', None, true),
        commit('3', None, false),
        commit('4', Some(""), false),
        commit('5', Some("published/last"), true),
        commit('6', Some(""), true),
    ];
    assert_eq!(selection::missing_refs(&commits), ["33333333", "44444444"]);
    assert_eq!(selection::missing_refs(&[]), Vec::<String>::new());
    assert_eq!(
        commits.iter().map(|commit| commit.skip).collect::<Vec<_>>(),
        [false, true, false, false, true, true]
    );
}

#[test]
fn an_unpublished_context_uses_the_trunk_and_a_published_context_retains_its_identity() {
    for context in [None, Some(""), Some("published/base|é")] {
        let commits = [
            commit('1', context, true),
            commit('2', Some("selected/first"), false),
            commit('3', Some("excluded/middle"), true),
            commit('4', Some("selected/last"), false),
        ];
        let expected_context = match context {
            None | Some("") => "release/stable",
            Some(reference) => reference,
        };
        assert_eq!(
            selection::resolve_bases(&commits, 1, "release/stable"),
            [
                "release/stable",
                expected_context,
                "selected/first",
                "selected/first",
            ]
        );
    }
}

#[test]
fn trunk_protection_uses_the_complete_literal_branch_identity() {
    let trunk = "release/stable";
    let error = protect_trunk(trunk, trunk).unwrap_err().to_string();
    assert!(error.contains("release/stable"));
    assert!(error.contains("separate PR branch"));
    for different in [
        "release",
        "release/stable-more",
        "release/stable/topic",
        "refs/heads/release/stable",
        "Release/stable",
        "release/stablé",
    ] {
        protect_trunk(different, trunk).unwrap();
    }
}

#[test]
fn configuration_status_one_with_a_diagnostic_is_never_treated_as_absence() {
    for key in [
        "git config --get git-pr.tags",
        "git config --type=bool --get git-pr.stackUp",
    ] {
        let world = World::new(1);
        world.reply(key, 1, "configuration diagnostic");
        let error = format!("{:#}", world.run(&[]).unwrap_err());
        assert!(
            error.contains("failed to read Git configuration"),
            "{error}"
        );
        assert!(error.contains("configuration diagnostic"), "{error}");
        assert!(!world.trace().iter().any(|event| {
            event["kind"] == "http" || (event["program"] == "git" && event["args"][0] == "push")
        }));
    }
}

#[derive(Debug)]
struct Replies(Mutex<VecDeque<Response>>, Mutex<Vec<(String, String)>>);

impl Transport for Replies {
    fn request(&self, request: Request<'_>) -> anyhow::Result<Vec<u8>> {
        Ok(self.response(request)?.body)
    }

    fn response(&self, request: Request<'_>) -> anyhow::Result<Response> {
        self.1
            .lock()
            .unwrap()
            .push((request.method.into(), request.url.into()));
        let response = self.0.lock().unwrap().pop_front().unwrap();
        if response.status >= 400 {
            return Err(HttpStatusError {
                url: request.url.into(),
                status: reqwest::StatusCode::from_u16(response.status).unwrap(),
                quota: crate::github::http::quota(&response.headers),
                body: String::from_utf8(response.body).unwrap(),
            }
            .into());
        }
        Ok(response)
    }
}

fn response(status: u16, body: &str, headers: &[(&'static str, &str)]) -> Response {
    Response {
        status,
        body: body.as_bytes().to_vec(),
        headers: headers
            .iter()
            .map(|(name, value)| {
                (
                    reqwest::header::HeaderName::from_static(name),
                    value.parse().unwrap(),
                )
            })
            .collect(),
    }
}

fn with_replies(replies: Vec<Response>, action: impl FnOnce(&GitHub<'_>, &World, &Replies)) {
    let world = World::new(1);
    let config = world.config();
    let commands = world.commands();
    let replies = Replies(Mutex::new(replies.into()), Mutex::default());
    let github = GitHub {
        config: &config,
        session: Session::default(),
        commands: &commands,
        transport: &replies,
        clock: Some(world.as_ref()),
        template: "",
    };
    action(&github, &world, &replies);
}

#[test]
fn positive_and_unknown_reset_quotas_do_not_delay_subsequent_reads() {
    for headers in [
        vec![
            ("x-ratelimit-remaining", "1"),
            ("x-ratelimit-reset", "9999999999"),
        ],
        vec![("x-ratelimit-remaining", "0")],
    ] {
        with_replies(
            vec![
                response(200, "first", &headers),
                response(200, "second", &[]),
            ],
            |github, world, replies| {
                assert_eq!(github.request("GET", "pulls/1", None).unwrap(), b"first");
                assert_eq!(github.request("GET", "pulls/2", None).unwrap(), b"second");
                assert!(!world.trace().iter().any(|event| event["kind"] == "wait"));
                assert_eq!(replies.0.lock().unwrap().len(), 0);
            },
        );
    }
}

#[test]
fn malformed_graphql_error_text_is_returned_without_a_throttle_retry() {
    let body = r#"{"errors": broken JSON"#;
    with_replies(vec![response(200, body, &[])], |github, world, replies| {
        assert_eq!(
            github
                .request(
                    "POST",
                    "graphql",
                    Some(&serde_json::json!({"query":"query { x }"}))
                )
                .unwrap(),
            body.as_bytes()
        );
        assert!(!world.trace().iter().any(|event| event["kind"] == "wait"));
        assert_eq!(replies.0.lock().unwrap().len(), 0);
    });
}

#[test]
fn secondary_wording_and_explicit_forbidden_retry_metadata_each_authorise_bounded_recovery() {
    for (limited, expected_wait) in [
        (response(403, "secondary rate exceeded", &[]), 60_000),
        (
            response(403, "quota unavailable", &[("retry-after", "2")]),
            2_000,
        ),
    ] {
        with_replies(
            vec![limited, response(200, "done", &[])],
            |github, world, replies| {
                assert_eq!(github.request("GET", "pulls/1", None).unwrap(), b"done");
                let waits: Vec<_> = world
                    .trace()
                    .iter()
                    .filter(|event| event["kind"] == "wait")
                    .map(|event| event["millis"].as_u64().unwrap())
                    .collect();
                assert_eq!(waits, [expected_wait]);
                assert_eq!(replies.0.lock().unwrap().len(), 0);
            },
        );
    }
}

#[test]
fn contradictory_open_and_merged_observations_cannot_authorise_a_publication_base() {
    with_replies(Vec::new(), |github, _, replies| {
        let observation = serde_json::json!({
            "number":101,"node_id":"PR_101","body":"human","title":"title",
            "labels":[],"state":"open","merged":true,"merged_at":null,
            "head":{"ref":"topic/1","repo":{"full_name":"org/repo"}},
            "base":{"ref":"main","repo":{"full_name":"org/repo"}}
        });
        replies
            .0
            .lock()
            .unwrap()
            .push_back(response(200, &observation.to_string(), &[]));
        github.get(101).unwrap();
        let commit = Commit {
            pr_number: 101,
            ..Commit::default()
        };
        assert!(!github.base_is_current(&commit, "main"));
        assert!(github.verify_base(&commit, "main").is_err());
        assert_eq!(replies.0.lock().unwrap().len(), 0);
    });
}

#[test]
fn an_ascii_stack_schema_error_retries_without_stack_fields_but_empty_errors_do_not() {
    for message in [Some("Field stack doesn't exist"), None] {
        let error = serde_json::json!({"errors": message.map_or_else(
            Vec::new,
            |message| vec![serde_json::json!({"message":message})],
        )});
        let empty = serde_json::json!({"data":{"repository":{"p0":{
            "nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}});
        with_replies(
            vec![
                response(200, &error.to_string(), &[]),
                response(200, &empty.to_string(), &[]),
            ],
            |github, _, replies| {
                let mut config = github.config.clone();
                config.github_stack = true;
                let github = GitHub {
                    config: &config,
                    session: Session::default(),
                    ..*github
                };
                let result = github.observe(&["topic/1".into()]);
                if message.is_some() {
                    result.unwrap();
                    assert_eq!(replies.0.lock().unwrap().len(), 0);
                } else {
                    assert!(result.is_err());
                    assert_eq!(replies.0.lock().unwrap().len(), 1);
                }
            },
        );
    }
}

#[test]
fn commit_association_access_failures_propagate_without_caching_absence() {
    for status in [401, 404] {
        with_replies(
            vec![
                response(status, "repository access is unavailable", &[]),
                response(200, "[]", &[]),
            ],
            |github, world, replies| {
                let commit = commit('1', None, false);
                let error = format!("{:#}", github.find(&commit).unwrap_err());
                assert!(error.contains(&status.to_string()), "{error}");
                assert!(
                    error.contains("repository access is unavailable"),
                    "{error}"
                );
                assert_eq!(replies.0.lock().unwrap().len(), 1);
                assert_eq!(github.find(&commit).unwrap(), 0);
                assert_eq!(replies.0.lock().unwrap().len(), 0);
                let requests = replies.1.lock().unwrap();
                assert_eq!(requests.len(), 2);
                assert!(requests.iter().all(|(method, url)| {
                    method == "GET"
                        && url.ends_with(&format!(
                            "/commits/{}/pulls?per_page=100&page=1",
                            commit.hash
                        ))
                }));
                drop(requests);
                assert!(!world.trace().iter().any(|event| event["kind"] == "wait"));
            },
        );
    }
}

#[derive(Debug)]
struct ConcurrentReplacement {
    world: Arc<World>,
    mutations: Mutex<Vec<serde_json::Value>>,
}

impl Transport for ConcurrentReplacement {
    fn request(&self, request: Request<'_>) -> anyhow::Result<Vec<u8>> {
        let payload: serde_json::Value = request
            .body
            .map_or(Ok(serde_json::Value::Null), serde_json::from_slice)?;
        if !payload["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("mutation"))
        {
            return self.world.request(request);
        }
        self.mutations.lock().unwrap().push(payload);
        // The original update completed, then another actor closed that PR and
        // opened a replacement on the same head before its acknowledgement arrived.
        self.world.request(request)?;
        {
            let mut memory = self.world.0.lock().unwrap();
            let original = memory.prs.get_mut(&101).unwrap();
            original["state"] = serde_json::json!("closed");
            let mut replacement = original.clone();
            replacement["number"] = serde_json::json!(201);
            replacement["node_id"] = serde_json::json!("PR_201");
            replacement["state"] = serde_json::json!("open");
            replacement["title"] = serde_json::json!("Replacement title");
            replacement["body"] = serde_json::json!("Replacement human notes");
            memory.prs.insert(201, replacement);
        }
        anyhow::bail!("lost acknowledgement after concurrent PR replacement")
    }
}

#[test]
fn content_recovery_never_replays_an_update_onto_a_replacement_pr_number() {
    let world = World::new(1);
    world.run(&["--no-stack"]).unwrap();
    let config = world.config();
    let commands = world.commands();
    let transport = ConcurrentReplacement {
        world: Arc::clone(&world),
        mutations: Mutex::default(),
    };
    let github = GitHub {
        config: &config,
        session: Session::default(),
        commands: &commands,
        transport: &transport,
        clock: Some(world.as_ref()),
        template: "",
    };
    let commit = Commit {
        pr_number: 101,
        title: "Desired title".into(),
        ..super::support::commit(1)
    };
    let error = format!(
        "{:#}",
        github
            .update_content(&[&commit], &mut |_, _| Ok(Some(
                "Desired managed body".into()
            )))
            .unwrap_err()
    );
    assert!(error.contains("identity or base changed"), "{error}");
    let attempted = transport.mutations.lock().unwrap();
    assert_eq!(attempted.len(), 1);
    assert_eq!(attempted[0]["variables"]["p0"]["pullRequestId"], "PR_101");
    drop(attempted);
    let memory = world.0.lock().unwrap();
    assert_eq!(memory.prs[&101]["state"], "closed");
    assert_eq!(memory.prs[&101]["body"], "Desired managed body");
    assert_eq!(memory.prs[&201]["title"], "Replacement title");
    assert_eq!(memory.prs[&201]["body"], "Replacement human notes");
    assert_eq!(memory.prs[&201]["head"]["ref"], "topic/1");
    drop(memory);
    assert!(!github.was_updated(201));
}

#[test]
fn inserting_a_middle_pr_rebuilds_native_membership_without_dropping_existing_members() {
    let world = World::new(3);
    world.run(&["--no-stack"]).unwrap();
    let mut config = world.config();
    config.auto_accept = true;
    let commands = world.commands();
    {
        let mut memory = world.0.lock().unwrap();
        // The middle PR already has the intended base relationships; native
        // membership still contains only the first and final PRs.
        for number in [101, 103] {
            memory.prs.get_mut(&number).unwrap()["stack"] = serde_json::json!({"number":9});
        }
        memory.trace.clear();
    }
    let before = world.0.lock().unwrap().prs.clone();
    let github = GitHub {
        config: &config,
        session: Session::default(),
        commands: &commands,
        transport: world.as_ref(),
        clock: Some(world.as_ref()),
        template: "",
    };
    let commits: Vec<_> = (1..=3)
        .map(|number| Commit {
            pr_number: 100 + number,
            ..super::support::commit(number)
        })
        .collect();
    github.manage_stack(&commits, "main").unwrap();
    let effects: Vec<_> = world
        .trace()
        .into_iter()
        .filter(|event| event["kind"] == "http" && event["method"] == "POST")
        .collect();
    let paths: Vec<_> = effects
        .iter()
        .map(|event| event["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["stacks/9/unstack", "graphql", "stacks"]);
    assert_eq!(
        effects.last().unwrap()["body"]["pull_requests"],
        serde_json::json!([101, 102, 103])
    );
    let after = world.0.lock().unwrap().prs.clone();
    let stack = after[&101]["stack"]["number"].as_u64().unwrap();
    assert_ne!(stack, 0);
    for number in [101, 102, 103] {
        assert_eq!(after[&number]["stack"]["number"], stack);
        let mut current = after[&number].clone();
        current.as_object_mut().unwrap().remove("stack");
        let mut original = before[&number].clone();
        original.as_object_mut().unwrap().remove("stack");
        assert_eq!(current, original);
    }
}
