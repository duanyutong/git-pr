//! Complete observations, cache ownership and partial service failures.
use super::{GitHub, Session};
use crate::{
    github::http::{Request, Response, Transport},
    tests::unit::support::{Response as Reply, World, commit},
};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn unpublished_commit_associations_accept_missing_commit_statuses_only() {
    for status in [404, 422] {
        with_github(|g, w| {
            let mut unpublished = commit(3);
            unpublished.attrs.clear();
            let path = format!("commits/{}/pulls", unpublished.hash);
            w.http(
                &format!("GET {path}"),
                vec![Reply::Output(
                    status,
                    json!({"message":format!("No commit found for SHA: {}", unpublished.hash),"status":status.to_string()}).to_string(),
                )],
            );
            assert_eq!(
                g.identity_heads(&unpublished.hash).unwrap(),
                Vec::<String>::new()
            );
            assert_eq!(g.find(&unpublished).unwrap(), 0);
            assert_eq!(
                w.trace()
                    .iter()
                    .filter(|event| event["path"] == path)
                    .count(),
                1
            );
        });
    }
    for (status, message) in [
        (404, "Not Found"),
        (422, "Validation Failed"),
        (401, "No commit found"),
        (403, "No commit found"),
    ] {
        with_github(|g, w| {
            let hash = commit(3).hash;
            let path = format!("commits/{hash}/pulls");
            w.http(
                &format!("GET {path}"),
                vec![
                    Reply::Output(status, json!({"message":message}).to_string()),
                    Reply::Output(200, "[]".into()),
                ],
            );
            assert!(g.identity_heads(&hash).is_err());
            assert_eq!(g.identity_heads(&hash).unwrap(), Vec::<String>::new());
            assert_eq!(
                w.trace()
                    .iter()
                    .filter(|event| event["path"] == path)
                    .count(),
                2
            );
        });
    }
}

#[test]
fn publication_dependants_require_complete_paginated_identity_observations() {
    with_github(|g, w| {
        let selected = std::collections::HashMap::from([(102, ("topic/2", "topic/1"))]);
        w.http(
            "POST graphql",
            vec![
                graph_reply(json!({"p0":connection(json!([]),Some("next"))})),
                graph_reply(
                    json!({"p0":connection(json!([{"number":102,"headRefName":"topic/2"}]),None)}),
                ),
            ],
        );
        w.0.lock().unwrap().trace.clear();
        g.check_publication_dependants(&["topic/1".into()], &selected)
            .unwrap();
        assert_eq!(w.trace().len(), 2);
        assert_eq!(w.trace()[1]["body"]["variables"]["c0"], "next");
        // An unselected dependant on a later page must still prevent publication.
        w.http(
            "POST graphql",
            vec![
                graph_reply(json!({"p0":connection(json!([]),Some("next"))})),
                graph_reply(
                    json!({"p0":connection(json!([{"number":999,"headRefName":"outside"}]),None)}),
                ),
            ],
        );
        assert!(
            g.check_publication_dependants(&["topic/1".into()], &selected)
                .unwrap_err()
                .to_string()
                .contains("#999")
        );
        for response in [
            Reply::Failure("unavailable".into()),
            Reply::Output(200, "not json".into()),
            Reply::Output(200, json!({"errors":[{"message":"denied"}]}).to_string()),
            graph_reply(json!({"p0":null})),
            graph_reply(
                json!({"p0":connection(json!([{"number":0,"headRefName":"topic/2"}]),None)}),
            ),
            graph_reply(json!({"p0":connection(json!([{"number":102,"headRefName":""}]),None)})),
            graph_reply(
                json!({"p0":connection(json!([{"number":102,"headRefName":"different"}]),None)}),
            ),
            graph_reply(json!({"p0":{"nodes":[]}})),
        ] {
            w.http("POST graphql", vec![response]);
            assert!(
                g.check_publication_dependants(&["topic/1".into()], &selected)
                    .is_err()
            );
        }
        let mut config = g.config.clone();
        config.repo.name = "invalid".into();
        let invalid = GitHub {
            config: &config,
            session: Session::default(),
            ..*g
        };
        assert!(
            invalid
                .check_publication_dependants(&[], &selected)
                .is_err()
        );
    });
}

#[test]
fn closed_observations_cannot_skip_base_checks_or_authorise_content_and_native_mutations() {
    with_github(|g, w| {
        assert!(g.publication_pr("unobserved").is_err());
        g.invalidate_heads(&["unobserved".into()]);
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["state"] = json!("closed");
        g.observe(&["topic/1".into()]).unwrap();
        let mut c = commit(1);
        c.pr_number = 101;
        assert!(!g.base_is_current(&c, "main"));
        w.0.lock().unwrap().trace.clear();
        assert!(g.verify_base(&c, "main").is_err());
        assert!(update_content(g, &[(101, "changed".into(), "changed".into())]).is_err());
        assert!(!w.trace().iter().any(|event| event["method"] == "PATCH"));
    });
}

#[test]
fn native_rebuild_refreshes_after_unstacking_and_never_retargets_changed_prs() {
    for failure in ["read", "closed", "base"] {
        with_github(|g, w| {
            for pr in w.0.lock().unwrap().prs.values_mut() {
                pr["stack"] = json!({"number":9});
            }
            w.http("GET stacks/9", vec![Reply::Output(200, json!({"number":9,"base":{"ref":"stale"},"pull_requests":[{"number":101},{"number":102}]}).to_string())]);
            let response = if failure == "read" {
                Reply::Failure("state unavailable".into())
            } else {
                let mut node = paged_node(101, "topic/1");
                node[if failure == "closed" {
                    "state"
                } else {
                    "baseRefName"
                }] = json!(if failure == "closed" {
                    "CLOSED"
                } else {
                    "changed"
                });
                graph_reply(
                    json!({"p0":connection(json!([node]),None),"p1":connection(json!([paged_node(102,"topic/2")]),None)}),
                )
            };
            w.http("POST graphql", vec![response]);
            let mut commits = [commit(1), commit(2)];
            commits[0].pr_number = 101;
            commits[1].pr_number = 102;
            w.0.lock().unwrap().trace.clear();
            assert!(g.manage_stack(&commits, "main").is_err());
            let trace = w.trace();
            assert!(
                trace
                    .iter()
                    .any(|event| event["path"] == "stacks/9/unstack")
            );
            assert!(
                !trace
                    .iter()
                    .any(|event| event["method"] == "PATCH" || event["path"] == "stacks")
            );
        });
    }
}

fn with_github(test: impl FnOnce(&GitHub<'_>, &Arc<World>)) {
    let world = World::new(2);
    world.run(&["--no-stack"]).unwrap();
    let mut config = world.config();
    config.auto_accept = true;
    let mut commands = world.commands();
    commands.verbose = true;
    commands.diagnostics = crate::diagnostics::Diagnostics::new(world.clone());
    let github = GitHub {
        session: Session::default(),
        clock: Some(world.as_ref()),
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "template",
    };
    test(&github, &world);
}

fn update_content(g: &GitHub<'_>, changes: &[(u64, String, String)]) -> anyhow::Result<()> {
    let commits: Vec<_> = changes
        .iter()
        .map(|(number, title, body)| crate::model::Commit {
            pr_number: *number,
            title: title.clone(),
            message: body.clone(),
            ..crate::model::Commit::default()
        })
        .collect();
    g.update_content(&commits.iter().collect::<Vec<_>>(), &mut |commit, _| {
        Ok(Some(commit.message.clone()))
    })
}

fn update_bases(
    g: &GitHub<'_>,
    changes: Vec<(crate::model::Commit, String)>,
) -> anyhow::Result<Vec<crate::model::Commit>> {
    g.verify_bases(changes.iter().map(|(c, b)| (c, b.as_str())).collect())?;
    Ok(changes.into_iter().map(|(c, _)| c).collect())
}

#[test]
fn label_reconciliation_uses_ascii_case_insensitive_identity_and_preserves_observed_names() {
    with_github(|g, world| {
        let mut config = g.config.clone();
        config.labels = vec!["bug".into(), "NEW".into(), "new".into()];
        config.tip_labels = vec!["BUG".into(), "Tip /LABEL".into(), "tip /label".into()];
        let g = GitHub {
            config: &config,
            session: Session::default(),
            clock: None,
            ..*g
        };
        world.0.lock().unwrap().prs.get_mut(&101).unwrap()["labels"] = json!([
            {"name": "Bug"}, "TIP /LABEL", "tip /Label", "Human"
        ]);
        g.observe(&["topic/1".into()]).unwrap();
        let mut commit = commit(1);
        commit.pr_number = 101;
        world.0.lock().unwrap().trace.clear();
        g.update_labels(&[(&commit, false)]).unwrap();
        let trace = world.trace();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0]["method"], "POST");
        assert_eq!(trace[0]["body"]["labels"], json!(["NEW"]));
        assert_eq!(trace[1]["method"], "DELETE");
        assert_eq!(trace[1]["path"], "issues/101/labels/tip%20%2FLabel");
        assert_eq!(
            g.get(101)
                .unwrap()
                .labels
                .iter()
                .map(crate::model::pull_request::Label::name)
                .collect::<Vec<_>>(),
            ["Bug", "Human", "NEW"]
        );
        world.0.lock().unwrap().trace.clear();
        g.update_labels(&[(&commit, false)]).unwrap();
        assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
        // A new session observes the service's spelling, not the submitted spelling.
        let g = GitHub {
            session: Session::default(),
            ..g
        };
        world.0.lock().unwrap().prs.get_mut(&101).unwrap()["labels"] =
            json!(["BUG", "New", "Human"]);
        g.observe(&["topic/1".into()]).unwrap();
        world.0.lock().unwrap().trace.clear();
        g.update_labels(&[(&commit, false)]).unwrap();
        assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
    });
}

#[test]
fn immutable_snapshots_and_unchanged_descriptions_share_storage() {
    with_github(|g, w| {
        let footer = "* #101";
        let mut seeded = g.get(101).unwrap();
        Arc::make_mut(&mut seeded).body = format!(
            "{}\n\n{}\n{footer}\n{}",
            "Human notes ".repeat(5000),
            crate::stack::description::START,
            crate::stack::description::END
        )
        .into();
        g.remember(seeded);
        let before = g.get(101).unwrap();
        assert!(Arc::ptr_eq(&before, &g.get(101).unwrap()));
        let source = Arc::clone(&before.body);
        let owners = Arc::strong_count(&source);
        let mut bodies = crate::stack::description::BodyCache::default();
        assert!(
            bodies
                .get_shared(101, &source)
                .unwrap()
                .render_update("", footer, "")
                .unwrap()
                .is_none()
        );
        assert_eq!(Arc::strong_count(&source), owners + 1);
        let mut target = commit(1);
        target.pr_number = 101;
        let mut render = |c: &crate::model::Commit, pr: &crate::model::PullRequest| {
            bodies
                .get_shared(pr.number, &pr.body)?
                .render_update(&c.message, footer, "")
        };
        w.0.lock().unwrap().trace.clear();
        g.update_content(&[&target], &mut render).unwrap();
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
        assert!(Arc::ptr_eq(&before, &g.get(101).unwrap()));
        assert_eq!(Arc::strong_count(&source), owners + 1);
        target.title = "Renamed PR".into();
        g.update_content(&[&target], &mut render).unwrap();
        let renamed = g.get(101).unwrap();
        assert!(!Arc::ptr_eq(&before, &renamed));
        assert_eq!(before.title, "Feature 1");
        assert_eq!(renamed.title, "Renamed PR");
        assert!(Arc::ptr_eq(&source, &renamed.body));
        let edited = renamed.body.replacen("Human notes", "Edited notes", 1);
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] = json!(edited);
        g.expire_pr_observation_for_test(101);
        g.update_content(&[&target], &mut render).unwrap();
        let refreshed = g.get(101).unwrap();
        assert!(refreshed.body.starts_with("Edited notes"));
        assert!(before.body.starts_with("Human notes"));
        assert!(!Arc::ptr_eq(&source, &refreshed.body));
    });
}

#[test]
fn label_freshness_checks_scale_with_targets_and_expiry_retains_batching() {
    use crate::tests::unit::support::{LABEL_FRESHNESS_CHECKS, PR_REFRESH_PROBES};
    for expired in [0, 1, 51] {
        let w = World::new(100);
        w.run(&["--no-stack"]).unwrap();
        let mut config = w.config();
        config.labels = vec!["ready".into()];
        let commands = w.commands();
        let g = GitHub {
            config: &config,
            session: Session::default(),
            clock: Some(w.as_ref()),
            commands: &commands,
            transport: w.as_ref(),
            template: "",
        };
        for pr in w.0.lock().unwrap().prs.values_mut() {
            pr["labels"] = json!(["ready"]);
            g.observed(serde_json::from_value(pr.clone()).unwrap());
        }
        let commits: Vec<_> = (1..=100)
            .map(|i| {
                let mut c = commit(i);
                c.pr_number = 100 + i;
                c
            })
            .collect();
        for number in 101..101 + expired {
            g.expire_pr_observation_for_test(number);
        }
        LABEL_FRESHNESS_CHECKS.set(0);
        PR_REFRESH_PROBES.set(0);
        w.0.lock().unwrap().trace.clear();
        g.update_labels(&commits.iter().map(|c| (c, false)).collect::<Vec<_>>())
            .unwrap();
        assert_eq!(LABEL_FRESHNESS_CHECKS.get(), 100);
        let expected = usize::try_from(expired.div_ceil(25)).unwrap();
        assert_eq!(PR_REFRESH_PROBES.get(), expected * 25);
        let requests: Vec<_> = w
            .trace()
            .into_iter()
            .filter(|v| v["kind"] == "http")
            .collect();
        assert_eq!(requests.len(), expected);
        assert!(
            requests.iter().all(|v| v["path"] == "graphql"
                && v["body"]["query"].as_str().unwrap().starts_with("query"))
        );
    }
}

#[test]
fn cache_identity_and_content_completion_are_explicit() {
    with_github(|g, w| {
        assert_eq!(g.get(101).unwrap().number, 101);
        assert_eq!(g.get(101).unwrap().number, 101);
        w.http("GET pulls/103", vec![Reply::Output(200, "{}".into())]);
        assert!(g.get(103).is_err());
        update_content(g, &[]).unwrap();
        let mut pr = g.get(101).unwrap();
        Arc::make_mut(&mut pr).node_id.clear();
        g.remember(pr);
        update_content(g, &[(101, "new title".into(), "new body".into())]).unwrap();
        assert_eq!(g.get(101).unwrap().body.as_ref(), "new body");
        g.get(102).unwrap();
        w.0.lock().unwrap().trace.clear();
        w.http("POST graphql",vec![Reply::Output(200,json!({"data":{"p0":{"pullRequest":{"number":102}}},"errors":[{"message":"partial"}]}).to_string())]);
        update_content(g, &[(102, "known completion".into(), "body".into())]).unwrap();
        assert_eq!(
            w.trace()
                .iter()
                .filter(|event| event["kind"] == "http")
                .count(),
            1
        );
        assert_eq!(g.get(102).unwrap().title, "known completion");
        w.http(
            "POST graphql",
            vec![Reply::Output(200, json!({"data":{}}).to_string())],
        );
        assert!(update_content(g, &[(102, "unknown completion".into(), "body".into())]).is_err());
        assert_eq!(g.get(102).unwrap().title, "known completion");
    });
}

#[test]
fn rest_description_progress_counts_success_and_preserves_the_failed_target() {
    use crate::tests::unit::live_progress::{Progress, Recorder};
    with_github(|g, world| {
        let recorder = Recorder::new(world.clone());
        let mut commands = g.commands.clone();
        commands.terminal = recorder.clone();
        let github = GitHub {
            session: Session::default(),
            commands: &commands,
            ..*g
        };
        // Retain the existing defensive REST fallback for cached observations
        // without GraphQL identity; wire observations must still be complete.
        for number in 101..=102 {
            let mut pr = github.get(number).unwrap();
            Arc::make_mut(&mut pr).node_id.clear();
            github.remember(pr);
        }
        world.http("PATCH pulls/102", vec![Reply::Output(403, "denied".into())]);
        let error = update_content(
            &github,
            &[
                (101, "Updated one".into(), "body".into()),
                (102, "Updated two".into(), "body".into()),
            ],
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("no unsafe replay"));
        assert_eq!(
            recorder.events.lock().unwrap().last(),
            Some(&Progress {
                label: "Descriptions #102".into(),
                done: 1,
                total: 2
            })
        );
        assert!(github.was_updated(101));
        assert!(!github.was_updated(102));
    });
}

#[test]
fn malformed_observations_and_foreign_repository_heads_are_not_absence() {
    for body in [
        json!({"errors":[{"message":"rate limited"}]}),
        json!({"errors":[{}]}),
        json!({"errors":"invalid"}),
        json!({"data":{"repository":{"p0":{"nodes":[],"pageInfo":{}}}}}),
    ] {
        with_github(|g, w| {
            w.http("POST graphql", vec![Reply::Output(200, body.to_string())]);
            assert!(g.observe(&["topic/1".into()]).is_err());
        });
    }
    with_github(|g, w| {
        w.http("POST graphql",vec![Reply::Output(200,json!({"data":{"repository":{"p0":{"nodes":[{"headRepository":{"nameWithOwner":"fork/repo"},"labels":{"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
        assert_eq!(g.find_head("topic/1", "all").unwrap(), 0);
    });
}

#[test]
fn topology_reconciliation_appends_and_protects_unselected_members() {
    with_github(|g, w| {
        let mut first = g.get(101).unwrap();
        Arc::make_mut(&mut first).stack =
            Some(crate::model::pull_request::NativeStack { number: 9 });
        g.remember(first);
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 102;
        g.manage_stack(&commits, "main").unwrap();
        assert!(w.trace().iter().any(|v| v["path"] == "stacks/9/add"));
        let mut second = g.get(102).unwrap();
        Arc::make_mut(&mut second).stack =
            Some(crate::model::pull_request::NativeStack { number: 10 });
        g.remember(second);
        assert!(g.manage_stack(&commits, "main").is_err());
        commits[1].pr_number = 0;
        assert!(g.manage_stack(&commits, "main").is_err());
    });
    with_github(|g, w| {
        let mut first = g.get(101).unwrap();
        Arc::make_mut(&mut first).stack =
            Some(crate::model::pull_request::NativeStack { number: 9 });
        g.remember(first);
        w.http("GET stacks/9",vec![Reply::Output(200,json!({"number":9,"base":{"ref":"stale"},"pull_requests":[{"number":101},{"number":102},{"number":103}]}).to_string())]);
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 102;
        assert!(
            g.manage_stack(&commits, "main")
                .unwrap_err()
                .to_string()
                .contains("unselected")
        );
    });
}

#[test]
fn native_topology_validation_prevents_mutation_of_unverified_stacks() {
    for (body, expected) in [
        (
            json!({"number":10,"base":{"ref":"main"},"pull_requests":[{"number":101}]}),
            "instead of requested #9",
        ),
        (
            json!({"number":0,"base":{"ref":"main"},"pull_requests":[{"number":101}]}),
            "instead of requested #9",
        ),
        (
            json!({"number":9,"base":{},"pull_requests":[{"number":101}]}),
            "base or member list is empty",
        ),
        (
            json!({"number":9,"base":{"ref":"main"},"pull_requests":[]}),
            "base or member list is empty",
        ),
        (
            json!({"number":9,"base":{"ref":"main"},"pull_requests":[{"number":0}]}),
            "must be non-zero and unique",
        ),
        (
            json!({"number":9,"base":{"ref":"main"},"pull_requests":[{"number":101},{"number":101}]}),
            "must be non-zero and unique",
        ),
        (
            json!({"number":9,"base":{"ref":"main"},"pull_requests":[{"number":102}]}),
            "claims membership but is missing",
        ),
    ] {
        with_github(|g, w| {
            let mut first = g.get(101).unwrap();
            Arc::make_mut(&mut first).stack =
                Some(crate::model::pull_request::NativeStack { number: 9 });
            g.remember(first);
            w.http("GET stacks/9", vec![Reply::Output(200, body.to_string())]);
            let mut commits = [commit(1), commit(2)];
            commits[0].pr_number = 101;
            commits[1].pr_number = 102;
            w.0.lock().unwrap().trace.clear();
            let error = format!("{:#}", g.manage_stack(&commits, "main").unwrap_err());
            assert!(error.contains(expected), "{error}");
            assert!(
                !w.trace()
                    .iter()
                    .any(|event| { event["kind"] == "http" && event["method"] != "GET" })
            );
            assert!(!w.0.lock().unwrap().stdout.contains("GitHub Stack updated"));
        });
    }
    with_github(|g, w| {
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 101;
        w.0.lock().unwrap().trace.clear();
        assert!(
            g.manage_stack(&commits, "main")
                .unwrap_err()
                .to_string()
                .contains("duplicate PR identities")
        );
        assert!(!w.trace().iter().any(|event| event["kind"] == "http"));
        commits[1].pr_number = 102;
        let mut first = g.get(101).unwrap();
        Arc::make_mut(&mut first).stack =
            Some(crate::model::pull_request::NativeStack { number: 0 });
        g.remember(first);
        w.0.lock().unwrap().trace.clear();
        assert!(
            g.manage_stack(&commits, "main")
                .unwrap_err()
                .to_string()
                .contains("invalid native stack identity #0")
        );
        assert!(
            !w.trace()
                .iter()
                .any(|event| event["kind"] == "http" && event["method"] != "GET")
        );
    });
}

fn poison<T: Send>(mutex: &Mutex<T>) {
    std::thread::scope(|scope| {
        let _ = scope
            .spawn(|| {
                let _guard = mutex.lock().unwrap();
                panic!("injected owner failure");
            })
            .join();
    });
}
#[test]
fn cache_values_survive_worker_unwinding_without_incomplete_observations() {
    with_github(|g, _| {
        g.observe(&["topic/1".into()]).unwrap();
        poison(&g.session.heads);
        poison(&g.session.prs);
        poison(&g.session.dispatch);
        assert_eq!(g.find_head("topic/1", "open").unwrap(), 101);
        assert_eq!(g.get(101).unwrap().number, 101);
        g.request("GET", "pulls/101", None).unwrap();
    });
}

#[derive(Debug)]
struct RateLimit {
    attempts: Mutex<usize>,
}
impl Transport for RateLimit {
    fn request(&self, _: Request<'_>) -> anyhow::Result<Vec<u8>> {
        unreachable!("response owns this fixture")
    }
    fn response(&self, request: Request<'_>) -> anyhow::Result<Response> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts <= 2 {
            return Err(crate::github::http::HttpStatusError {
                url: request.url.into(),
                status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                body: "limited".into(),
                quota: crate::github::limits::Quota {
                    retry_after: Some(std::time::Duration::from_secs(2)),
                    ..Default::default()
                },
            }
            .into());
        }
        drop(attempts);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-github-request-id",
            reqwest::header::HeaderValue::from_static("fixture"),
        );
        Ok(Response {
            status: 200,
            headers,
            body: b"{}".to_vec(),
        })
    }
}
#[test]
fn explicit_rate_limits_are_bounded_and_mutation_dispatch_is_paced() {
    with_github(|g, w| {
        let transport = RateLimit {
            attempts: Mutex::new(0),
        };
        let limited = GitHub {
            session: Session::default(),
            clock: g.clock,
            config: g.config,
            commands: g.commands,
            transport: &transport,
            template: "",
        };
        limited
            .request("PATCH", "pulls/101", Some(&json!({})))
            .unwrap();
        limited
            .request("PATCH", "pulls/101", Some(&json!({})))
            .unwrap();
        assert_eq!(*transport.attempts.lock().unwrap(), 4);
        let stderr = w.0.lock().unwrap().stderr.clone();
        assert_eq!(
            stderr
                .matches("start wait: GitHub rate-limit retry; retry")
                .count(),
            2
        );
        assert!(stderr.contains("start wait: GitHub mutation spacing"));
        assert!(stderr.contains("attempt 3"));
        assert!(stderr.contains("HTTP 429"));
        assert!(
            w.0.lock()
                .unwrap()
                .stderr
                .contains("GitHub request fixture")
        );
    });
}

#[test]
fn stale_cache_is_refreshed_before_planning_human_content() {
    with_github(|g, w| {
        let g = GitHub {
            session: Session::default(),
            clock: g.clock,
            config: g.config,
            commands: g.commands,
            transport: g.transport,
            template: "",
        };
        g.observe(&["topic/1".into()]).unwrap();
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] = json!("Human edit");
        g.session.head_times.lock().unwrap().insert(
            "topic/1".into(),
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(31))
                .unwrap(),
        );
        g.refresh_observations(&["topic/1".into()]).unwrap();
        assert_eq!(g.get(101).unwrap().body.as_ref(), "Human edit");
    });
}

#[test]
fn metadata_failures_preserve_known_effects_and_block_later_work() {
    with_github(|g, w| {
        w.http("GET pulls/103", vec![Reply::Output(200, "not json".into())]);
        assert!(g.get(103).is_err());
        let mut c = commit(1);
        c.attrs.clear();
        w.http(
            &format!("GET commits/{}/pulls", c.hash),
            vec![Reply::Output(
                200,
                json!([{"number":101,"state":"open","head":{"ref":"topic/1","repo":{"full_name":"org/repo"}}}]).to_string(),
            )],
        );
        assert_eq!(g.find(&c).unwrap(), 101);
        c.skip = true;
        assert_eq!(g.number(&mut c, "main").unwrap(), 0);
        w.http("POST pulls", vec![Reply::Failure("create failed".into())]);
        w.http(
            "POST graphql",
            vec![Reply::Failure("observation failed".into())],
        );
        assert!(g.create(&mut commit(1), "main").is_err());
        assert!(update_content(g, &[(103, "t".into(), "b".into())]).is_err());
        let mut pr = g.get(101).unwrap();
        Arc::make_mut(&mut pr).node_id.clear();
        g.remember(pr);
        w.http(
            "PATCH pulls/101",
            vec![Reply::Failure("patch failed".into())],
        );
        assert!(update_content(g, &[(101, "t".into(), "b".into())]).is_err());
        assert!(update_content(g, &[(102, "t".into(), "b".into())]).is_err());
        w.http("POST graphql", vec![Reply::Output(200, "not json".into())]);
        assert!(update_content(g, &[(102, "t".into(), "b".into())]).is_err());
        c.pr_number = 103;
        assert!(g.verify_base(&c, "main").is_err());
        let mut pr = g.get(101).unwrap();
        Arc::make_mut(&mut pr).stack = Some(crate::model::pull_request::NativeStack { number: 9 });
        g.remember(pr);
        c.pr_number = 101;
        c.skip = false;
        assert!(g.verify_base(&c, "new-base").is_err());
        c.base_blocked = true;
        g.warn_blocked(std::slice::from_ref(&c));
        c.pr_number = 0;
        g.warn_blocked(std::slice::from_ref(&c));
        c.skip = true;
        g.warn_blocked(&[c]);
        assert!(w.0.lock().unwrap().stderr.contains("(unknown)"));
    });
}

#[test]
fn rest_and_graphql_values_require_valid_documents_and_complete_labels() {
    for labels in [false, true] {
        for reply in [
            Reply::Failure("page lookup failed".into()),
            Reply::Output(200, "not json".into()),
        ] {
            with_github(|g, w| {
                let mut node = paged_node(101, "topic/1");
                if labels {
                    node["labels"] = connection(json!([]), Some("label-page"));
                }
                let initial =
                    connection(json!([node]), if labels { None } else { Some("head-page") });
                w.http(
                    "POST graphql",
                    vec![graph_reply(json!({"p0":initial})), reply],
                );
                w.0.lock().unwrap().trace.clear();
                assert!(g.observe(&["topic/1".into()]).is_err());
                assert_eq!(w.trace().len(), 2);
                assert!(g.session.heads().is_empty());
            });
        }
    }
    with_github(|g, w| {
        w.http(
            "POST graphql",
            vec![Reply::Output(
                200,
                json!({"errors":[{"message":"Field stack doesn’t exist"}]}).to_string(),
            )],
        );
        assert!(g.observe(&["topic/1".into()]).is_err());
    });
    with_github(|g, w| {
        w.http("POST graphql",vec![Reply::Output(200,json!({"data":{"repository":{"p0":{"nodes":[{"number":"invalid","headRepository":{"nameWithOwner":"org/repo"},"labels":{"pageInfo":{"hasNextPage":false}}}],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
        assert!(g.observe(&["topic/1".into()]).is_err());
        let mut config = g.config.clone();
        config.repo.name = "invalid".into();
        let bad = GitHub {
            session: Session::default(),
            clock: g.clock,
            config: &config,
            commands: g.commands,
            transport: g.transport,
            template: "",
        };
        assert!(bad.observe(&[]).is_err());
    });
    for (body, merged) in [
        (
            json!({"body":null,"merged_at":"2026-01-01T00:00:00Z"}),
            true,
        ),
        (json!({"body":"","merged":false}), false),
    ] {
        let pr: crate::model::PullRequest = serde_json::from_value(body).unwrap();
        assert_eq!(pr.is_merged(), merged);
    }
    assert!(serde_json::from_value::<crate::model::PullRequest>(json!({"body":12})).is_err());
    assert_eq!(
        crate::model::pull_request::Label::Name("name".into()).name(),
        "name"
    );
}

#[test]
fn creating_a_ready_pr_and_topology_service_failures_are_explicit() {
    with_github(|g, _| {
        let mut config = g.config.clone();
        config.draft = false;
        let ready = GitHub {
            session: Session::default(),
            clock: g.clock,
            config: &config,
            commands: g.commands,
            transport: g.transport,
            template: "",
        };
        assert_eq!(ready.number(&mut commit(3), "topic/2").unwrap(), 103);
    });
    for (path, reply) in [
        ("GET pulls/102", Reply::Failure("read failed".into())),
        (
            "GET stacks/9",
            Reply::Failure("topology unavailable".into()),
        ),
        ("GET stacks/9", Reply::Output(200, "invalid".into())),
        (
            "POST stacks/9/add",
            Reply::Failure("append rejected".into()),
        ),
    ] {
        with_github(|g, w| {
            w.0.lock().unwrap().prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
            w.http(path, vec![reply]);
            let mut commits = [commit(1), commit(2)];
            commits[0].pr_number = 101;
            commits[1].pr_number = 102;
            assert!(g.manage_stack(&commits, "main").is_err());
        });
    }
    for path in ["PATCH pulls/101", "POST stacks"] {
        with_github(|g, w| {
            {
                let mut m = w.0.lock().unwrap();
                let pr = m.prs.get_mut(&101).unwrap();
                pr["stack"] = json!({"number":9});
                pr["base"]["ref"] = json!("stale");
                drop(m);
            }
            w.http(path, vec![Reply::Failure("rebuild rejected".into())]);
            let mut commits = [commit(1), commit(2)];
            commits[0].pr_number = 101;
            commits[1].pr_number = 102;
            assert!(g.manage_stack(&commits, "main").is_err());
        });
    }
    with_github(|g, w| {
        {
            let mut m = w.0.lock().unwrap();
            let pr = m.prs.get_mut(&101).unwrap();
            pr["stack"] = json!({"number":9});
            pr["base"]["ref"] = json!("topic/0");
            drop(m);
        }
        w.http("GET stacks/9",vec![Reply::Output(200,json!({"number":9,"base":{"ref":"main"},"pull_requests":[{"number":100},{"number":101},{"number":102}]}).to_string())]);
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 102;
        g.manage_stack(&commits, "topic/0").unwrap();
        assert!(w.0.lock().unwrap().stdout.contains("Stack unchanged"));
    });
}

#[test]
fn missing_or_inconsistent_graphql_identities_cannot_authorise_creation() {
    let valid = json!({"number":101,"id":"PR_101","title":"T","body":null,"state":"OPEN","merged":false,"headRefName":"topic/1","baseRefName":"main","headRepository":{"nameWithOwner":"org/repo"},"labels":{"nodes":[],"pageInfo":{"hasNextPage":false}},"stack":null});
    for (key, value) in [
        ("headRepository", json!({})),
        ("number", json!(0)),
        ("title", json!(null)),
        ("id", json!("")),
        ("headRefName", json!("other")),
        ("state", json!("unknown")),
        ("absent-body", json!(null)),
        ("absent-title", json!(null)),
        ("absent-merged", json!(null)),
        ("absent-repository", json!(null)),
    ] {
        with_github(|g, w| {
            let mut node = valid.clone();
            if let Some(field) = key
                .strip_prefix("absent-")
                .filter(|field| *field != "repository")
            {
                node.as_object_mut().unwrap().remove(field);
            } else if key == "absent-repository" {
                node.as_object_mut().unwrap().remove("headRepository");
            } else {
                node[key] = value;
            }
            w.http("POST graphql",vec![Reply::Output(200,json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
            assert!(g.observe(&["topic/1".into()]).is_err());
        });
    }
    with_github(|g, w| {
        let mut node = valid.clone();
        node["headRepository"]["nameWithOwner"] = json!("ORG/Repo");
        w.http("POST graphql", vec![Reply::Output(200, json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
        assert_eq!(g.find_head("topic/1", "open").unwrap(), 101);
    });
    with_github(|g, w| {
        let mut node = valid;
        node["headRepository"] = json!(null);
        w.http("POST graphql",vec![Reply::Output(200,json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}}).to_string())]);
        assert_eq!(g.find_head("topic/1", "all").unwrap(), 0);
    });
}

#[test]
fn declined_rebuild_never_reports_completion_and_keeps_stale_base_warning() {
    with_github(|g, w| {
        let mut config = g.config.clone();
        config.auto_accept = false;
        let github = GitHub {
            config: &config,
            session: Session::default(),
            ..*g
        };
        {
            let mut memory = w.0.lock().unwrap();
            memory.stdout.clear();
            memory.stderr.clear();
            memory.answers.push_back(false);
            for pr in memory.prs.values_mut() {
                pr["stack"] = json!({"number":9});
            }
        }
        w.http("GET stacks/9", vec![Reply::Output(200, json!({"number":9,"base":{"ref":"stale"},"pull_requests":[{"number":101},{"number":102}]}).to_string())]);
        let mut commits = [commit(1), commit(2)];
        for (i, c) in commits.iter_mut().enumerate() {
            c.pr_number = 101 + i as u64;
            c.base_blocked = true;
        }
        assert_eq!(
            github.manage_stack(&commits, "main").unwrap(),
            super::native::StackOutcome::Declined
        );
        let memory = w.0.lock().unwrap();
        assert!(memory.stdout.contains("skipped GitHub Stack rebuild"));
        assert!(!memory.stdout.contains("GitHub Stack updated"));
        assert!(memory.stderr.contains("base is stale"));
        drop(memory);
        assert_eq!(
            github.manage_stack(&commits[..1], "main").unwrap(),
            super::native::StackOutcome::Unchanged
        );
    });
}

#[test]
fn observation_age_preserves_fresh_entries_and_partial_mutations_do_not_renew_them() {
    with_github(|g, w| {
        g.observe(&["topic/1".into(), "topic/2".into()]).unwrap();
        let old = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(31))
            .unwrap();
        g.session
            .head_times
            .lock()
            .unwrap()
            .insert("topic/1".into(), old);
        g.session.pr_times.lock().unwrap().insert(101, old);
        let pr = g.session.prs().get(&101).unwrap().clone();
        g.remember(pr);
        assert_eq!(g.session.pr_times.lock().unwrap()[&101], old);
        w.0.lock().unwrap().trace.clear();
        g.refresh_observations(&["topic/1".into(), "topic/2".into()])
            .unwrap();
        g.refresh_observations(&["topic/1".into(), "topic/2".into()])
            .unwrap();
        let queries: Vec<_> = w
            .trace()
            .into_iter()
            .filter(|v| v["path"] == "graphql")
            .collect();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0]["body"]["variables"]["h0"], "topic/1");
        assert!(queries[0]["body"]["variables"].get("h1").is_none());
        g.session.pr_times.lock().unwrap().insert(102, old);
        w.0.lock().unwrap().prs.get_mut(&102).unwrap()["body"] = json!("fresh human content");
        assert_eq!(g.get(102).unwrap().body.as_ref(), "fresh human content");
    });
}

#[test]
fn content_batches_respect_encoded_bytes_and_reject_an_oversized_pr() {
    with_github(|g, w| {
        let body = "\"\\\n🦀".repeat(60_000);
        w.0.lock().unwrap().trace.clear();
        update_content(
            g,
            &[(101, "one".into(), body.clone()), (102, "two".into(), body)],
        )
        .unwrap();
        let requests: Vec<_> = w
            .trace()
            .into_iter()
            .filter(|v| v["path"] == "graphql")
            .collect();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|r| serde_json::to_vec(&r["body"]).unwrap().len() <= super::CONTENT_LIMIT)
        );
        let oversized = "x".repeat(super::CONTENT_LIMIT);
        assert!(
            update_content(g, &[(101, "large".into(), oversized.clone())])
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
        let mut pr = g.get(101).unwrap();
        Arc::make_mut(&mut pr).node_id.clear();
        g.remember(pr);
        assert!(update_content(g, &[(101, "large".into(), oversized)]).is_err());
    });
}

#[test]
fn content_request_boundaries_reuse_preparation_only_for_unchanged_observations() {
    for expiry in [false, true] {
        for edit in [false, true] {
            let world = World::new(25);
            world.run(&["--no-stack"]).unwrap();
            let config = world.config();
            let commands = world.commands();
            let github = GitHub {
                session: Session::default(),
                clock: Some(world.as_ref()),
                config: &config,
                commands: &commands,
                transport: world.as_ref(),
                template: "",
            };
            let commits: Vec<_> = (1..=25)
                .map(|n| {
                    let mut c = commit(n);
                    c.pr_number = 100 + n;
                    c
                })
                .collect();
            let mut counts = std::collections::BTreeMap::<u64, usize>::new();
            let large = "x".repeat(300_000);
            world.0.lock().unwrap().trace.clear();
            crate::tests::unit::support::CONTENT_ENCODINGS.set(0);
            github
                .update_content(&commits.iter().collect::<Vec<_>>(), &mut |c, pr| {
                    let count = counts.entry(c.pr_number).or_default();
                    *count += 1;
                    if c.pr_number == 104 && *count == 1 && expiry {
                        github.expire_pr_observation_for_test(104);
                        if edit {
                            world.0.lock().unwrap().prs.get_mut(&104).unwrap()["body"] =
                                json!("Refreshed human notes");
                        }
                    }
                    Ok(Some(if pr.body.as_ref() == "Refreshed human notes" {
                        format!("{}\n{large}", pr.body)
                    } else {
                        large.clone()
                    }))
                })
                .unwrap();
            assert_eq!(
                counts.values().sum::<usize>(),
                25 + usize::from(expiry && edit)
            );
            assert_eq!(counts[&104], 1 + usize::from(expiry && edit));
            assert_eq!(
                crate::tests::unit::support::CONTENT_ENCODINGS.get(),
                25 + usize::from(expiry && edit)
            );
            assert_eq!(
                github
                    .get(104)
                    .unwrap()
                    .body
                    .starts_with("Refreshed human notes"),
                expiry && edit
            );
            let trace = world.trace();
            let writes: Vec<_> = trace
                .iter()
                .filter(|event| {
                    event["path"] == "graphql"
                        && event["body"]["query"]
                            .as_str()
                            .unwrap_or("")
                            .starts_with("mutation")
                })
                .collect();
            assert_eq!(writes.len(), 9);
            assert!(
                writes
                    .iter()
                    .all(|event| serde_json::to_vec(&event["body"]).unwrap().len()
                        <= super::CONTENT_LIMIT)
            );
            let changed = Arc::clone(&github.get(104).unwrap().body);
            world.0.lock().unwrap().trace.clear();
            crate::tests::unit::support::CONTENT_ENCODINGS.set(0);
            github
                .update_content(&commits.iter().collect::<Vec<_>>(), &mut |c, _| {
                    Ok(Some(if c.pr_number == 104 {
                        changed.to_string()
                    } else {
                        large.clone()
                    }))
                })
                .unwrap();
            assert_eq!(world.trace(), [] as [serde_json::Value; 0]);
        }
    }
}

#[test]
fn description_mutations_render_from_refreshed_human_content_and_skip_noops() {
    with_github(|g, w| {
        let old = g.get(101).unwrap();
        crate::tests::unit::support::BODY_ANALYSES.set(0);
        let mut bodies = crate::stack::description::BodyCache::default();
        bodies.get(101, &old.body).unwrap().history();
        assert_eq!(crate::tests::unit::support::BODY_ANALYSES.get(), 1);
        let edited = old.body.replacen("Body 1", "New human text", 1);
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] = json!(edited);
        super::guard(&g.session.pr_times).insert(
            101,
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(31))
                .unwrap(),
        );
        let mut target = commit(1);
        target.pr_number = 101;
        let mut render = |commit: &crate::model::Commit, pr: &crate::model::PullRequest| {
            bodies.get_shared(pr.number, &pr.body)?.render_update(
                &commit.message,
                "Changed stack footer",
                "",
            )
        };
        g.update_content(&[&target], &mut render).unwrap();
        assert_eq!(crate::tests::unit::support::BODY_ANALYSES.get(), 2);
        assert!(g.get(101).unwrap().body.contains("New human text"));
        assert!(g.get(101).unwrap().body.contains("Changed stack footer"));
        w.0.lock().unwrap().trace.clear();
        g.update_content(&[&target], &mut render).unwrap();
        assert_eq!(crate::tests::unit::support::BODY_ANALYSES.get(), 3);
        g.update_content(&[&target], &mut render).unwrap();
        assert_eq!(crate::tests::unit::support::BODY_ANALYSES.get(), 3);
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
    });
}

#[test]
fn malformed_later_description_stops_an_entire_prepared_mutation_batch() {
    with_github(|g, w| {
        let before = g.get(101).unwrap();
        w.0.lock().unwrap().prs.get_mut(&102).unwrap()["body"] =
            json!(format!("{}\nHuman notes", crate::stack::description::START));
        let mut first = commit(1);
        first.pr_number = 101;
        first.title = "Changed first title".into();
        let mut second = commit(2);
        second.pr_number = 102;
        let mut bodies = crate::stack::description::BodyCache::default();
        w.0.lock().unwrap().trace.clear();
        let diagnostic = format!(
            "{:#}",
            g.update_content(&[&first, &second], &mut |commit, pr| {
                bodies.get_shared(pr.number, &pr.body)?.render_update(
                    &commit.message,
                    "* #101\n* #102",
                    "",
                )
            })
            .unwrap_err()
        );
        assert!(diagnostic.contains("PR #102 description"));
        assert!(diagnostic.contains("malformed managed stack footer"));
        assert_eq!(g.get(101).unwrap().title, before.title);
        assert_eq!(g.get(101).unwrap().body, before.body);
        let memory = w.0.lock().unwrap();
        assert_eq!(memory.prs[&101]["title"], before.title);
        assert_eq!(memory.prs[&101]["body"], before.body.as_ref());
        assert!(!memory.trace.iter().any(|event| {
            event["body"]["query"]
                .as_str()
                .is_some_and(|query| query.starts_with("mutation"))
        }));
        drop(memory);
    });
}

#[test]
fn unrelated_observation_errors_do_not_disable_stack_schema_checks() {
    with_github(|g, w| {
        let message = "Cannot query field title";
        w.http(
            "POST graphql",
            vec![Reply::Output(
                200,
                json!({"errors":[{"message":message}]}).to_string(),
            )],
        );
        w.0.lock().unwrap().trace.clear();
        let error = g.observe(&["topic/1".into()]).unwrap_err();
        assert!(error.to_string().contains(message));
        assert_eq!(w.trace().len(), 1);
        assert!(g.session.heads().is_empty());

        w.http("POST graphql", vec![]);
        w.0.lock().unwrap().trace.clear();
        g.observe(&["topic/1".into()]).unwrap();
        let trace = w.trace();
        assert_eq!(trace.len(), 1);
        assert!(
            trace[0]["body"]["query"]
                .as_str()
                .unwrap()
                .contains("stack{number}")
        );
        assert_eq!(g.publication_pr("topic/1").unwrap().unwrap().number, 101);
    });
}

#[test]
fn unsupported_stack_schema_is_remembered_and_empty_heads_remain_batched() {
    with_github(|g, w| {
        let mut repository = json!({});
        for i in 0..25 {
            repository[format!("p{i}")] = json!({"nodes":[],"pageInfo":{"hasNextPage":false}});
        }
        w.http(
            "POST graphql",
            vec![
                Reply::Output(
                    200,
                    json!({"errors":[{"message":"Cannot query field stack"}]}).to_string(),
                ),
                graph_reply(repository),
            ],
        );
        w.0.lock().unwrap().trace.clear();
        g.observe(
            &(1..=100)
                .map(|n| format!("missing/{n}"))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        g.observe(&["missing/101".into()]).unwrap();
        let trace = w.trace();
        assert_eq!(trace.len(), 6);
        assert!(trace.iter().all(|e| e["path"] == "graphql"));
        assert_eq!(
            trace
                .iter()
                .filter(|e| e["body"]["query"]
                    .as_str()
                    .unwrap()
                    .contains("stack{number}"))
                .count(),
            1
        );
    });
}

#[test]
fn reduced_observations_require_authoritative_membership_only_for_native_changes() {
    with_github(|g, w| {
        let node = json!({"number":101,"id":"PR_101","title":"T","body":"Human text","state":"OPEN","merged":false,"headRefName":"topic/1","baseRefName":"main","headRepository":{"nameWithOwner":"org/repo"},"labels":{"nodes":[],"pageInfo":{"hasNextPage":false}}});
        w.http("POST graphql", vec![
            Reply::Output(200, json!({"errors":[{"message":"Field stack doesn’t exist"}]}).to_string()),
            Reply::Output(200, json!({"data":{"repository":{"p0":{"nodes":[node],"pageInfo":{"hasNextPage":false}}}}}).to_string()),
        ]);
        g.observe(&["topic/1".into()]).unwrap();
        let mut target = commit(1);
        target.pr_number = 101;
        w.0.lock().unwrap().trace.clear();
        assert_eq!(g.get(101).unwrap().body.as_ref(), "Human text");
        g.verify_base(&target, "main").unwrap();
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["stack"] = json!({"number":9});
        assert!(g.verify_base(&target, "other").is_err());
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
        g.get_with_stack(101).unwrap();
        assert_eq!(w.trace().len(), 1);
    });
    with_github(|g, w| {
        let mut target = commit(1);
        target.pr_number = 101;
        let mut old = g.get(101).unwrap();
        Arc::make_mut(&mut old).base.name = "old".into();
        g.remember(old);
        super::guard(&g.session.unknown_stacks).insert(101);
        w.0.lock().unwrap().trace.clear();
        assert!(g.verify_base(&target, "main").is_err());
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
        g.get_with_stack(101).unwrap();
        g.verify_base(&target, "main").unwrap();
        assert_eq!(w.trace().len(), 1);
        assert_eq!(w.trace()[0]["method"], "GET");
    });
}

#[test]
fn unknown_membership_failure_stops_base_mutation() {
    with_github(|g, w| {
        g.get(101).unwrap();
        super::guard(&g.session.unknown_stacks).insert(101);
        w.http(
            "GET pulls/101",
            vec![Reply::Failure("membership unavailable".into())],
        );
        let mut target = commit(1);
        target.pr_number = 101;
        w.0.lock().unwrap().trace.clear();
        assert!(g.verify_base(&target, "different").is_err());
        assert!(!w.trace().iter().any(|e| e["method"] == "PATCH"));
    });
}

#[test]
fn expired_content_observations_refresh_in_bounded_batches() {
    let w = World::new(51);
    w.run(&["--no-stack"]).unwrap();
    let config = w.config();
    let commands = w.commands();
    let g = GitHub {
        session: Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    g.observe(&(1..=51).map(|n| format!("topic/{n}")).collect::<Vec<_>>())
        .unwrap();
    for time in super::guard(&g.session.pr_times).values_mut() {
        *time = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(31))
            .unwrap();
    }
    // Head observations remain fresh; per-PR expiry must still trigger a refresh.
    let changes = (101..=151)
        .map(|n| (n, "changed title".into(), "changed body".into()))
        .collect::<Vec<_>>();
    w.0.lock().unwrap().trace.clear();
    update_content(&g, &changes).unwrap();
    let trace = w.trace();
    let queries: Vec<_> = trace
        .iter()
        .filter_map(|e| e["body"]["query"].as_str())
        .collect();
    assert_eq!(queries.iter().filter(|q| q.starts_with("query")).count(), 3);
    assert_eq!(
        queries.iter().filter(|q| q.starts_with("mutation")).count(),
        11
    );
    assert!(!trace.iter().any(|e| e["method"] == "GET"));
    for request in trace.iter().filter(|e| e["path"] == "graphql") {
        assert!(request["body"]["variables"].as_object().unwrap().len() <= 27);
    }
    w.0.lock().unwrap().trace.clear();
    update_content(&g, &changes).unwrap();
    assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
}

#[test]
fn expired_content_refresh_preserves_identity_and_stops_on_observation_failure() {
    with_github(|g, w| {
        g.observe(&["topic/1".into(), "topic/2".into()]).unwrap();
        // A branch rename must not cause another PR on the old head to be updated.
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["head"]["ref"] = json!("renamed");
        w.0.lock().unwrap().prs.get_mut(&102).unwrap()["head"]["ref"] = json!("topic/1");
        for time in super::guard(&g.session.pr_times).values_mut() {
            *time = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(31))
                .unwrap();
        }
        update_content(g, &[(101, "renamed PR".into(), "new body".into())]).unwrap();
        assert_eq!(w.0.lock().unwrap().prs[&101]["title"], "renamed PR");
        assert_ne!(w.0.lock().unwrap().prs[&102]["title"], "renamed PR");
        super::guard(&g.session.pr_times).remove(&102);
        w.http(
            "POST graphql",
            vec![Reply::Failure("refresh failed".into())],
        );
        w.0.lock().unwrap().trace.clear();
        assert!(update_content(g, &[(102, "must not write".into(), "new body".into())]).is_err());
        assert_ne!(w.0.lock().unwrap().prs[&102]["title"], "must not write");
        assert!(!w.trace().iter().any(|e| {
            e["method"] == "PATCH"
                || e["body"]["query"]
                    .as_str()
                    .is_some_and(|q| q.starts_with("mutation"))
        }));
    });
}

#[test]
fn native_expiry_refreshes_in_bounded_batches() {
    for (stale, failed) in [(false, false), (true, false), (true, true)] {
        let w = World::new(51);
        w.run(&["--no-stack"]).unwrap();
        let mut config = w.config();
        config.auto_accept = true;
        let commands = w.commands();
        let g = GitHub {
            session: Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: w.as_ref(),
            template: "",
        };
        g.observe(&(1..=51).map(|n| format!("topic/{n}")).collect::<Vec<_>>())
            .unwrap();
        if stale {
            for time in super::guard(&g.session.pr_times).values_mut() {
                *time = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(31))
                    .unwrap();
            }
        }
        let commits = (1..=51)
            .map(|n| {
                let mut c = commit(n);
                c.pr_number = 100 + n;
                c
            })
            .collect::<Vec<_>>();
        w.0.lock().unwrap().trace.clear();
        if failed {
            w.http(
                "POST graphql",
                vec![Reply::Failure("refresh failed".into())],
            );
            assert!(g.manage_stack(&commits, "main").is_err());
            assert!(!w.trace().iter().any(|e| {
                e["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("stacks"))
            }));
            continue;
        }
        g.manage_stack(&commits, "main").unwrap();
        let reads = w
            .trace()
            .iter()
            .filter(|e| {
                e["method"] == "GET" && e["path"].as_str().is_some_and(|p| p.starts_with("pulls/"))
            })
            .count();
        assert_eq!(reads, 0);
        let queries = w
            .trace()
            .iter()
            .filter(|e| {
                e["body"]["query"]
                    .as_str()
                    .is_some_and(|q| q.starts_with("query"))
            })
            .count();
        assert_eq!(queries, if stale { 3 } else { 0 });
    }
}

#[test]
fn historical_status_queries_are_batched_deduplicated_and_use_fresh_observations() {
    let w = World::new(51);
    w.run(&["--no-stack"]).unwrap();
    let config = w.config();
    let commands = w.commands();
    let g = GitHub {
        session: Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    w.0.lock().unwrap().prs.get_mut(&101).unwrap()["merged"] = json!(true);
    let mut numbers = (101..=151).collect::<Vec<_>>();
    numbers.push(101);
    w.0.lock().unwrap().trace.clear();
    assert_eq!(
        g.merged_prs(&numbers).unwrap(),
        std::collections::HashSet::from([101])
    );
    assert_eq!(w.trace().len(), 3);
    for request in w.trace() {
        assert_eq!(request["method"], "POST");
        assert!(request["body"]["variables"].as_object().unwrap().len() <= 27);
    }
    g.observe(&["topic/1".into(), "topic/2".into()]).unwrap();
    w.0.lock().unwrap().trace.clear();
    assert_eq!(
        g.merged_prs(&[101, 102]).unwrap(),
        std::collections::HashSet::from([101])
    );
    assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
    w.0.lock().unwrap().prs.get_mut(&102).unwrap()["merged"] = json!(true);
    super::guard(&g.session.pr_times).insert(
        102,
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(31))
            .unwrap(),
    );
    assert_eq!(
        g.merged_prs(&[101, 102]).unwrap(),
        std::collections::HashSet::from([101, 102])
    );
    assert_eq!(w.trace().len(), 1);
}

#[test]
fn historical_status_queries_reject_incomplete_or_failed_observations() {
    with_github(|g, w| {
        assert!(g.merged_prs(&[u64::MAX]).is_err());
        for response in [
            Reply::Failure("network failure".into()),
            Reply::Output(200, "not json".into()),
            Reply::Output(200, json!({"errors":[{"message":"denied"}]}).to_string()),
            Reply::Output(200, json!({"data":{"repository":{"p0":null}}}).to_string()),
            Reply::Output(
                200,
                json!({"data":{"repository":{"p0":{"number":999,"merged":true}}}}).to_string(),
            ),
            Reply::Output(
                200,
                json!({"data":{"repository":{"p0":{"number":101}}}}).to_string(),
            ),
        ] {
            w.http("POST graphql", vec![response]);
            assert!(g.merged_prs(&[101]).is_err());
        }
        w.http(
            "POST graphql",
            vec![Reply::Output(
                200,
                json!({"data":{"repository":{"p0":{"number":0,"merged":true}}}}).to_string(),
            )],
        );
        assert!(g.merged_prs(&[0]).is_err());
        let mut config = g.config.clone();
        config.repo.name = "invalid".into();
        let invalid = GitHub {
            config: &config,
            session: Session::default(),
            clock: None,
            commands: g.commands,
            transport: g.transport,
            template: "",
        };
        assert!(invalid.merged_prs(&[101]).is_err());
    });
}

#[test]
fn context_lookup_reuses_open_heads_and_caches_historical_fallbacks() {
    with_github(|g, w| {
        let c = commit(1);
        g.observe(&[c.remote_ref().into()]).unwrap();
        w.http(
            &format!("GET commits/{}/pulls", c.hash),
            vec![Reply::Output(200, "[]".into())],
        );
        w.0.lock().unwrap().trace.clear();
        assert_eq!(g.lookup(&c).unwrap(), 101);
        assert_eq!(g.lookup(&c).unwrap(), 101);
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);

        // Once the head observation expires, use its newly observed state.
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["state"] = json!("closed");
        super::guard(&g.session.head_times).insert(
            c.remote_ref().into(),
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(31))
                .unwrap(),
        );
        let historical = json!([{"number":101,"state":"closed","head":{"ref":"topic/1","repo":{"full_name":"org/repo"}}}]);
        w.http(
            &format!("GET commits/{}/pulls", c.hash),
            vec![Reply::Output(200, historical.to_string())],
        );
        assert_eq!(g.lookup(&c).unwrap(), 101);
        w.0.lock().unwrap().trace.clear();
        assert_eq!(g.lookup(&c).unwrap(), 101);
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
        assert!(g.session.prs()[&101].body.contains("git-pr-stack"));

        super::guard(&g.session.commit_prs)
            .get_mut(&c.hash)
            .unwrap()
            .0 = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(31))
            .unwrap();
        w.http(
            &format!("GET commits/{}/pulls", c.hash),
            vec![Reply::Output(200, "[]".into())],
        );
        assert_eq!(g.lookup(&c).unwrap(), 0);
        assert_eq!(g.lookup(&c).unwrap(), 0);
        assert_eq!(w.trace().len(), 1);

        let missing = commit(3);
        w.http(
            &format!("GET commits/{}/pulls", missing.hash),
            vec![
                Reply::Failure("unavailable".into()),
                Reply::Output(200, "[]".into()),
            ],
        );
        assert!(g.lookup(&missing).is_err());
        assert_eq!(g.lookup(&missing).unwrap(), 0);
        w.0.lock().unwrap().trace.clear();
        assert_eq!(g.lookup(&missing).unwrap(), 0);
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
    });
}

#[test]
fn base_reconciliation_batches_expired_observations_and_reuses_fresh_ones() {
    let w = World::new(51);
    w.run(&["--no-stack"]).unwrap();
    let config = w.config();
    let commands = w.commands();
    let g = GitHub {
        session: Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        template: "",
    };
    g.observe(&(1..=51).map(|n| format!("topic/{n}")).collect::<Vec<_>>())
        .unwrap();
    let changes: Vec<_> = (1..=51)
        .map(|n| {
            let mut c = commit(n);
            c.pr_number = 100 + n;
            (
                c,
                if n == 1 {
                    "main".into()
                } else {
                    format!("topic/{}", n - 1)
                },
            )
        })
        .collect();
    assert_eq!(update_bases(&g, Vec::new()).unwrap().len(), 0);
    for expired in [true, false] {
        if expired {
            for time in super::guard(&g.session.pr_times).values_mut() {
                *time = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(31))
                    .unwrap();
            }
        }
        w.0.lock().unwrap().trace.clear();
        let updated = update_bases(&g, changes.clone()).unwrap();
        assert_eq!(
            updated.iter().map(|c| c.pr_number).collect::<Vec<_>>(),
            (101..=151).collect::<Vec<_>>()
        );
        assert!(updated.iter().all(|c| !c.base_updated && !c.base_blocked));
        let trace = w.trace();
        assert_eq!(trace.len(), if expired { 3 } else { 0 });
        for request in trace {
            assert_eq!(request["path"], "graphql");
            assert!(
                request["body"]["query"]
                    .as_str()
                    .unwrap()
                    .starts_with("query")
            );
            assert!(request["body"]["variables"].as_object().unwrap().len() <= 27);
        }
    }
    super::guard(&g.session.pr_times).clear();
    w.http(
        "POST graphql",
        vec![Reply::Failure("refresh failed".into())],
    );
    w.0.lock().unwrap().trace.clear();
    assert!(update_bases(&g, changes).is_err());
    assert_eq!(w.trace().len(), 1);
}

#[test]
fn base_batches_preserve_identity_and_stop_instead_of_retargeting_after_publication() {
    with_github(|g, w| {
        g.observe(&["topic/1".into(), "topic/2".into()]).unwrap();
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["head"]["ref"] = json!("renamed");
        w.0.lock().unwrap().prs.get_mut(&102).unwrap()["head"]["ref"] = json!("topic/1");
        super::guard(&g.session.pr_times).clear();
        let mut c = commit(1);
        c.pr_number = 101;
        w.0.lock().unwrap().trace.clear();
        let error = update_bases(g, vec![(c, "other".into()); 26]).unwrap_err();
        assert!(error.to_string().contains("unsafe publication"));
        assert_eq!(w.0.lock().unwrap().prs[&101]["base"]["ref"], "main");
        assert_eq!(w.0.lock().unwrap().prs[&102]["base"]["ref"], "topic/1");
        assert!(!w.trace().iter().any(|e| e["method"] == "PATCH"));
    });
}

fn paged_node(number: u64, reference: &str) -> serde_json::Value {
    json!({"number":number,"id":format!("PR_{number}"),"title":"Title","body":"Human text","state":"OPEN","merged":false,"headRefName":reference,"baseRefName":"main","headRepository":{"nameWithOwner":"org/repo"},"labels":{"nodes":[],"pageInfo":{"hasNextPage":false}},"stack":null})
}

fn connection(nodes: serde_json::Value, cursor: Option<&str>) -> serde_json::Value {
    let mut page = json!({"pageInfo":{"hasNextPage":cursor.is_some(),"endCursor":cursor}});
    page["nodes"] = nodes;
    page
}

fn graph_reply(repository: serde_json::Value) -> Reply {
    let mut response = json!({"data":{}});
    response["data"]["repository"] = repository;
    Reply::Output(200, response.to_string())
}

#[test]
fn paginated_heads_keep_initial_results_and_filter_foreign_repositories() {
    with_github(|g, w| {
        let first: Vec<_> = (1..=100)
            .map(|n| {
                let mut node = paged_node(n, "topic/1");
                node["state"] = json!("CLOSED");
                node
            })
            .collect();
        let mut foreign = paged_node(999, "topic/1");
        foreign["headRepository"]["nameWithOwner"] = json!("org/another-repo");
        w.http("POST graphql", vec![
            graph_reply(json!({"p0":connection(json!(first),Some("head-1")),"p1":connection(json!([]),None)})),
            graph_reply(json!({"p0":connection(json!([first[99],foreign,paged_node(101,"topic/1")]),None)})),
        ]);
        w.0.lock().unwrap().trace.clear();
        g.observe(&[
            "topic/1".into(),
            "topic/2".into(),
            "topic/1".into(),
            String::new(),
        ])
        .unwrap();
        assert_eq!(g.find_head("topic/1", "open").unwrap(), 101);
        assert_eq!(g.session.heads()["topic/1"].len(), 101);
        assert!(!g.session.prs().contains_key(&999));
        assert_eq!(g.get(1).unwrap().body.as_ref(), "Human text");
        let trace = w.trace();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[1]["body"]["variables"]["c0"], "head-1");
        assert!(trace[1]["body"]["variables"].get("h1").is_none());
        assert!(trace.iter().all(|r| r["path"] == "graphql"));
    });
}

#[test]
fn overflowing_labels_keep_first_pages_and_batch_only_unfinished_connections() {
    with_github(|g, w| {
        let nodes: Vec<_> = (1..=26)
            .map(|n| {
                let mut node = paged_node(n, "topic/1");
                node["labels"] = connection(
                    json!(
                        (0..100)
                            .map(|i| json!({"name":format!("label-{i}")}))
                            .collect::<Vec<_>>()
                    ),
                    Some("labels-1"),
                );
                node
            })
            .collect();
        let mut labels = json!({});
        for i in 0..25 {
            labels[format!("p{i}")] = json!({"number":i+1,"labels":connection(json!([{"name":"extra"}]), if i==0 {Some("labels-2")} else {None})});
        }
        w.http(
            "POST graphql",
            vec![
                graph_reply(json!({"p0":connection(json!(nodes),None)})),
                graph_reply(labels),
                graph_reply(
                    json!({"p0":{"number":26,"labels":connection(json!([{"name":"extra"}]),None)}}),
                ),
                graph_reply(
                    json!({"p0":{"number":1,"labels":connection(json!([{"name":"last"}]),None)}}),
                ),
            ],
        );
        w.0.lock().unwrap().trace.clear();
        g.observe(&["topic/1".into()]).unwrap();
        assert_eq!(g.get(1).unwrap().labels.len(), 102);
        assert_eq!(g.get(26).unwrap().labels.len(), 101);
        assert_eq!(g.get(2).unwrap().labels[0].name(), "label-0");
        let trace = w.trace();
        assert_eq!(trace.len(), 4);
        assert_eq!(trace[1]["body"]["variables"]["c0"], "labels-1");
        assert_eq!(trace[3]["body"]["variables"]["c0"], "labels-2");
        assert!(
            trace.iter().all(|r| r["path"] == "graphql"
                && r["body"]["variables"].as_object().unwrap().len() <= 52)
        );
    });
}

#[test]
fn incomplete_or_repeating_pagination_is_never_cached() {
    for page in [
        json!({"nodes":[],"pageInfo":{"hasNextPage":true}}),
        connection(json!([]), Some("")),
        connection(json!([]), Some("again")),
        json!({"nodes":[],"pageInfo":{}}),
    ] {
        with_github(|g, w| {
            w.http("POST graphql", vec![graph_reply(json!({"p0":page}))]);
            assert!(g.observe(&["topic/1".into()]).is_err());
            assert!(g.session.heads().is_empty());
        });
    }
    let node = {
        let mut n = paged_node(101, "topic/1");
        n["labels"] = connection(json!([]), Some("again"));
        n
    };
    for reply in [
        Reply::Failure("offline".into()),
        Reply::Output(200, "invalid json".into()),
        Reply::Output(200, json!({"errors":[{"message":"denied"}]}).to_string()),
        graph_reply(json!({"p0":{"number":102,"labels":connection(json!([]),None)}})),
        graph_reply(
            json!({"p0":{"number":101,"labels":{"nodes":null,"pageInfo":{"hasNextPage":false}}}}),
        ),
        graph_reply(json!({"p0":{"number":101,"labels":connection(json!([]),Some("again"))}})),
    ] {
        with_github(|g, w| {
            w.http(
                "POST graphql",
                vec![
                    graph_reply(json!({"p0":connection(json!([node]),None)})),
                    reply,
                ],
            );
            assert!(g.observe(&["topic/1".into()]).is_err());
            assert!(g.session.heads().is_empty());
            assert!(g.session.prs().is_empty());
        });
    }
    with_github(|g, w| {
        let mut large = node;
        large["number"] = json!(u64::MAX);
        w.http(
            "POST graphql",
            vec![graph_reply(json!({"p0":connection(json!([large]),None)}))],
        );
        assert!(g.observe(&["topic/1".into()]).is_err());
    });
}

#[test]
fn association_pages_require_source_repository_identity_before_matching_branches() {
    with_github(|g, w| {
        let c = commit(3);
        let foreign = json!({"number":999,"state":"open","head":{"ref":"topic/3","repo":{"full_name":"fork/repo"}}});
        let local = json!({"number":103,"state":"closed","head":{"ref":"topic/3","repo":{"full_name":"ORG/REPO"}}});
        w.http(
            &format!("GET commits/{}/pulls", c.hash),
            vec![
                Reply::Output(200, json!(vec![foreign; 100]).to_string()),
                Reply::Output(200, json!([{"head":{"repo":null}},local]).to_string()),
            ],
        );
        assert_eq!(g.lookup(&c).unwrap(), 103);
        assert!(!g.session.prs().contains_key(&999));
        w.0.lock().unwrap().trace.clear();
        assert_eq!(g.lookup(&c).unwrap(), 103);
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
    });
    for node in [
        json!({"number":101,"head":{"ref":"topic/1"}}),
        json!({"head":{"repo":{}}}),
        json!({"number":0,"head":{"ref":"topic/1","repo":{"full_name":"org/repo"}}}),
        json!({"number":101,"state":"open","head":{"repo":{"full_name":"org/repo"}}}),
        json!({"number":101,"state":"invalid","head":{"ref":"topic/1","repo":{"full_name":"org/repo"}}}),
        json!({"number":"invalid","head":{"repo":{"full_name":"org/repo"}}}),
    ] {
        with_github(|g, w| {
            w.http(
                "GET commits/test/pulls",
                vec![Reply::Output(200, json!([node]).to_string())],
            );
            assert!(g.commit_prs("test").is_err());
            assert!(super::guard(&g.session.commit_prs).is_empty());
        });
    }
}

#[test]
fn label_reconciliation_refreshes_expired_plans_and_remembers_partial_success() {
    with_github(|g, w| {
        let mut config = g.config.clone();
        config.labels = vec!["required".into()];
        config.tip_labels = vec!["tip /label".into()];
        let g = GitHub {
            config: &config,
            session: Session::default(),
            clock: None,
            ..*g
        };
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["labels"] = json!([{"name":"required"}]);
        g.observe(&["topic/1".into()]).unwrap();
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["labels"] = json!(["tip /label", "human"]);
        super::guard(&g.session.pr_times).insert(
            101,
            std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(31))
                .unwrap(),
        );
        let mut c = commit(1);
        c.pr_number = 101;
        w.0.lock().unwrap().trace.clear();
        w.http(
            "DELETE issues/101/labels/tip%20%2Flabel",
            vec![Reply::Failure("delete failed".into())],
        );
        assert!(g.update_labels(&[(&c, false)]).is_err());
        let pr = g.get(101).unwrap();
        assert!(pr.labels.iter().any(|l| l.name() == "required"));
        assert!(pr.labels.iter().any(|l| l.name() == "tip /label"));
        w.0.lock().unwrap().http_responses.clear();
        w.0.lock().unwrap().trace.clear();
        g.update_labels(&[(&c, false)]).unwrap();
        assert_eq!(w.trace().len(), 1);
        assert_eq!(w.trace()[0]["method"], "DELETE");
        assert!(
            g.get(101)
                .unwrap()
                .labels
                .iter()
                .any(|l| l.name() == "human")
        );
        w.0.lock().unwrap().trace.clear();
        g.update_labels(&[(&c, false)]).unwrap();
        assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
    });
}

#[test]
fn labels_refresh_in_bounded_groups_and_fail_before_mutation_on_observation_errors() {
    let w = World::new(51);
    w.run(&["--no-stack"]).unwrap();
    let mut config = w.config();
    config.labels = vec!["required".into()];
    for pr in w.0.lock().unwrap().prs.values_mut() {
        pr["labels"] = json!(["required"]);
    }
    let commands = w.commands();
    let g = GitHub {
        config: &config,
        commands: &commands,
        transport: w.as_ref(),
        clock: None,
        session: Session::default(),
        template: "",
    };
    g.observe(&(1..=51).map(|n| format!("topic/{n}")).collect::<Vec<_>>())
        .unwrap();
    for time in super::guard(&g.session.pr_times).values_mut() {
        *time = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(31))
            .unwrap();
    }
    let commits: Vec<_> = (1..=51)
        .map(|n| {
            let mut c = commit(n);
            c.pr_number = n + 100;
            c
        })
        .collect();
    let targets: Vec<_> = commits.iter().map(|c| (c, false)).collect();
    w.0.lock().unwrap().trace.clear();
    g.update_labels(&targets).unwrap();
    assert_eq!(w.trace().len(), 3);
    assert!(w.trace().iter().all(
        |v| v["path"] == "graphql" && v["body"]["query"].as_str().unwrap().starts_with("query")
    ));
    w.0.lock().unwrap().trace.clear();
    g.update_labels(&targets).unwrap();
    assert_eq!(w.trace(), [] as [serde_json::Value; 0]);
    super::guard(&g.session.pr_times).clear();
    w.http(
        "POST graphql",
        vec![Reply::Failure("refresh failed".into())],
    );
    assert!(g.update_labels(&targets).is_err());
    let empty = GitHub {
        session: Session::default(),
        ..g
    };
    w.http("GET pulls/101", vec![Reply::Failure("read failed".into())]);
    assert!(empty.update_labels(&targets).is_err());
}

#[test]
fn missing_initial_label_page_information_prevents_caching() {
    with_github(|g, w| {
        let mut node = paged_node(101, "topic/1");
        node["labels"] = json!({"nodes":[]});
        w.http(
            "POST graphql",
            vec![graph_reply(json!({"p0":connection(json!([node]),None)}))],
        );
        assert!(g.observe(&["topic/1".into()]).is_err());
        assert!(g.session.heads().is_empty());
    });
}

#[test]
fn cached_identity_and_expired_base_refresh_preserve_observation_rules() {
    with_github(|g, w| {
        let mut c = commit(1);
        assert_eq!(g.cached_find(&c), None);
        c.pr_number = 101;
        assert_eq!(g.cached_find(&c), Some(101));
        assert!(!g.base_is_current(&c, "main"));
        c.pr_number = 0;
        g.observe(&["topic/1".into()]).unwrap();
        assert_eq!(g.cached_find(&c), Some(101));
        g.expire_head_observation_for_test("topic/1");
        assert_eq!(g.cached_find(&c), None);
        assert_eq!(g.find_head("topic/1", "open").unwrap(), 101);
        c.pr_number = 101;
        assert!(g.base_is_current(&c, "main"));
        w.0.lock().unwrap().prs.get_mut(&101).unwrap()["base"]["ref"] = json!("desired");
        g.expire_pr_observation_for_test(101);
        assert!(!g.base_is_current(&c, "main"));
        w.0.lock().unwrap().trace.clear();
        g.verify_base(&c, "desired").unwrap();
        assert!(!c.base_updated);
        assert_eq!(w.trace().len(), 1);
        assert_eq!(w.trace()[0]["method"], "GET");
        assert!(g.base_is_current(&c, "desired"));
    });
}

#[test]
fn graphql_field_normalisation_preserves_owned_description_storage() {
    let mut node = json!({
        "id": "PR_101", "number": 101, "title": "Feature", "body": "β".repeat(60_000),
        "state": "OPEN", "merged": false, "merged_at": "unexpected REST field", "headRefName": "topic/1", "baseRefName": "main",
        "labels": {"nodes": [{"name": "label"}]}, "stack": {"number": 7}
    });
    let pointer = node["body"].as_str().unwrap().as_ptr();
    let head = node["headRefName"].as_str().unwrap().as_ptr();
    let base = node["baseRefName"].as_str().unwrap().as_ptr();
    super::observation::normalise_node(&mut node);
    assert_eq!(node["body"].as_str().unwrap().as_ptr(), pointer);
    assert_eq!(node["head"]["ref"].as_str().unwrap().as_ptr(), head);
    assert_eq!(node["base"]["ref"].as_str().unwrap().as_ptr(), base);
    let pr: crate::model::PullRequest = serde_json::from_value(node).unwrap();
    assert_eq!(pr.body.len(), 120_000);
    assert_eq!(pr.head.name, "topic/1");
    assert_eq!(pr.base.name, "main");
    assert_eq!(pr.labels[0].name(), "label");
    assert_eq!(pr.state, "open");
    assert!(!pr.is_merged());
    assert_eq!(pr.stack.unwrap().number, 7);
}

#[test]
fn malformed_graphql_connections_return_errors_without_caching_partial_heads() {
    for response in [
        json!({"data": false}),
        json!({"data": {"repository": []}}),
        json!({"data": {"repository": {"p0": false}}}),
        json!({"data": {"repository": {"p0": {"nodes": null}}}}),
    ] {
        with_github(|g, w| {
            w.http(
                "POST graphql",
                vec![Reply::Output(200, response.to_string())],
            );
            assert!(g.observe(&["topic/1".into()]).is_err());
            assert!(g.session.heads().is_empty());
        });
    }
}

#[test]
fn native_topology_uses_only_required_member_identities() {
    with_github(|g, w| {
        for pr in w.0.lock().unwrap().prs.values_mut() {
            pr["stack"] = json!({"number": 9});
        }
        let topology = json!({"number":9,"base":{"ref":"main"},"pull_requests":[
            {"number":101,"body":"x".repeat(60_000),"labels":"unused malformed labels","head":null},
            {"number":102,"body":17,"title":false,"labels":null}
        ]});
        w.http(
            "GET stacks/9",
            vec![Reply::Output(200, topology.to_string())],
        );
        let mut commits = [commit(1), commit(2)];
        commits[0].pr_number = 101;
        commits[1].pr_number = 102;
        w.0.lock().unwrap().trace.clear();
        assert_eq!(
            g.manage_stack(&commits, "main").unwrap(),
            super::native::StackOutcome::Unchanged
        );
        assert!(!w.trace().iter().any(|event| event["method"] == "POST"
            || event["method"] == "PATCH"
            || event["method"] == "DELETE"));
        let mut missing_identity = topology;
        missing_identity["pull_requests"][0]
            .as_object_mut()
            .unwrap()
            .remove("number");
        w.http(
            "GET stacks/9",
            vec![Reply::Output(200, missing_identity.to_string())],
        );
        let diagnostic = format!("{:#}", g.manage_stack(&commits, "main").unwrap_err());
        assert!(diagnostic.contains("invalid native stack observation"));
        assert!(diagnostic.contains("missing field `number`"));
    });
}

#[test]
fn rest_observations_require_complete_content_before_cache_replacement() {
    for field in [
        "number", "node_id", "body", "title", "labels", "state", "merged", "head", "base",
    ] {
        with_github(|g, w| {
            w.0.lock().unwrap().prs.get_mut(&101).unwrap()["body"] =
                json!("Human text which must survive");
            let before = g.get(101).unwrap();
            let mut incomplete = w.0.lock().unwrap().prs[&101].clone();
            incomplete.as_object_mut().unwrap().remove(field);
            g.expire_pr_observation_for_test(101);
            super::guard(&g.session.unknown_stacks).insert(101);
            w.http(
                "GET pulls/101",
                vec![Reply::Output(200, incomplete.to_string())],
            );
            w.0.lock().unwrap().trace.clear();
            let error = g.get(101).unwrap_err();
            assert!(format!("{error:#}").contains("invalid GitHub response for PR #101"));
            assert!(Arc::ptr_eq(&before, &g.session.prs()[&101]));
            assert!(super::guard(&g.session.unknown_stacks).contains(&101));
            assert_eq!(
                w.0.lock().unwrap().prs[&101]["body"],
                "Human text which must survive"
            );
            assert_eq!(w.trace().len(), 1);
            assert_eq!(w.trace()[0]["method"], "GET");
        });
    }
    for field in ["head", "base"] {
        for nested in ["ref", "repo"] {
            with_github(|g, w| {
                let mut incomplete = w.0.lock().unwrap().prs[&101].clone();
                incomplete[field].as_object_mut().unwrap().remove(nested);
                w.http(
                    "GET pulls/101",
                    vec![Reply::Output(200, incomplete.to_string())],
                );
                assert!(g.get(101).is_err());
                assert!(g.session.prs().is_empty());
            });
        }
    }
    with_github(|g, w| {
        w.http(
            "GET pulls/101",
            vec![Reply::Output(200, json!({"number":101}).to_string())],
        );
        let mut c = commit(1);
        c.pr_number = 101;
        w.0.lock().unwrap().trace.clear();
        assert!(
            g.update_content(&[&c], &mut |_, _| panic!(
                "incomplete observation must not be rendered"
            ))
            .is_err()
        );
        assert_eq!(w.trace().len(), 1);
        assert_eq!(w.trace()[0]["method"], "GET");
    });
}

#[test]
fn rest_identity_validation_retains_nullable_bodies_and_deleted_heads() {
    with_github(|g, w| {
        let mut pr = w.0.lock().unwrap().prs[&101].clone();
        pr["body"] = serde_json::Value::Null;
        pr["head"]["repo"] = serde_json::Value::Null;
        pr["state"] = json!("closed");
        pr["base"]["repo"]["full_name"] = json!("ORG/REPO");
        w.http("GET pulls/101", vec![Reply::Output(200, pr.to_string())]);
        let observed = g.get(101).unwrap();
        assert!(observed.body.is_empty());
        assert_eq!(observed.state, "closed");
    });
    for (path, value) in [
        (vec!["number"], json!(0)),
        (vec!["number"], json!(102)),
        (vec!["node_id"], json!("")),
        (vec!["state"], json!("invalid")),
        (vec!["head", "ref"], json!("")),
        (vec!["base", "ref"], json!("")),
        (vec!["head", "repo", "full_name"], json!("fork/repo")),
        (vec!["base", "repo", "full_name"], json!("other/repo")),
        (vec!["base", "repo"], serde_json::Value::Null),
    ] {
        with_github(|g, w| {
            let mut pr = w.0.lock().unwrap().prs[&101].clone();
            let mut target = &mut pr;
            for part in &path {
                target = &mut target[*part];
            }
            *target = value.clone();
            w.http("GET pulls/101", vec![Reply::Output(200, pr.to_string())]);
            assert!(g.get(101).is_err());
            assert!(g.session.prs().is_empty());
        });
    }
}

#[test]
fn creation_validates_requested_identity_without_adopting_other_prs() {
    for (path, value) in [
        (vec!["head", "ref"], json!("topic/2")),
        (vec!["base", "ref"], json!("wrong-base")),
        (vec!["head", "repo", "full_name"], json!("fork/repo")),
        (vec!["base", "repo", "full_name"], json!("other/repo")),
        (vec!["head", "repo"], serde_json::Value::Null),
        (vec!["state"], json!("closed")),
        (vec!["merged"], json!(true)),
        (vec!["merged_at"], json!("2026-10-14T00:00:00Z")),
    ] {
        with_github(|g, w| {
            let mut pr = w.0.lock().unwrap().prs[&101].clone();
            let mut target = &mut pr;
            for part in &path {
                target = &mut target[*part];
            }
            *target = value.clone();
            w.http("POST pulls", vec![Reply::Output(200, pr.to_string())]);
            w.0.lock().unwrap().trace.clear();
            let mut c = commit(1);
            let error = g.create(&mut c, "main").unwrap_err();
            let diagnostic = format!("{error:#}");
            assert!(diagnostic.contains("unexpected"));
            assert!(diagnostic.contains("a PR may already exist"));
            assert!(diagnostic.contains("before retrying"));
            assert_eq!(c.pr_number, 0);
            assert!(!c.newly_created);
            assert!(g.session.prs().is_empty());
            assert!(g.session.heads().is_empty());
            assert_eq!(w.trace().len(), 1);
            assert_eq!(w.trace()[0]["method"], "POST");
            assert_eq!(w.trace()[0]["path"], "pulls");
        });
    }
    with_github(|g, w| {
        let mut response = w.0.lock().unwrap().prs[&102].clone();
        response["body"] = json!("Another branch's human description");
        w.http("POST pulls", vec![Reply::Output(200, response.to_string())]);
        let mut c = commit(1);
        assert!(g.create(&mut c, "main").is_err());
        assert_eq!(c.pr_number, 0);
        assert_eq!(w.0.lock().unwrap().prs[&102]["title"], "Feature 2");
    });
}

fn association_rows() -> Vec<serde_json::Value> {
    (0..100).map(|i| json!({"number":1000+i,"state":if i==0 {"open"} else {"closed"},"head":{"ref":format!("topic/{i}"),"repo":{"full_name":"org/repo"}}})).collect()
}

#[test]
fn association_pages_deduplicate_overlaps_and_share_cached_identities() {
    with_github(|g, w| {
        let mut c = commit(3);
        c.attrs.clear();
        let rows = association_rows();
        let mut last = rows[1].clone();
        last["number"] = json!(1100);
        w.http(
            &format!("GET commits/{}/pulls", c.hash),
            vec![
                Reply::Output(200, json!(rows).to_string()),
                Reply::Output(200, json!([rows[0], last]).to_string()),
            ],
        );
        w.0.lock().unwrap().trace.clear();
        assert_eq!(g.find(&c).unwrap(), 1000);
        let first = g.commit_prs(&c.hash).unwrap();
        let cached = g.commit_prs(&c.hash).unwrap();
        assert!(Arc::ptr_eq(&first, &cached));
        assert_eq!(first.len(), 101);
        assert_eq!(first.iter().filter(|pr| pr.number == 1000).count(), 1);
        assert_eq!(w.trace().len(), 2);
    });
}

#[test]
fn association_pagination_stops_when_full_pages_make_no_progress() {
    for foreign in [false, true] {
        with_github(|g, w| {
            let mut rows = association_rows();
            if foreign {
                for row in &mut rows {
                    row["head"]["repo"]["full_name"] = json!("fork/repo");
                }
            }
            let mut reordered = rows.clone();
            reordered.reverse();
            w.http(
                "GET commits/test/pulls",
                vec![
                    Reply::Output(200, json!(rows).to_string()),
                    Reply::Output(200, json!(reordered).to_string()),
                    Reply::Failure("third page must not be requested".into()),
                ],
            );
            w.0.lock().unwrap().trace.clear();
            let error = g.commit_prs("test").unwrap_err();
            let diagnostic = format!("{error:#}");
            assert!(diagnostic.contains("pagination did not advance"));
            assert!(diagnostic.contains("commits/test/pulls at page 2"));
            assert_eq!(w.trace().len(), 2);
            assert!(super::guard(&g.session.commit_prs).is_empty());
        });
    }
}

#[test]
fn association_decoding_ignores_unused_content_and_preserves_source_identity() {
    with_github(|g, w| {
        let mut rows = association_rows();
        for row in &mut rows {
            row["body"] = json!("x".repeat(60_000));
            row["title"] = json!({"unused":"malformed title"});
            row["labels"] = json!(false);
            row["node_id"] = json!([]);
        }
        w.http(
            "GET commits/test/pulls",
            vec![
                Reply::Output(200, json!(rows).to_string()),
                Reply::Output(200, "[]".into()),
            ],
        );
        let prs = g.commit_prs("test").unwrap();
        assert_eq!(prs.len(), 100);
        assert_eq!(prs[0].number, 1000);
        assert_eq!(prs[0].state, "open");
        assert_eq!(prs[0].head, "topic/0");
        assert!(Arc::ptr_eq(&prs, &g.commit_prs("test").unwrap()));
    });
}
