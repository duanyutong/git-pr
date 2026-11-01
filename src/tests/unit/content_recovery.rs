use super::*;
use crate::{
    github::Session,
    github::http::{Request, Transport},
    tests::unit::support::{World, commit},
};
use std::sync::Mutex;

#[derive(Debug)]
struct Fault {
    world: Arc<World>,
    prefix: usize,
    failures: Mutex<usize>,
    skip_batches: Mutex<usize>,
    mode: &'static str,
    applied: Mutex<Vec<u64>>,
    attempted: Mutex<Vec<Vec<u64>>>,
    edit: Option<(&'static str, Value)>,
}

impl Transport for Fault {
    fn response(&self, request: Request<'_>) -> Result<crate::github::http::Response> {
        let body = self.request(request)?;
        Ok(crate::github::http::Response {
            acquired: std::time::Instant::now(),
            status: if body.is_empty() { 204 } else { 200 },
            headers: reqwest::header::HeaderMap::default(),
            quota: limits::Quota {
                retry_after: (self.mode == "throttle-long"
                    && body.windows(12).any(|bytes| bytes == b"RATE_LIMITED"))
                .then_some(std::time::Duration::from_secs(600)),
                ..Default::default()
            },
            body,
        })
    }
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        let payload: Value = request
            .body
            .map_or(Value::Null, |body| serde_json::from_slice(body).unwrap());
        if !payload["query"]
            .as_str()
            .is_some_and(|q| q.starts_with("mutation"))
        {
            return self.world.request(request);
        }
        let mut inputs: Vec<_> = payload["variables"].as_object().unwrap().iter().collect();
        inputs.sort_by_key(|(alias, _)| alias[1..].parse::<usize>().unwrap());
        let numbers: Vec<u64> = inputs
            .iter()
            .map(|(_, input)| {
                input["pullRequestId"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("PR_")
                    .parse()
                    .unwrap()
            })
            .collect();
        self.attempted.lock().unwrap().push(numbers.clone());
        let failed = {
            let mut failures = self.failures.lock().unwrap();
            let failed = *failures > 0
                && self.attempted.lock().unwrap().len() > *self.skip_batches.lock().unwrap();
            if failed {
                *failures -= 1;
            }
            failed
        };
        let count = if failed {
            self.prefix.min(inputs.len())
        } else {
            inputs.len()
        };
        let mut data = json!({});
        {
            let mut memory = self.world.0.lock().unwrap();
            for ((alias, input), number) in inputs.iter().zip(&numbers).take(count) {
                let pr = memory.prs.get_mut(number).unwrap();
                pr["title"] = input["title"].clone();
                pr["body"] = input["body"].clone();
                data[*alias] = json!({"pullRequest":{"number":number}});
                self.applied.lock().unwrap().push(*number);
            }
            if failed && let Some((key, value)) = &self.edit {
                let pr = memory.prs.get_mut(&numbers[0]).unwrap();
                if *key == "remove" {
                    memory.prs.remove(&numbers[0]);
                } else if *key == "base" {
                    pr["base"]["ref"] = value.clone();
                } else {
                    pr[*key] = value.clone();
                }
            }
            drop(memory);
        }
        if !failed {
            return Ok(serde_json::to_vec(&json!({"data":data}))?);
        }
        self.failure_response(request.url, &data, &numbers)
    }
}

impl Fault {
    fn failure_response(&self, url: &str, data: &Value, numbers: &[u64]) -> Result<Vec<u8>> {
        match self.mode {
            "FORBIDDEN"
            | "NOT_FOUND"
            | "UNPROCESSABLE"
            | "UNPROCESSABLE_ENTITY"
            | "BAD_USER_INPUT"
            | "GRAPHQL_VALIDATION_FAILED" => Ok(serde_json::to_vec(
                &json!({"data":data,"errors":[{"type":self.mode,"message":"Permission denied"}]}),
            )?),
            "partial" => Ok(serde_json::to_vec(
                &json!({"data":data,"errors":[{"message":"internal failure"}]}),
            )?),
            "throttle_reject" => Ok(serde_json::to_vec(
                &json!({"errors":[{"type":"RATE_LIMITED"}]}),
            )?),
            "throttle" | "throttle-long" => Ok(serde_json::to_vec(
                &json!({"data":data,"errors":[{"type":"RATE_LIMITED"}]}),
            )?),
            "stale_ack" => Ok(serde_json::to_vec(
                &json!({"data":{"p0":{"pullRequest":{"number":101}}},"errors":[{"message":"internal failure"}]}),
            )?),
            "malformed_ack" => Ok(serde_json::to_vec(
                &json!({"data":{"p0":{"pullRequest":{"number":numbers[0].to_string()}},"p1":null,"p99":{"pullRequest":{"number":numbers[0]}}},"errors":[{"message":"internal failure"}]}),
            )?),
            "malformed" => Ok(b"incomplete response".to_vec()),
            "lost" => bail!("connection reset after dispatch"),
            "truncated" | "oversized" => {
                let error = anyhow::Error::new(crate::github::http::IncompleteResponse {
                    quota: limits::Quota {
                        retry_after: Some(std::time::Duration::from_secs(2)),
                        ..Default::default()
                    },
                });
                Err(if self.mode == "truncated" {
                    error.context(crate::github::http::TransientRead)
                } else {
                    error
                })
            }
            _ => Err(crate::github::http::HttpStatusError {
                url: url.into(),
                status: reqwest::StatusCode::from_u16(
                    self.mode.split('-').next().unwrap().parse().unwrap(),
                )
                .unwrap(),
                body: "injected failure".into(),
                quota: limits::Quota {
                    retry_after: match self.mode {
                        "429-hint" => Some(std::time::Duration::from_secs(2)),
                        "429-long-hint" => Some(std::time::Duration::from_secs(600)),
                        _ => None,
                    },
                    ..limits::Quota::default()
                },
            }
            .into()),
        }
    }
}

fn fixture(
    count: u64,
    prefix: usize,
    failures: usize,
    mode: &'static str,
    edit: Option<(&'static str, Value)>,
    action: impl FnOnce(&GitHub<'_>, &Fault, &[Commit]),
) {
    let world = World::new(count);
    world.run(&["--no-stack"]).unwrap();
    let config = world.config();
    let commands = world.commands();
    let service = Fault {
        world: world.clone(),
        prefix,
        failures: Mutex::new(failures),
        skip_batches: Mutex::new(0),
        mode,
        applied: Mutex::default(),
        attempted: Mutex::default(),
        edit,
    };
    let github = GitHub {
        config: &config,
        commands: &commands,
        session: Session::default(),
        transport: &service,
        clock: Some(world.as_ref()),
        template: "",
    };
    let commits: Vec<_> = (1..=count)
        .map(|n| {
            let mut c = commit(n);
            c.pr_number = 100 + n;
            c.title = format!("Updated {n}");
            c
        })
        .collect();
    world.0.lock().unwrap().trace.clear();
    action(&github, &service, &commits);
}

fn update(g: &GitHub<'_>, commits: &[Commit]) -> Result<()> {
    g.update_content(&commits.iter().collect::<Vec<_>>(), &mut |_, pr| {
        let human = pr.body.split("\n<!-- fixture stack -->").next().unwrap();
        Ok(Some(format!(
            "{human}\n<!-- fixture stack -->\ncomplete listing"
        )))
    })
}

#[test]
fn acknowledged_content_updates_preserve_later_title_edits_during_partial_recovery() {
    use crate::tests::unit::live_progress::Recorder;
    for mode in ["partial", "throttle"] {
        for title in ["Feature 1", "Human renamed this"] {
            fixture(
                2,
                1,
                1,
                mode,
                Some(("title", json!(title))),
                |g, service, commits| {
                    let recorder = Recorder::new(service.world.clone());
                    let mut commands = g.commands.clone();
                    commands.terminal = recorder.clone();
                    let github = GitHub {
                        session: Session::default(),
                        commands: &commands,
                        ..*g
                    };
                    update(&github, commits).unwrap();
                    assert_eq!(*service.applied.lock().unwrap(), [101, 102]);
                    assert_eq!(service.world.0.lock().unwrap().prs[&101]["title"], title);
                    assert_eq!(
                        service.world.0.lock().unwrap().prs[&102]["title"],
                        "Updated 2"
                    );
                    assert!(github.was_updated(101) && github.was_updated(102));
                    let events = recorder.events.lock().unwrap();
                    assert!(
                        events
                            .iter()
                            .all(|event| event.total == 2 && event.done <= 2)
                    );
                    assert_eq!(events.last().unwrap().done, 2);
                    assert!(events.windows(2).all(|pair| pair[0].done <= pair[1].done));
                    drop(events);
                },
            );
        }
    }
}

#[test]
fn acknowledged_recovery_chunks_are_not_replayed_after_a_later_failure() {
    for mode in ["partial", "throttle"] {
        fixture(
            8,
            1,
            2,
            mode,
            Some(("title", json!("Human renamed this"))),
            |g, service, commits| {
                update(g, commits).unwrap();
                assert_eq!(
                    *service.applied.lock().unwrap(),
                    (101..=108).collect::<Vec<_>>()
                );
                let memory = service.world.0.lock().unwrap();
                assert_eq!(memory.prs[&101]["title"], "Human renamed this");
                assert_eq!(memory.prs[&102]["title"], "Human renamed this");
                assert_eq!(memory.prs[&108]["title"], "Updated 8");
            },
        );
    }
}

#[test]
fn stale_and_malformed_content_acknowledgements_leave_every_pending_alias_recoverable() {
    for mode in ["stale_ack", "malformed_ack"] {
        fixture(8, 0, 1, mode, None, |g, service, commits| {
            *service.skip_batches.lock().unwrap() = 1;
            update(g, commits).unwrap();
            assert_eq!(
                *service.applied.lock().unwrap(),
                (101..=108).collect::<Vec<_>>()
            );
            assert!(
                service
                    .world
                    .0
                    .lock()
                    .unwrap()
                    .prs
                    .iter()
                    .all(|(number, pr)| pr["title"] == format!("Updated {}", number - 100))
            );
            assert!((101..=108).all(|number| g.was_updated(number)));
        });
    }
}

#[test]
fn description_progress_counts_recovered_batches_once_and_never_completes_rejected_work() {
    use crate::tests::unit::live_progress::Recorder;
    for (mode, succeeds) in [("lost", true), ("FORBIDDEN", false)] {
        fixture(8, 2, 1, mode, None, |g, service, commits| {
            let recorder = Recorder::new(service.world.clone());
            let mut commands = g.commands.clone();
            commands.terminal = recorder.clone();
            let github = GitHub {
                session: Session::default(),
                commands: &commands,
                ..*g
            };
            assert_eq!(update(&github, commits).is_ok(), succeeds);
            let events = recorder.events.lock().unwrap().clone();
            assert!(events.windows(2).all(|pair| pair[0].done <= pair[1].done));
            assert!(events.iter().all(|event| event.total == 8));
            assert_eq!(events.last().unwrap().done, if succeeds { 8 } else { 0 });
            if succeeds {
                let mut applied = service.applied.lock().unwrap().clone();
                applied.sort_unstable();
                assert_eq!(applied, (101..=108).collect::<Vec<_>>());
            }
        });
    }
}

#[test]
fn every_partial_prefix_recovers_without_repeating_completed_writes_or_publication() {
    for prefix in 0..=5 {
        for mode in [
            "502",
            "504",
            "partial",
            "malformed",
            "lost",
            "throttle",
            "truncated",
            "oversized",
        ] {
            fixture(20, prefix, 1, mode, None, |g, service, commits| {
                update(g, commits).unwrap();
                assert!((101..=120).all(|number| g.was_updated(number)));
                let mut applied = service.applied.lock().unwrap().clone();
                applied.sort_unstable();
                assert_eq!(applied, (101..=120).collect::<Vec<_>>(), "{prefix}/{mode}");
                if matches!(mode, "truncated" | "oversized") {
                    assert_eq!(
                        service
                            .world
                            .trace()
                            .iter()
                            .filter(|event| event["kind"] == "wait" && event["millis"] == 2000)
                            .count(),
                        1
                    );
                }
                assert!(
                    service
                        .attempted
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|batch| batch.len() <= 5)
                );
                assert!(
                    !service.world.trace().iter().any(|v| v["args"][0] == "push"
                        || (v["path"] == "pulls" && v["method"] == "POST"))
                );
                let memory = service.world.0.lock().unwrap();
                assert!(
                    memory
                        .prs
                        .values()
                        .all(|pr| pr["body"].as_str().unwrap().ends_with("complete listing"))
                );
                drop(memory);
                let calls = service.attempted.lock().unwrap().len();
                update(g, commits).unwrap();
                assert_eq!(service.attempted.lock().unwrap().len(), calls);
            });
        }
    }
}

#[test]
fn recovery_preserves_fresh_unmanaged_text_and_rejects_identity_lifecycle_and_title_changes() {
    fixture(
        2,
        0,
        1,
        "502",
        Some(("body", json!("Fresh human notes"))),
        |g, s, c| {
            update(g, c).unwrap();
            assert!(
                s.world.0.lock().unwrap().prs[&101]["body"]
                    .as_str()
                    .unwrap()
                    .starts_with("Fresh human notes")
            );
        },
    );
    for (key, value) in [
        ("title", json!("Human renamed this")),
        ("node_id", json!("different")),
        ("state", json!("closed")),
        ("base", json!("different")),
        ("remove", json!(true)),
    ] {
        fixture(2, 0, 1, "502", Some((key, value)), |g, s, c| {
            let error = format!("{:#}", update(g, c).unwrap_err());
            assert!(error.contains("automatic recovery stopped"), "{error}");
            assert_eq!(s.attempted.lock().unwrap().len(), 1);
        });
    }
}

#[test]
fn later_batch_failures_recover_twelve_and_twenty_completed_updates_without_duplicates() {
    for (skip, prefix) in [(2, 2), (3, 5)] {
        fixture(20, prefix, 1, "502", None, |g, service, commits| {
            *service.skip_batches.lock().unwrap() = skip;
            update(g, commits).unwrap();
            let mut applied = service.applied.lock().unwrap().clone();
            applied.sort_unstable();
            assert_eq!(applied, (101..=120).collect::<Vec<_>>());
            assert_eq!(*service.failures.lock().unwrap(), 0);
            let calls = service.attempted.lock().unwrap().len();
            update(g, commits).unwrap();
            assert_eq!(service.attempted.lock().unwrap().len(), calls);
        });
    }
}

#[test]
fn repeated_failure_reports_outstanding_prs_and_permanent_errors_are_not_replayed() {
    fixture(2, 1, 999, "502", None, |g, s, c| {
        // Each retry completes another outstanding PR; progress is preserved.
        update(g, c).unwrap();
        assert_eq!(s.applied.lock().unwrap().len(), 2);
    });
    fixture(2, 0, 999, "502", None, |g, s, c| {
        let error = format!("{:#}", update(g, c).unwrap_err());
        assert!(error.contains("exhausted after 4 attempt"), "{error}");
        assert!(error.contains("#101, #102"));
        assert_eq!(s.attempted.lock().unwrap().len(), 4);
    });
    fixture(2, 0, 1, "422", None, |g, s, c| {
        assert!(format!("{:#}", update(g, c).unwrap_err()).contains("no unsafe replay"));
        assert_eq!(s.attempted.lock().unwrap().len(), 1);
    });
}

#[test]
fn long_pacing_and_rate_limit_retries_refresh_content_after_waiting() {
    for throttled in [false, true] {
        fixture(
            2,
            0,
            usize::from(throttled),
            "429",
            None,
            |g, service, commits| {
                service
                    .world
                    .0
                    .lock()
                    .unwrap()
                    .changes_on_wait
                    .insert(101, "Human edit during quota wait".into());
                if !throttled {
                    g.session.dispatch().observe(
                        g.now(),
                        true,
                        limits::Quota {
                            remaining: Some(0),
                            reset: Some(std::time::Duration::from_secs(60)),
                            ..limits::Quota::default()
                        },
                    );
                }
                update(g, commits).unwrap();
                assert!(
                    service.world.0.lock().unwrap().prs[&101]["body"]
                        .as_str()
                        .unwrap()
                        .starts_with("Human edit during quota wait")
                );
                assert_eq!(service.applied.lock().unwrap().len(), 2);
            },
        );
    }
}

#[test]
fn graphql_permission_errors_stop_without_replaying_the_failed_fields() {
    for kind in [
        "FORBIDDEN",
        "NOT_FOUND",
        "UNPROCESSABLE",
        "UNPROCESSABLE_ENTITY",
        "BAD_USER_INPUT",
        "GRAPHQL_VALIDATION_FAILED",
    ] {
        fixture(2, 0, 1, kind, None, |g, service, commits| {
            let error = format!("{:#}", update(g, commits).unwrap_err());
            assert!(error.contains(kind), "{error}");
            assert_eq!(service.attempted.lock().unwrap().len(), 1);
        });
    }
}

#[test]
fn content_throttles_preserve_server_delay_and_unavailable_waiting_stops_clearly() {
    for mode in ["429-hint", "throttle_reject"] {
        fixture(2, 0, 1, mode, None, |g, service, commits| {
            update(g, commits).unwrap();
            assert_eq!(service.applied.lock().unwrap().len(), 2);
        });
    }
    fixture(2, 0, 999, "502", None, |g, _, commits| {
        let g = GitHub {
            clock: None,
            session: Session::default(),
            ..*g
        };
        assert!(format!("{:#}", update(&g, commits).unwrap_err()).contains("no wait clock"));
    });
    fixture(2, 0, 0, "502", None, |g, service, commits| {
        g.session.dispatch().observe(
            g.now(),
            true,
            limits::Quota {
                remaining: Some(0),
                reset: Some(std::time::Duration::from_hours(1)),
                ..limits::Quota::default()
            },
        );
        let error = format!("{:#}", update(g, commits).unwrap_err());
        assert!(error.contains("budget"), "{error}");
        assert_eq!(service.applied.lock().unwrap().len(), 0);
    });
}

#[test]
fn recovery_preserves_bodies_when_only_titles_change_and_rejects_unrenderable_or_oversized_edits() {
    fixture(2, 0, 1, "502", None, |g, service, commits| {
        let before = service.world.0.lock().unwrap().prs[&101]["body"].clone();
        g.update_content(&commits.iter().collect::<Vec<_>>(), &mut |_, _| Ok(None))
            .unwrap();
        assert_eq!(service.world.0.lock().unwrap().prs[&101]["body"], before);
    });
    fixture(
        2,
        0,
        1,
        "502",
        Some(("body", json!("bad delimiters"))),
        |g, _, commits| {
            let error = g
                .update_content(&commits.iter().collect::<Vec<_>>(), &mut |_, pr| {
                    if pr.body.as_ref() == "bad delimiters" {
                        bail!("unmatched managed section");
                    }
                    Ok(None)
                })
                .unwrap_err();
            assert!(
                format!("{error:#}")
                    .contains("failed to prepare PR #101 description during recovery")
            );
        },
    );
    fixture(
        2,
        0,
        1,
        "502",
        Some(("body", json!("x".repeat(CONTENT_LIMIT)))),
        |g, service, commits| {
            let error = format!("{:#}", update(g, commits).unwrap_err());
            assert!(error.contains("1 MiB"), "{error}");
            assert_eq!(service.attempted.lock().unwrap().len(), 1);
        },
    );
}

#[test]
fn deferred_recovery_without_a_clock_stops_before_observing_or_writing() {
    fixture(1, 0, 0, "502", None, |g, service, commits| {
        let before = g.get(101).unwrap();
        let g = GitHub {
            clock: None,
            session: Session::default(),
            ..*g
        };
        let error = g
            .recover_content(
                vec![Change {
                    alias: 0,
                    commit: &commits[0],
                    after: Arc::clone(&before),
                    before,
                    acknowledged: false,
                }],
                &mut |_, _| Ok(None),
                limits::RefreshAfterWait {
                    delay: std::time::Duration::from_secs(60),
                    reason: limits::WaitReason::Primary,
                }
                .into(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("no wait clock"));
        assert!(service.attempted.lock().unwrap().is_empty());
    });
}

#[test]
fn description_recovery_has_one_budget_and_retains_completed_effects() {
    fixture(
        3,
        1,
        999,
        "throttle-long",
        None,
        |github, service, commits| {
            let started = github.now();
            let error = update(github, commits).unwrap_err();
            assert!(
                format!("{error:#}").contains("cumulative wait budget"),
                "{error:#}"
            );
            assert_eq!(github.now().duration_since(started).as_secs(), 600);
            assert_eq!(
                *service.attempted.lock().unwrap(),
                [vec![101, 102, 103], vec![102, 103]]
            );
            assert_eq!(*service.applied.lock().unwrap(), [101, 102]);
        },
    );
}

#[derive(Debug)]
struct RepeatedContentDeferral<'a> {
    world: &'a World,
    session: &'a Session,
}
impl crate::commands::Clock for RepeatedContentDeferral<'_> {
    fn now(&self) -> std::time::Instant {
        crate::commands::Clock::now(self.world)
    }
    fn sleep(&self, duration: std::time::Duration) {
        crate::commands::Clock::sleep(self.world, duration);
        self.session.dispatch().reserve(self.now(), 60);
    }
}
#[test]
fn exhausted_unsent_description_deferrals_do_not_wait_again() {
    fixture(1, 0, 0, "502", None, |original, service, commits| {
        let mut github = GitHub {
            clock: None,
            session: Session::default(),
            ..*original
        };
        let clock = RepeatedContentDeferral {
            world: &service.world,
            session: &github.session,
        };
        github.clock = Some(&clock);
        github.get(101).unwrap();
        github.session.dispatch().reserve(github.now(), 60);
        let error = update(&github, commits).unwrap_err();
        assert!(
            format!("{error:#}").contains("unsent deferral"),
            "{error:#}"
        );
        let waits = service
            .world
            .trace()
            .iter()
            .filter(|event| event["kind"] == "wait")
            .count();
        assert_eq!(waits, 3);
        assert!(service.attempted.lock().unwrap().is_empty());
        assert!(
            !github.commands.outcomes.snapshot()
                [&crate::model::outcome::Operation::Description(101)]
                .unconfirmed
        );
    });
}

#[test]
fn missing_acquisition_evidence_refuses_rest_and_graphql_content_mutations() {
    fixture(1, 0, 0, "502", None, |github, service, commits| {
        let before = github.get(101).unwrap();
        super::super::guard(&github.session.pr_times).clear();
        assert!(
            github
                .update_rest_content(
                    &commits[0],
                    before.clone(),
                    Arc::from("updated"),
                    &mut |_, _| Ok(None)
                )
                .is_err()
        );
        let mut changes = [Change {
            alias: 0,
            commit: &commits[0],
            before: before.clone(),
            after: before,
            acknowledged: false,
        }];
        assert!(github.send_recovery(&mut changes, github.now()).is_err());
        assert!(service.attempted.lock().unwrap().is_empty());
    });
}

#[test]
fn recovery_splits_growing_human_descriptions_by_encoded_bytes() {
    fixture(5, 0, 0, "502", None, |github, service, commits| {
        let human = "𐀀".repeat(60_000);
        for number in 101..=105 {
            service
                .world
                .0
                .lock()
                .unwrap()
                .changes_on_wait
                .insert(number, human.clone());
        }
        github.session.dispatch().reserve(github.now(), 60);
        update(github, commits).unwrap();
        assert_eq!(
            *service.attempted.lock().unwrap(),
            [vec![101, 102, 103, 104], vec![105]]
        );
        assert_eq!(*service.applied.lock().unwrap(), [101, 102, 103, 104, 105]);
        for pr in service.world.0.lock().unwrap().prs.values() {
            assert!(pr["body"].as_str().unwrap().starts_with(&human));
        }
    });
}
#[test]
fn unsent_description_failures_preserve_confirmed_work_without_claiming_uncertainty() {
    for rest in [false, true] {
        for confirmed in [false, true] {
            fixture(1, 0, 0, "502", None, |github, service, commits| {
                let mut before = github.get(101).unwrap();
                if rest {
                    Arc::make_mut(&mut before).node_id.clear();
                    github.session.prs().insert(101, before);
                }
                let operation = crate::model::outcome::Operation::Description(101);
                if confirmed {
                    github.commands.outcomes.confirm(operation.clone());
                }
                github.session.dispatch().reserve(github.now(), 450);
                assert!(update(github, commits).is_err());
                let effect = github.commands.outcomes.snapshot()[&operation];
                assert_eq!(effect.confirmed, confirmed);
                assert!(!effect.unconfirmed);
                assert!(service.attempted.lock().unwrap().is_empty());
                assert!(
                    !service
                        .world
                        .trace()
                        .iter()
                        .any(|event| event["method"] == "PATCH")
                );
            });
        }
    }
}

#[test]
fn fully_acknowledged_rate_limited_descriptions_preserve_the_next_request_deadline() {
    for count in [5, 20] {
        fixture(count, 5, 1, "throttle", None, |github, service, commits| {
            update(github, commits).unwrap();
            assert_eq!(
                service.attempted.lock().unwrap().len(),
                usize::try_from(count).unwrap() / 5
            );
            let mut applied = service.applied.lock().unwrap().clone();
            applied.sort_unstable();
            assert_eq!(applied, (101..101 + count).collect::<Vec<_>>());
            if count == 5 {
                assert!(
                    !service
                        .world
                        .trace()
                        .iter()
                        .any(|event| event["kind"] == "wait")
                );
                // Every alias was acknowledged, so no outcome read is necessary.
                assert_eq!(
                    service
                        .world
                        .trace()
                        .iter()
                        .filter(|event| event["kind"] == "http")
                        .count(),
                    5
                );
                github.request("GET", "pulls/101", None).unwrap();
            }
            let waits: Vec<_> = service
                .world
                .trace()
                .iter()
                .filter(|event| event["kind"] == "wait")
                .map(|event| event["millis"].as_u64().unwrap())
                .collect();
            assert_eq!(waits.iter().filter(|&&millis| millis == 60_000).count(), 1);
            assert!(waits.iter().all(|&millis| matches!(millis, 1000 | 60_000)));
            update(github, commits).unwrap();
            assert_eq!(
                *service.applied.lock().unwrap(),
                (101..101 + count).collect::<Vec<_>>()
            );
        });
    }
}
