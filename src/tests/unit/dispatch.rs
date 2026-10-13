use super::*;
use crate::{github::Session, github::http::Transport, tests::unit::support::World};
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};

#[derive(Clone, Debug)]
struct Reply {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: String,
}

impl Reply {
    fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.into(),
            headers: reqwest::header::HeaderMap::new(),
        }
    }
    fn header(mut self, key: &'static str, value: &str) -> Self {
        self.headers.insert(key, value.parse().unwrap());
        self
    }
}

#[derive(Debug)]
struct Service {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<usize>,
}

impl Transport for Service {
    fn request(&self, _: Request<'_>) -> Result<Vec<u8>> {
        unreachable!()
    }
    fn response(&self, request: Request<'_>) -> Result<Response> {
        *self.calls.lock().unwrap() += 1;
        let mut replies = self.replies.lock().unwrap();
        let reply = if replies.len() > 1 {
            replies.pop_front().unwrap()
        } else {
            replies.front().unwrap().clone()
        };
        if reply.status >= 400 {
            return Err(HttpStatusError {
                url: request.url.into(),
                status: reqwest::StatusCode::from_u16(reply.status).unwrap(),
                quota: crate::github::http::quota(&reply.headers),
                body: reply.body,
            }
            .into());
        }
        Ok(Response {
            status: reply.status,
            headers: reply.headers,
            body: reply.body.into_bytes(),
        })
    }
}

fn run(replies: Vec<Reply>, action: impl FnOnce(&GitHub<'_>, &World, &Service)) {
    let world = World::new(1);
    let config = world.config();
    let commands = world.commands();
    let service = Service {
        replies: Mutex::new(replies.into()),
        calls: Mutex::new(0),
    };
    let github = GitHub {
        config: &config,
        commands: &commands,
        session: Session::default(),
        transport: &service,
        clock: Some(world.as_ref()),
        template: "",
    };
    action(&github, &world, &service);
}

fn waits(world: &World) -> Vec<u64> {
    world
        .trace()
        .iter()
        .filter(|v| v["kind"] == "wait")
        .map(|v| v["millis"].as_u64().unwrap())
        .collect()
}

#[test]
fn explicit_throttles_and_secondary_errors_retry_but_permission_errors_do_not() {
    for reply in [
        Reply::new(429, "limited").header("retry-after", "2"),
        Reply::new(403, "secondary rate limit"),
        Reply::new(
            200,
            r#"{"errors":[{"type":"RATE_LIMITED","message":"limited"}]}"#,
        ),
    ] {
        run(vec![reply, Reply::new(200, "{}")], |g, w, s| {
            g.request_url(
                "POST",
                "https://api.github.com/graphql",
                Some(&json!({"query":"mutation { x }"})),
            )
            .unwrap();
            assert_eq!(*s.calls.lock().unwrap(), 2);
            assert!(matches!(waits(w).as_slice(), [2000 | 60000]));
        });
    }
    run(
        vec![Reply::new(403, "SSO authorisation required")],
        |g, w, s| {
            assert!(g.request("GET", "pulls/1", None).is_err());
            assert_eq!(*s.calls.lock().unwrap(), 1);
            assert_eq!(waits(w), [] as [u64; 0]);
        },
    );
}

#[test]
fn retries_are_bounded_and_large_server_deadlines_are_never_shortened() {
    run(vec![Reply::new(429, "limited")], |g, w, s| {
        let error = format!("{:#}", g.request("GET", "pulls/1", None).unwrap_err());
        assert!(error.contains("exhausted after 4 attempt"), "{error}");
        assert_eq!(*s.calls.lock().unwrap(), 4);
        assert_eq!(waits(w), [60_000, 120_000, 240_000]);
    });
    run(
        vec![Reply::new(429, "limited").header("retry-after", "3600")],
        |g, w, s| {
            assert!(
                format!("{:#}", g.request("GET", "pulls/1", None).unwrap_err()).contains("3600s")
            );
            assert_eq!(waits(w), [] as [u64; 0]);
            assert_eq!(*s.calls.lock().unwrap(), 1);
        },
    );
}

#[test]
fn successful_headers_prevent_quota_exhaustion_on_the_next_request() {
    let exhausted = Reply::new(200, "{}")
        .header("date", "Thu, 15 Oct 2026 12:00:00 GMT")
        .header("x-ratelimit-limit", "5000")
        .header("x-ratelimit-remaining", "0")
        .header("x-ratelimit-reset", "1792065610");
    run(vec![exhausted.clone(), Reply::new(200, "{}")], |g, w, s| {
        g.request("GET", "pulls/1", None).unwrap();
        g.request("GET", "pulls/2", None).unwrap();
        assert_eq!(*s.calls.lock().unwrap(), 2);
        assert_eq!(waits(w), [11_000]);
    });
    run(
        vec![exhausted.header("x-ratelimit-reset", "1792072800")],
        |g, w, s| {
            g.request("GET", "pulls/1", None).unwrap();
            let error = format!("{:#}", g.request("GET", "pulls/2", None).unwrap_err());
            assert!(error.contains("no request was sent early"), "{error}");
            assert_eq!(*s.calls.lock().unwrap(), 1);
            assert_eq!(waits(w), [] as [u64; 0]);
        },
    );
}

#[test]
fn transient_reads_retry_while_uncertain_mutations_require_reconciliation() {
    run(
        vec![Reply::new(502, "gateway"), Reply::new(200, "{}")],
        |g, w, s| {
            g.request("GET", "pulls/1", None).unwrap();
            assert_eq!(*s.calls.lock().unwrap(), 2);
            assert_eq!(waits(w), [1000]);
        },
    );
    run(vec![Reply::new(502, "gateway")], |g, w, s| {
        assert!(g.request("POST", "pulls", Some(&json!({}))).is_err());
        assert_eq!(*s.calls.lock().unwrap(), 1);
        assert_eq!(waits(w), [] as [u64; 0]);
    });
    run(
        vec![Reply::new(
            200,
            r#"{"data":{"p0":{"pullRequest":{"number":1}}},"errors":[{"type":"RATE_LIMITED"}]}"#,
        )],
        |g, w, s| {
            let error = g
                .request_url(
                    "POST",
                    "https://api.github.com/graphql",
                    Some(&json!({"query":"mutation { x }"})),
                )
                .unwrap_err();
            assert!(error.downcast_ref::<Throttled>().unwrap().partial);
            let partial = error.downcast_ref::<PartialGraphql>().unwrap();
            assert_eq!(partial.data["p0"]["pullRequest"]["number"], 1);
            assert_eq!(
                partial.to_string(),
                "GraphQL response contained partial results"
            );
            assert_eq!(format!("{partial:?}"), "PartialGraphql { .. }");
            assert!(!format!("{error:#}").contains("pullRequest"));
            assert_eq!(*s.calls.lock().unwrap(), 1);
            assert_eq!(waits(w), [] as [u64; 0]);
        },
    );
}

#[test]
fn retry_metadata_accepts_dates_and_does_not_treat_other_graphql_errors_as_limits() {
    let reply = Reply::new(200, "{}")
        .header("date", "Thu, 15 Oct 2026 12:00:00 GMT")
        .header("retry-after", "Thu, 15 Oct 2026 12:00:03 GMT");
    assert_eq!(
        crate::github::http::quota(&reply.headers).retry_after,
        Some(Duration::from_secs(3))
    );
    for body in [
        "not json",
        r#"{"errors":[]}"#,
        r#"{"errors":[{"message":"validation failed"}]}"#,
    ] {
        run(
            vec![Reply::new(200, body).header("x-ratelimit-remaining", "0")],
            |g, w, s| {
                g.request_url(
                    "POST",
                    "https://api.github.com/graphql",
                    Some(&json!({"query":"query { x }"})),
                )
                .unwrap();
                assert_eq!(*s.calls.lock().unwrap(), 1);
                assert_eq!(waits(w), [] as [u64; 0]);
            },
        );
    }
    assert!(retry(&anyhow::anyhow!("invalid observation"), 0, 0).is_none());
    run(vec![Reply::new(429, "limited")], |g, _, _| {
        let g = GitHub {
            clock: None,
            session: Session::default(),
            ..*g
        };
        assert!(
            format!("{:#}", g.request("GET", "pulls/1", None).unwrap_err())
                .contains("no wait clock")
        );
    });
}

#[test]
fn connection_failures_are_retryable_only_for_reads_and_graphql_quotas_are_reported() {
    let error = anyhow::Error::new(crate::github::http::TransientRead);
    assert_eq!(retry(&error, 0, 0).unwrap().0, Duration::from_secs(1));
    assert!(retry(&error, 1, 0).is_none());
    assert!(error.to_string().contains("connection"));
    let reply = Reply::new(200, "{}")
        .header("date", "Thu, 15 Oct 2026 12:00:00 GMT")
        .header("x-ratelimit-limit", "5000")
        .header("x-ratelimit-remaining", "100")
        .header("x-ratelimit-reset", "1792065610");
    run(vec![reply], |g, _, _| {
        g.request_url(
            "POST",
            "https://api.github.com/graphql",
            Some(&json!({"query":"query { x }"})),
        )
        .unwrap();
    });
}
