use super::*;
use crate::tests::unit::support::{DiscoveryApi, World, commit};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

fn with_github<T>(world: &Arc<World>, run: impl FnOnce(&GitHub<'_>) -> T) -> T {
    let commands = world.commands();
    let config = world.config();
    run(&GitHub {
        commands: &commands,
        config: &config,
        session: super::super::Session::default(),
        clock: Some(world.as_ref()),
        transport: world.as_ref(),
        template: "",
    })
}

#[test]
fn ten_layers_reuse_api_refs_and_negative_objects_without_changing_creation_order() {
    for (enabled, native) in [(false, false), (true, false), (false, true), (true, true)] {
        let flags = if native {
            &["--github-stack"][..]
        } else {
            &["--no-stack"][..]
        };
        let world = World::new(1);
        world.run(flags).unwrap();
        {
            let mut memory = world.0.lock().unwrap();
            memory.commits = (1..=10).map(commit).collect();
            for commit in &mut memory.commits[1..] {
                commit.attrs.clear();
            }
            memory.branch = "topic/10".into();
            memory.discovery_api = if enabled {
                DiscoveryApi::Supported
            } else {
                DiscoveryApi::Unsupported
            };
            memory.trace.clear();
        }
        world.run(flags).unwrap();
        let trace = world.trace();
        assert_eq!(
            trace.iter().filter(|e| e["args"][0] == "ls-remote").count(),
            usize::from(!enabled)
        );
        assert_eq!(trace.iter().filter(|e| e["args"][0] == "push").count(), 1);
        assert_eq!(
            trace
                .iter()
                .filter(|e| {
                    e["path"]
                        .as_str()
                        .is_some_and(|p| p.starts_with("commits/"))
                })
                .count(),
            if enabled { 0 } else { 9 }
        );
        let created = trace
            .iter()
            .filter(|e| e["path"] == "pulls" && e["method"] == "POST")
            .map(|e| e["body"]["head"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            created,
            (2..=10).map(|i| format!("topic/{i}")).collect::<Vec<_>>()
        );
        assert_eq!(world.0.lock().unwrap().prs.len(), 10);
        assert!(
            world
                .0
                .lock()
                .unwrap()
                .prs
                .values()
                .all(|pr| pr["state"] == "open")
        );
        let http = trace.iter().filter(|e| e["kind"] == "http").count();
        assert_eq!(http, if enabled { 14 } else { 23 } + usize::from(native));
        assert_eq!(
            trace
                .iter()
                .filter(|event| event["args"][1]
                    == "--format=%(refname:strip=2)%00%(push:remotename)%00%(push:remoteref)")
                .count(),
            1
        );
        eprintln!(
            "TEN-LAYER API={enabled} NATIVE={native}: {http} HTTP requests, one push, nine sequential creations"
        );
    }
}

#[test]
fn new_native_stack_combines_preflight_discovery_and_preserves_create_order() {
    let world = World::new(7);
    {
        let mut memory = world.0.lock().unwrap();
        memory.discovery_api = DiscoveryApi::Supported;
        for commit in &mut memory.commits {
            commit.attrs.clear();
        }
    }
    world.run(&["--github-stack", "--draft"]).unwrap();
    let trace = world.trace();
    let push = trace.iter().position(|e| e["args"][0] == "push").unwrap();
    let before_push: Vec<_> = trace[..push]
        .iter()
        .filter(|e| e["kind"] == "http")
        .collect();
    assert_eq!(before_push.len(), 1);
    let variables = &before_push[0]["body"]["variables"];
    for index in 0..7 {
        assert_eq!(
            variables[format!("candidateDependant{index}")],
            format!("topic/{}", index + 1)
        );
        assert!(variables.get(format!("publicationRef{index}")).is_some());
    }
    let published = trace[push + 1..]
        .iter()
        .find(|event| event["kind"] == "http")
        .unwrap();
    assert_eq!(published["path"], "graphql");
    let variables = &published["body"]["variables"];
    assert_eq!(variables["h0"], "topic/1");
    assert_eq!(variables["h6"], "topic/7");
    assert!(variables.get("publicationRef0").is_none());
    assert!(variables.get("candidateDependant0").is_none());
    let writes = trace[push + 1..]
        .iter()
        .filter(|e| e["path"] == "pulls" && e["method"] == "POST")
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 7);
    for (index, event) in writes.into_iter().enumerate() {
        assert_eq!(event["body"]["head"], format!("topic/{}", index + 1));
        assert_eq!(event["body"]["draft"], true);
    }
    assert_eq!(trace.iter().filter(|e| e["kind"] == "http").count(), 12);

    world.0.lock().unwrap().trace.clear();
    world.run(&["--github-stack", "--draft"]).unwrap();
    assert!(world.trace().iter().all(|e| {
        e["args"][0] != "push"
            && e["body"]["variables"].get("candidateDependant0").is_none()
            && e["body"]["variables"].get("b0").is_none()
    }));
}

#[test]
fn possible_dependant_still_requires_live_preflight_before_push() {
    let world = World::new(1);
    {
        let mut memory = world.0.lock().unwrap();
        memory.discovery_api = DiscoveryApi::Supported;
        memory.commits[0].attrs.clear();
        memory.prs.insert(
            201,
            json!({
                "number": 201, "state": "open",
                "head": {"ref": "outside", "sha": commit(1).hash},
                "base": {"ref": "topic/1"}
            }),
        );
        drop(memory);
    }
    let error = world.run(&["--no-stack"]).unwrap_err();
    assert!(format!("{error:#}").contains("publication declined"));
    let trace = world.trace();
    assert!(
        trace
            .iter()
            .any(|e| e["body"]["variables"]["candidateDependant0"] == "topic/1")
    );
    assert!(
        trace
            .iter()
            .any(|e| e["body"]["variables"]["b0"] == "topic/1")
    );
    assert!(!trace.iter().any(|e| e["args"][0] == "push"));
}

#[test]
fn only_fresh_complete_empty_dependant_connections_can_skip_preflight() {
    let world = World::new(1);
    with_github(&world, |github| {
        let references = vec!["topic/1".into()];
        github.prepare_discovery(Vec::new(), BTreeSet::from(["topic/1".into()]));
        let probe = github.discovery_probe(references.clone(), true).unwrap();
        for (connection, expires, reusable) in [
            (Value::Null, false, false),
            (
                json!({"nodes":[],"pageInfo":{"hasNextPage":true}}),
                false,
                false,
            ),
            (
                json!({"nodes":[{"number":201}],"pageInfo":{"hasNextPage":false}}),
                false,
                false,
            ),
            (json!({"nodes":[],"pageInfo":{}}), false, false),
            (
                json!({"nodes":[],"pageInfo":{"hasNextPage":false}}),
                false,
                true,
            ),
            (
                json!({"nodes":[],"pageInfo":{"hasNextPage":false}}),
                true,
                false,
            ),
        ] {
            let acquired = github.now();
            let response = json!({"data":{"repository":{
                "nameWithOwner":"org/repo","publicationRef0":null,
                "candidateDependant0":connection
            }}});
            probe.remember(github, &response, acquired).unwrap();
            if expires {
                world.0.lock().unwrap().clock_elapsed += Duration::from_secs(31);
            }
            assert_eq!(
                github.take_empty_dependants(&references),
                reusable.then_some(acquired)
            );
        }
    });
}

#[test]
fn a_dependant_field_error_retries_discovery_without_the_optional_fields() {
    use crate::tests::unit::support::Response;

    let world = World::new(1);
    world.http(
        "POST graphql",
        vec![
            Response::Output(
                200,
                json!({"errors":[{
                    "message": "optional dependant lookup failed",
                    "path": ["repository", "candidateDependant0"]
                }]})
                .to_string(),
            ),
            Response::Output(
                200,
                json!({"data":{"repository":{
                    "nameWithOwner":"org/repo",
                    "publicationRef0":null,
                    "p0":{"nodes":[],"pageInfo":{"hasNextPage":false}}
                }}})
                .to_string(),
            ),
        ],
    );
    with_github(&world, |github| {
        let references = vec!["topic/1".into()];
        github.prepare_discovery(Vec::new(), BTreeSet::from(["topic/1".into()]));
        let result = github.observe_publication(&references);
        assert!(result.is_ok(), "{result:#?}; trace: {:?}", world.trace());
        assert!(github.take_empty_dependants(&references).is_none());
    });
    let trace = world.trace();
    let observations: Vec<_> = trace.iter().filter(|e| e["kind"] == "http").collect();
    assert_eq!(observations.len(), 2);
    assert_eq!(
        observations[0]["body"]["variables"]["candidateDependant0"],
        "topic/1"
    );
    assert!(
        observations[1]["body"]["variables"]
            .get("candidateDependant0")
            .is_none()
    );
}

#[test]
fn ref_cache_expires_and_preserves_expected_absence_and_exact_object_identity() {
    let world = World::new(1);
    world.0.lock().unwrap().discovery_api = DiscoveryApi::Supported;
    let references = vec!["topic/1".into()];
    with_github(&world, |github| {
        assert!(github.publication_refs(&references).unwrap().is_none());
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert_eq!(
            github.publication_refs(&references).unwrap().unwrap()["topic/1"],
            None
        );
        let count = world.trace().len();
        github.publication_refs(&references).unwrap();
        assert_eq!(world.trace().len(), count);
        let oid = "a".repeat(40);
        {
            let mut memory = world.0.lock().unwrap();
            memory
                .remote_refs
                .insert("refs/heads/topic/1".into(), oid.clone());
            memory.clock_elapsed += Duration::from_secs(31);
        }
        assert_eq!(
            github.publication_refs(&references).unwrap().unwrap()["topic/1"],
            Some(oid)
        );
    });
}

#[test]
fn discovery_decodes_transactionally_and_requires_exact_names_and_complete_aliases() {
    let world = World::new(1);
    with_github(&world, |github| {
        github.prepare_discovery(vec!["a".repeat(40)], BTreeSet::new());
        let probe = github
            .discovery_probe(vec!["topic/1".into()], false)
            .unwrap();
        let valid = json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationRef0":null,"publicationControl":{"__typename":"Commit","oid":"0".repeat(40)},"publicationObject0":null}}});
        for broken in [
            json!({"errors":[{"message":"unavailable"}]}),
            json!({"data":{}}),
            json!({"data":{"repository":{"nameWithOwner":"foreign/repo"}}}),
            json!({"data":{"repository":{"nameWithOwner":"org/repo"}}}),
            json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationRef0":{"prefix":"refs/tags/","name":"topic/1","target":{"__typename":"Commit","oid":"0".repeat(40)}}}}}),
        ] {
            assert!(probe.remember(github, &broken, github.now()).is_err());
            assert!(guard(&github.session.discovery).tips.is_empty());
        }
        for target in [
            json!({}),
            json!({"__typename":"Blob","oid":"a".repeat(40)}),
            json!({"__typename":"Commit","oid":"invalid"}),
        ] {
            assert!(commit_oid(&target).is_err());
        }
        let mut missing_object = valid.clone();
        missing_object["data"]["repository"]
            .as_object_mut()
            .unwrap()
            .remove("publicationObject0");
        assert!(
            probe
                .remember(github, &missing_object, github.now())
                .is_err()
        );
        let mut mismatched = valid.clone();
        mismatched["data"]["repository"]["publicationObject0"] =
            json!({"__typename":"Commit","oid":"b".repeat(40)});
        assert!(probe.remember(github, &mismatched, github.now()).is_err());
        probe.remember(github, &valid, github.now()).unwrap();
        assert_eq!(
            github.identity_heads(&"a".repeat(40)).unwrap(),
            Vec::<String>::new()
        );
        assert!(!world.trace().iter().any(|e| e["kind"] == "http"));
    });
}

#[test]
fn absent_objects_require_visibility_and_present_objects_keep_the_rest_reader() {
    let world = World::new(1);
    with_github(&world, |github| {
        github.prepare_discovery(vec!["a".repeat(40)], BTreeSet::new());
        let probe = github.discovery_probe(Vec::new(), false).unwrap();
        probe.remember(github,&json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationControl":null,"publicationObject0":null}}}),github.now()).unwrap();
        assert!(guard(&github.session.commit_prs).is_empty());
        let value = json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationControl":{"__typename":"Commit","oid":"0".repeat(40)},"publicationObject0":{"__typename":"Commit","oid":"a".repeat(40)}}}});
        probe.remember(github, &value, github.now()).unwrap();
        assert!(guard(&github.session.commit_prs).is_empty());
    });
}

#[test]
fn optional_discovery_falls_back_without_turning_unsupported_or_denied_fields_into_absence() {
    let world = World::new(1);
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        let probe = github
            .discovery_probe(vec!["topic/1".into()], false)
            .unwrap();
        assert!(probe.unsupported(
            &json!({"errors":[{"type":"FORBIDDEN","path":["repository","publicationRef0"]}]})
        ));
        assert!(probe.unsupported(&json!({"errors":[{"message":"Cannot query field ref"}]})));
        assert!(
            !probe.unsupported(
                &json!({"errors":[{"type":"NOT_FOUND","message":"wrong repository"}]})
            )
        );
        assert!(!probe.unsupported(&json!({"errors":[]})));
        probe
            .remember(github, &json!({"data":{"repository":{}}}), github.now())
            .unwrap();
        assert!(
            github
                .publication_refs(&["topic/1".into()])
                .unwrap()
                .is_none()
        );
        assert!(github.discovery_probe(Vec::new(), false).is_none());
    });
    let mut config = world.config();
    config.repo.host = "enterprise.example".into();
    let commands = world.commands();
    let github = GitHub {
        config: &config,
        commands: &commands,
        session: super::super::Session::default(),
        clock: None,
        transport: world.as_ref(),
        template: "",
    };
    github.prepare_discovery(Vec::new(), BTreeSet::new());
    assert!(github.discovery_probe(Vec::new(), false).is_none());
}

#[test]
fn unsupported_fields_retry_only_the_read_and_malformed_ref_reads_stop() {
    use crate::tests::unit::support::Response;
    let world = World::new(1);
    world.http(
        "POST graphql",
        vec![
            Response::Output(
                200,
                json!({"errors":[{"message":"Cannot query field ref"}]}).to_string(),
            ),
            Response::Output(
                200,
                json!({"data":{"repository":{"p0":{"nodes":[],"pageInfo":{"hasNextPage":false}}}}})
                    .to_string(),
            ),
        ],
    );
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        github.observe(&["topic/1".into()]).unwrap();
        assert!(
            github
                .publication_refs(&["topic/1".into()])
                .unwrap()
                .is_none()
        );
        assert_eq!(
            world.trace().iter().filter(|e| e["kind"] == "http").count(),
            2
        );
    });
    for response in [
        "not json".to_owned(),
        json!({"data":{"repository":{"nameWithOwner":"foreign/repo"}}}).to_string(),
    ] {
        let world = World::new(1);
        world.http("POST graphql", vec![Response::Output(200, response)]);
        with_github(&world, |github| {
            github.prepare_discovery(Vec::new(), BTreeSet::new());
            assert!(github.publication_refs(&["topic/1".into()]).is_err());
        });
    }
    let world = World::new(1);
    world.http(
        "POST graphql",
        vec![Response::Output(
            200,
            json!({"errors":[{"type":"FORBIDDEN","path":["repository","publicationRef0"]}]})
                .to_string(),
        )],
    );
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert!(
            github
                .publication_refs(&["topic/1".into()])
                .unwrap()
                .is_none()
        );
    });
}

#[test]
fn recovery_only_ref_queries_have_no_selected_branch_total_and_missing_control_is_not_absence() {
    let world = World::new(1);
    world.0.lock().unwrap().discovery_api = DiscoveryApi::Supported;
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert_eq!(github.publication_refs(&[]).unwrap(), Some(BTreeMap::new()));
        assert_eq!(
            github
                .publication_refs(&["git-pr/transition/101".into()])
                .unwrap()
                .unwrap()["git-pr/transition/101"],
            None
        );
        github.prepare_discovery(vec!["a".repeat(40)], BTreeSet::new());
        let probe = github.discovery_probe(Vec::new(), false).unwrap();
        assert!(probe.remember(github,&json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationControl":{"__typename":"Blob","oid":"a".repeat(40)},"publicationObject0":null}}}),github.now()).is_err());
    });
}

#[derive(Debug)]
struct ConcurrentBranch {
    world: Arc<World>,
    replacement: Option<String>,
}

impl crate::github::http::Transport for ConcurrentBranch {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let payload: Value = request
            .body
            .map_or(Value::Null, |body| serde_json::from_slice(body).unwrap());
        let bytes = self.world.request(request)?;
        if payload["variables"].get("b0").is_some() {
            let mut memory = self.world.0.lock().unwrap();
            if let Some(oid) = &self.replacement {
                memory
                    .remote_refs
                    .insert("refs/heads/topic/1".into(), oid.clone());
            } else {
                memory.remote_refs.remove("refs/heads/topic/1");
            }
        }
        Ok(bytes)
    }
}

#[test]
fn api_leases_reject_concurrent_branch_creation_movement_and_deletion_before_pr_creation() {
    for (initial, replacement) in [
        (None, Some("a".repeat(40))),
        (Some("b".repeat(40)), Some("a".repeat(40))),
        (Some("b".repeat(40)), None),
    ] {
        let world = World::new(1);
        {
            let mut memory = world.0.lock().unwrap();
            memory.discovery_api = DiscoveryApi::Supported;
            if let Some(oid) = initial {
                memory.remote_refs.insert("refs/heads/topic/1".into(), oid);
            }
        }
        let mut app = world.app();
        app.transport = Arc::new(ConcurrentBranch {
            world: world.clone(),
            replacement: replacement.clone(),
        });
        let flags = crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap();
        assert!(
            app.execute(
                &flags,
                &BTreeMap::from([("HOME".into(), "/unit/home".into())])
            )
            .is_err()
        );
        let memory = world.0.lock().unwrap();
        assert_eq!(
            memory.remote_refs.get("refs/heads/topic/1"),
            replacement.as_ref()
        );
        assert!(memory.prs.is_empty());
        assert!(
            !memory
                .trace
                .iter()
                .any(|event| event["path"] == "pulls" && event["method"] == "POST")
        );
        drop(memory);
    }
}

#[test]
fn ref_queries_are_bounded_and_metadata_omission_selects_git_fallback() {
    let world = World::new(1);
    world.0.lock().unwrap().discovery_api = DiscoveryApi::Supported;
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        let references = (1..=51).map(|i| format!("topic/{i}")).collect::<Vec<_>>();
        assert_eq!(
            github.publication_refs(&references).unwrap().unwrap().len(),
            51
        );
        assert_eq!(
            world
                .trace()
                .iter()
                .filter(|event| event["kind"] == "http")
                .count(),
            3
        );
        assert!(
            world
                .trace()
                .iter()
                .all(|event| event["body"]["variables"].get("publicationRef25").is_none())
        );
    });
    let world = World::new(1);
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert!(
            github
                .publication_refs(&["topic/1".into()])
                .unwrap()
                .is_none()
        );
    });
}

#[derive(Debug)]
struct SlowObservation(Arc<World>);

impl crate::github::http::Transport for SlowObservation {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let bytes = self.0.request(request)?;
        self.0.0.lock().unwrap().clock_elapsed += Duration::from_secs(31);
        Ok(bytes)
    }
}

#[test]
fn evidence_that_expires_during_acquisition_cannot_be_used_for_publication() {
    let world = World::new(1);
    world.0.lock().unwrap().discovery_api = DiscoveryApi::Supported;
    let commands = world.commands();
    let config = world.config();
    let transport = SlowObservation(world.clone());
    let github = GitHub {
        commands: &commands,
        config: &config,
        session: super::super::Session::default(),
        clock: Some(world.as_ref()),
        transport: &transport,
        template: "",
    };
    github.prepare_discovery(Vec::new(), BTreeSet::new());
    assert!(
        github
            .publication_refs(&["topic/1".into()])
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
}

#[test]
fn incomplete_cached_ref_evidence_is_refused() {
    let discovery = Discovery::default();
    assert!(
        discovery
            .verified_tips(&["topic/1".into()], Instant::now())
            .is_err()
    );
}

#[test]
fn discovery_refuses_unreadable_queries_and_invalid_repository_configuration() {
    use crate::tests::unit::support::Response;
    let world = World::new(1);
    world.http(
        "POST graphql",
        vec![Response::Failure("unavailable".into())],
    );
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert!(github.publication_refs(&["topic/1".into()]).is_err());
    });
    let commands = world.commands();
    let mut config = world.config();
    config.repo.name = "invalid".into();
    let github = GitHub {
        commands: &commands,
        config: &config,
        session: super::super::Session::default(),
        clock: Some(world.as_ref()),
        transport: world.as_ref(),
        template: "",
    };
    github.prepare_discovery(Vec::new(), BTreeSet::new());
    assert!(github.publication_refs(&["topic/1".into()]).is_err());
}

#[test]
fn incomplete_ref_fields_and_malformed_object_types_never_become_evidence() {
    let world = World::new(1);
    with_github(&world, |github| {
        github.prepare_discovery(vec!["a".repeat(40)], BTreeSet::new());
        let probe = github
            .discovery_probe(vec!["topic/1".into()], false)
            .unwrap();
        assert!(probe.unsupported(&json!({"errors":[{"message":"Cannot query field object"}]})));
        for node in [
            json!({"name":"topic/1"}),
            json!({"prefix":"refs/heads/"}),
            json!({"prefix":"refs/heads/","name":"topic/1","target":{"__typename":"Blob","oid":"a".repeat(40)}}),
        ] {
            assert!(probe.remember(github,&json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationRef0":node}}}),github.now()).is_err());
        }
        assert!(probe.remember(github,&json!({"data":{"repository":{"nameWithOwner":"org/repo","publicationRef0":null,"publicationObject0":{"__typename":"Blob","oid":"a".repeat(40)}}}}),github.now()).is_err());
        assert!(guard(&github.session.discovery).tips.is_empty());
    });
    world.http(
        "POST graphql",
        vec![crate::tests::unit::support::Response::Output(
            200,
            json!({"data":{"repository":{"nameWithOwner":"other/repo"}}}).to_string(),
        )],
    );
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert!(github.observe(&["topic/1".into()]).is_err());
    });
}

#[derive(Debug)]
struct InvalidHelper(Arc<World>);
impl crate::github::http::Transport for InvalidHelper {
    fn request(&self, request: crate::github::http::Request<'_>) -> Result<Vec<u8>> {
        let payload: Value = request
            .body
            .map_or(Value::Null, |body| serde_json::from_slice(body).unwrap());
        if payload["query"]
            .as_str()
            .is_some_and(|query| query.starts_with("query PublicationRefs"))
        {
            Ok(b"invalid".to_vec())
        } else {
            self.0.request(request)
        }
    }
}
#[test]
fn failed_helper_evidence_stops_before_publication() {
    let world = World::new(1);
    world.run(&["--no-stack"]).unwrap();
    {
        let mut memory = world.0.lock().unwrap();
        memory.discovery_api = DiscoveryApi::Supported;
        memory.trace.clear();
    }
    let mut app = world.app();
    app.transport = Arc::new(InvalidHelper(world.clone()));
    assert!(
        app.execute(
            &crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap(),
            &BTreeMap::from([("HOME".into(), "/unit/home".into())])
        )
        .is_err()
    );
    assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
}

#[derive(Debug)]
struct LostPush(Arc<World>);
impl crate::commands::CommandRunner for LostPush {
    fn edit_messages(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        messages: &[(String, String)],
    ) -> Result<crate::commands::CommandOutput> {
        self.0.edit_messages(program, args, cwd, env, messages)
    }
    fn execute(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    ) -> Result<crate::commands::CommandOutput> {
        let result = self.0.execute(program, args, cwd, env)?;
        if program == "git" && args.first().is_some_and(|arg| arg == "push") {
            bail!("lost push acknowledgement");
        }
        Ok(result)
    }
    fn execute_data(
        &self,
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
        input: &[u8],
    ) -> Result<crate::commands::StructuredOutput> {
        self.0.execute_data(program, args, cwd, env, input)
    }
}

#[test]
fn api_discovery_retains_authoritative_git_recovery_after_lost_push_acknowledgement() {
    let world = World::new(1);
    world.0.lock().unwrap().discovery_api = DiscoveryApi::Supported;
    let mut app = world.app();
    app.commands.runner = Arc::new(LostPush(world.clone()));
    app.execute(
        &crate::cli::Flags::parse(vec!["--no-stack".into()]).unwrap(),
        &BTreeMap::from([("HOME".into(), "/unit/home".into())]),
    )
    .unwrap();
    assert_eq!(world.0.lock().unwrap().prs.len(), 1);
    assert!(
        world
            .trace()
            .iter()
            .any(|event| event["args"][0] == "ls-remote")
    );
    assert_eq!(
        world.0.lock().unwrap().remote_refs["refs/heads/topic/1"],
        commit(1).hash
    );
}

#[test]
fn partial_graphql_errors_cannot_establish_publication_absence() {
    let world = World::new(1);
    world.http("POST graphql", vec![crate::tests::unit::support::Response::Output(
        200,
        json!({"errors":[{"type":"NOT_FOUND","message":"repository unavailable"}],"data":{"repository":{"nameWithOwner":"org/repo","publicationRef0":null}}}).to_string(),
    )]);
    with_github(&world, |github| {
        github.prepare_discovery(Vec::new(), BTreeSet::new());
        assert!(github.observe(&["topic/1".into()]).is_err());
        assert!(guard(&github.session.discovery).tips.is_empty());
        assert!(guard(&github.session.commit_prs).is_empty());
    });
}
