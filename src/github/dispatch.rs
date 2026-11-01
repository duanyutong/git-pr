//! Serial request execution, quota observations and bounded retries.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;

use super::{
    GitHub,
    limits::{self, Quota, Throttled},
};
use crate::github::http::{HttpStatusError, IncompleteResponse, Request, Response};

impl GitHub<'_> {
    pub(crate) fn now(&self) -> Instant {
        self.clock
            .map_or_else(Instant::now, crate::commands::Clock::now)
    }

    pub(super) fn wait_for(&self, delay: Duration, reason: &str) -> Result<()> {
        let clock = self
            .clock
            .context("cannot retry GitHub request: no wait clock is available")?;
        self.wait_with_clock(clock, delay, reason);
        Ok(())
    }

    pub(crate) fn wait_for_recovery(
        &self,
        delay: Duration,
        reason: &str,
        started: Instant,
    ) -> Result<()> {
        let remaining = limits::remaining(started, self.now());
        if remaining.is_zero() || delay > remaining {
            return Err(limits::Exhausted("automatic recovery exceeded its cumulative wait budget; no further request was sent".into()).into());
        }
        self.wait_for(delay, reason)
    }

    fn wait_with_clock(&self, clock: &dyn crate::commands::Clock, delay: Duration, reason: &str) {
        let _activity = self.commands.activity(reason, Some(delay));
        let wait = self.commands.diagnostics.start("wait", || {
            format!("{reason}; requested {:.3}s", delay.as_secs_f64())
        });
        clock.sleep(delay);
        wait.finish(|| "done".into());
    }

    fn send_request(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
        writes: usize,
        attempt: usize,
    ) -> Result<Response> {
        self.commands.debug(&format!("-> {method} {url}"));
        if self.commands.verbose
            && let Some(body) = body
        {
            self.commands.debug(&String::from_utf8_lossy(body));
        }
        let endpoint = crate::diagnostics::errors::redact_url_credentials(url);
        let endpoint = endpoint.split(['?', '#']).next().unwrap_or_default();
        let span = self.commands.diagnostics.start("http", || {
            format!(
                "{method} {endpoint}; {}; attempt {}",
                if writes > 0 { "mutation" } else { "read" },
                attempt + 1
            )
        });
        let acquired = self.now();
        let response = self
            .transport
            .response(Request {
                method,
                url,
                token: &self.config.token.0,
                body,
                timeout: self.config.timeout,
            })
            .map(|mut response| {
                response.acquired = acquired;
                response
            });
        span.finish(|| match &response {
            Ok(response) => format!("HTTP {}", response.status),
            Err(error) => error
                .downcast_ref::<HttpStatusError>()
                .map_or_else(|| "error".into(), |e| format!("HTTP {}", e.status.as_u16())),
        });
        response
    }

    fn observe_quota(
        &self,
        schedule: &mut limits::Schedule,
        response: &Result<Response>,
        graphql: bool,
    ) -> Quota {
        let quota = match response {
            Ok(response) => response.quota.clone(),
            Err(error) => error
                .downcast_ref::<HttpStatusError>()
                .map(|e| &e.quota)
                .or_else(|| error.downcast_ref::<IncompleteResponse>().map(|e| &e.quota))
                .cloned()
                .unwrap_or_default(),
        };
        if let (Some(remaining), Some(limit), Some(reset)) =
            (quota.remaining, quota.limit, quota.reset)
        {
            self.commands.terminal.quota(
                if graphql { "GraphQL" } else { "REST" },
                remaining,
                limit,
                reset.saturating_sub(quota.elapsed),
            );
        }
        schedule.observe(self.now(), graphql, quota.clone());
        if let Some(delay) = quota.retry_after {
            schedule.throttle(self.now(), delay.saturating_sub(quota.elapsed));
        }
        quota
    }

    pub(super) fn request_url_encoded(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
        writes: usize,
        observed: Option<Instant>,
        recovery_started: Option<Instant>,
    ) -> Result<Response> {
        let queue = self
            .commands
            .diagnostics
            .start("queue", || "GitHub dispatch".into());
        let mut schedule = self.session.dispatch();
        queue.finish(|| "ready".into());
        let graphql = url.ends_with("/graphql");
        let _activity = if graphql && writes > 0 {
            self.commands
                .activity(&format!("Updating {writes} PR(s)"), None)
        } else {
            self.commands.activity_with_detail(
                &format!("GitHub {method}"),
                url.split("/repos/").last().unwrap_or(url),
            )
        };
        let mut waited = Duration::ZERO;
        let mut attempt = 0;
        let result = loop {
            let now = self.now();
            let (delay, reason) = schedule.ready(now, graphql, writes);
            if let Err(error) = limits::check_window(now, delay, reason, observed, recovery_started)
            {
                break Err(error);
            }
            if let Some(clock) = self.clock.filter(|_| !delay.is_zero()) {
                if reason.retry_budget() && delay > limits::WAIT_BUDGET.saturating_sub(waited) {
                    break Err(limits::Exhausted(format!("{reason}: required wait {}s exceeds the remaining automatic wait budget ({}s); no request was sent early. Retry after the indicated reset", delay.as_secs(), limits::WAIT_BUDGET.saturating_sub(waited).as_secs())).into());
                }
                self.wait_with_clock(clock, delay, reason.label());
                if reason.retry_budget() {
                    waited += delay;
                }
            }
            if let Err(error) = limits::check_window(
                self.now(),
                Duration::ZERO,
                reason,
                observed,
                recovery_started,
            ) {
                break Err(error);
            }
            schedule.reserve(self.now(), writes);
            let response = self.send_request(method, url, body, writes, attempt);
            schedule.completed(self.now(), writes);
            let quota = self.observe_quota(&mut schedule, &response, graphql);
            let response =
                response.and_then(|response| graphql_result(response, graphql, &quota, attempt));
            // A partial acknowledgement prevents replay, but still constrains later requests.
            if let Err(error) = &response
                && let Some(throttled) = error.downcast_ref::<Throttled>()
            {
                schedule.throttle(self.now(), throttled.delay);
            }
            if let Err(error) = &response
                && let Some((delay, reason)) = retry(error, writes, attempt)
            {
                let delay = delay.max(quota.delay().unwrap_or_default());
                schedule.throttle(self.now(), delay);
                if observed.is_some() {
                    let backoff = error
                        .downcast_ref::<HttpStatusError>()
                        .is_some_and(|e| e.quota.delay().is_none())
                        || error.downcast_ref::<Throttled>().is_some_and(|e| e.backoff);
                    break response.map_err(|cause| {
                        let message = cause.to_string();
                        cause.context(Throttled {
                            delay,
                            partial: false,
                            backoff,
                            message,
                        })
                    });
                }
                if attempt == limits::RETRIES || delay > limits::WAIT_BUDGET.saturating_sub(waited)
                {
                    break response.with_context(|| limits::Exhausted(format!("GitHub retry exhausted after {} attempt(s) and {:.0}s waiting; next permitted retry requires {}s ({reason}). Inspect the operation before retrying", attempt + 1, waited.as_secs_f64(), delay.as_secs())));
                }
                let reason = format!("{reason}; retry {}/{}", attempt + 1, limits::RETRIES);
                let wait = recovery_started.map_or_else(
                    || self.wait_for(delay, &reason),
                    |started| self.wait_for_recovery(delay, &reason, started),
                );
                if let Err(wait_error) = wait {
                    break response.with_context(|| {
                        format!("automatic retry could not wait: {wait_error:#}")
                    });
                }
                waited += delay;
                attempt += 1;
                continue;
            }
            break response;
        };
        drop(schedule);
        if let Ok(response) = &result {
            self.log_response(response);
        }
        result.with_context(|| format!("GitHub {method} {url} failed"))
    }
    fn log_response(&self, response: &Response) {
        if self.commands.verbose {
            self.commands
                .debug(&String::from_utf8_lossy(&response.body));
        }
        if let Some(id) = response
            .headers
            .get("x-github-request-id")
            .and_then(|v| v.to_str().ok())
        {
            self.commands.debug(&format!("GitHub request {id}"));
        }
    }
}

pub(super) struct PartialGraphql {
    pub(super) data: Value,
    pub(super) acquired: Instant,
}

impl std::fmt::Debug for PartialGraphql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PartialGraphql { .. }")
    }
}

impl std::fmt::Display for PartialGraphql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GraphQL response contained partial results")
    }
}

impl std::error::Error for PartialGraphql {}

fn graphql_result(
    response: Response,
    graphql: bool,
    quota: &Quota,
    attempt: usize,
) -> Result<Response> {
    if graphql
        && response.body.windows(8).any(|bytes| bytes == b"\"errors\"")
        && let Ok(mut value) = serde_json::from_slice::<Value>(&response.body)
        && let Some(errors) = value
            .get("errors")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        && limits::rate_message(&errors.to_string())
    {
        let message = errors.to_string();
        let data = value["data"].take();
        let partial = !data.is_null();
        let error: anyhow::Error = Throttled {
            delay: quota
                .delay()
                .unwrap_or_else(|| limits::backoff(attempt, true))
                .max(Duration::from_secs(1)),
            partial,
            backoff: quota.delay().is_none(),
            message,
        }
        .into();
        return Err(if partial {
            error.context(PartialGraphql {
                data,
                acquired: response.acquired,
            })
        } else {
            error
        });
    }
    Ok(response)
}

fn retry(error: &anyhow::Error, writes: usize, attempt: usize) -> Option<(Duration, &'static str)> {
    if let Some(error) = error.downcast_ref::<Throttled>() {
        return (writes == 0 || !error.partial).then_some((error.delay, "GitHub rate-limit retry"));
    }
    if let Some(error) = error.downcast_ref::<HttpStatusError>() {
        let code = error.status.as_u16();
        if code == 429
            || (code == 403 && (error.quota.delay().is_some() || limits::rate_message(&error.body)))
        {
            return Some((
                error
                    .quota
                    .delay()
                    .unwrap_or_else(|| limits::backoff(attempt, true))
                    .max(Duration::from_secs(1)),
                "GitHub rate-limit retry",
            ));
        }
        return (writes == 0 && matches!(code, 408 | 500 | 502 | 503 | 504)).then(|| {
            (
                limits::backoff(attempt, false),
                "GitHub transient read failure",
            )
        });
    }
    (writes == 0 && error.is::<crate::github::http::TransientRead>())
        .then(|| (limits::backoff(attempt, false), "GitHub connection retry"))
}

#[cfg(test)]
#[path = "../tests/unit/dispatch.rs"]
mod unit;
