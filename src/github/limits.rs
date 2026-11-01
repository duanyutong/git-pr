//! Request scheduling policy; all time and server observations are supplied by callers.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(crate) const RETRIES: usize = 3;
pub(super) const FRESHNESS: Duration = Duration::from_secs(30);
pub(super) const WAIT_BUDGET: Duration = Duration::from_mins(15);
pub(super) fn remaining(started: Instant, now: Instant) -> Duration {
    WAIT_BUDGET.saturating_sub(now.saturating_duration_since(started))
}

pub(crate) fn check_window(
    now: Instant,
    delay: Duration,
    reason: WaitReason,
    observed: Option<Instant>,
    recovery_started: Option<Instant>,
) -> anyhow::Result<()> {
    if let Some(started) = recovery_started
        && (remaining(started, now).is_zero() || delay > remaining(started, now))
    {
        return Err(Exhausted(
            "automatic recovery exceeded its cumulative wait budget; no further request was sent"
                .into(),
        )
        .into());
    }
    if observed.is_some_and(|acquired| {
        now.saturating_duration_since(acquired)
            .saturating_add(delay)
            > FRESHNESS
    }) {
        return Err(RefreshAfterWait { delay, reason }.into());
    }
    Ok(())
}

pub(super) const CONTENT_BATCH: usize = 5;

#[derive(Clone, Debug, Default)]
pub(crate) struct Quota {
    pub remaining: Option<u64>,
    pub limit: Option<u64>,
    pub reset: Option<Duration>,
    pub retry_after: Option<Duration>,
    pub elapsed: Duration,
}

impl Quota {
    pub(super) fn delay(&self) -> Option<Duration> {
        self.retry_after
            .map(|delay| delay.saturating_sub(self.elapsed))
            .max(
                (self.remaining == Some(0))
                    .then_some(self.reset)
                    .flatten()
                    .map(|delay| {
                        delay
                            .saturating_add(Duration::from_secs(1))
                            .saturating_sub(self.elapsed)
                    }),
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitReason {
    Mutation,
    Primary,
    Secondary,
    Minute,
    Hour,
}
impl WaitReason {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Mutation => "GitHub mutation spacing",
            Self::Primary => "GitHub primary quota reset",
            Self::Secondary => "GitHub rate-limit cooldown",
            Self::Minute => "Local content budget (60 writes/minute)",
            Self::Hour => "Local content budget (450 writes/hour)",
        }
    }
    pub(super) const fn retry_budget(self) -> bool {
        !matches!(self, Self::Minute | Self::Hour)
    }
}
impl std::fmt::Display for WaitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Default)]
pub(super) struct Schedule {
    last_write: Option<Instant>,
    quotas: [Option<(Instant, Quota)>; 2],
    writes: VecDeque<(Instant, usize)>,
    cooldown: Option<(Instant, Duration)>,
}

impl Schedule {
    pub(super) fn observe(&mut self, now: Instant, graphql: bool, quota: Quota) {
        if quota.remaining.is_some() {
            self.quotas[usize::from(graphql)] = Some((now, quota));
        }
    }

    pub(super) const fn throttle(&mut self, now: Instant, delay: Duration) {
        self.cooldown = Some((now, delay));
    }

    pub(super) fn ready(
        &self,
        now: Instant,
        graphql: bool,
        writes: usize,
    ) -> (Duration, WaitReason) {
        let mut wait = (Duration::ZERO, WaitReason::Mutation);
        let mut consider = |delay, reason| {
            if delay > wait.0 {
                wait = (delay, reason);
            }
        };
        if let Some((observed, duration)) = self.cooldown {
            consider(
                duration.saturating_sub(now.saturating_duration_since(observed)),
                WaitReason::Secondary,
            );
        }
        if let Some((observed, quota)) = &self.quotas[usize::from(graphql)]
            && quota.remaining == Some(0)
            && let Some(reset) = quota.reset
        {
            consider(
                reset
                    .saturating_add(Duration::from_secs(1))
                    .saturating_sub(quota.elapsed)
                    .saturating_sub(now.saturating_duration_since(*observed)),
                WaitReason::Primary,
            );
        }
        if writes != 0 {
            if let Some(last) = self.last_write {
                consider(
                    Duration::from_secs(1).saturating_sub(now.saturating_duration_since(last)),
                    WaitReason::Mutation,
                );
            }
            consider(
                self.window_wait(now, writes, 60, Duration::from_secs(60)),
                WaitReason::Minute,
            );
            consider(
                self.window_wait(now, writes, 450, Duration::from_secs(3600)),
                WaitReason::Hour,
            );
        }
        wait
    }

    fn window_wait(
        &self,
        now: Instant,
        writes: usize,
        capacity: usize,
        window: Duration,
    ) -> Duration {
        let active = || {
            self.writes
                .iter()
                .filter(|(at, _)| now.saturating_duration_since(*at) < window)
        };
        let mut total = active().map(|(_, count)| count).sum::<usize>() + writes;
        for (at, count) in active() {
            if total <= capacity {
                break;
            }
            total -= count;
            if total <= capacity {
                return window.saturating_sub(now.saturating_duration_since(*at));
            }
        }
        Duration::ZERO
    }

    pub(super) fn reserve(&mut self, now: Instant, writes: usize) {
        self.writes
            .retain(|(at, _)| now.saturating_duration_since(*at) < Duration::from_secs(3600));
        if writes != 0 {
            self.writes.push_back((now, writes));
        }
    }

    pub(super) const fn completed(&mut self, now: Instant, writes: usize) {
        if writes != 0 {
            self.last_write = Some(now);
        }
    }
}

pub(super) fn rate_message(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("rate limit")
        || message.contains("rate_limit")
        || message.contains("secondary rate")
        || message.contains("abuse detection")
}

pub(super) fn backoff(attempt: usize, throttled: bool) -> Duration {
    Duration::from_secs((if throttled { 60 } else { 1 }) << attempt.min(RETRIES))
}

/// Only definite rejection or an unsent request permits another mutation attempt.
pub(crate) fn deferral(error: &anyhow::Error, attempt: usize) -> Option<(Duration, WaitReason)> {
    if let Some(deferred) = error.downcast_ref::<RefreshAfterWait>() {
        return Some((deferred.delay, deferred.reason));
    }
    error
        .downcast_ref::<Throttled>()
        .filter(|limit| !limit.partial)
        .map(|limit| {
            (
                if limit.backoff {
                    backoff(attempt, true)
                } else {
                    limit.delay
                },
                WaitReason::Secondary,
            )
        })
}

pub(super) fn rejected(error: &anyhow::Error) -> bool {
    error.is::<RefreshAfterWait>()
        || error.is::<Exhausted>()
        || error
            .downcast_ref::<Throttled>()
            .is_some_and(|limit| !limit.partial)
        || error
            .downcast_ref::<super::http::HttpStatusError>()
            .is_some_and(|status| {
                matches!(status.status.as_u16(), 400..=499) && status.status.as_u16() != 408
            })
}

/// A rate-limited GraphQL response may contain already completed mutations.
#[derive(Debug)]
pub(super) struct Throttled {
    pub delay: Duration,
    pub partial: bool,
    pub backoff: bool,
    pub message: String,
}

impl std::fmt::Display for Throttled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GitHub rate limit: {}; retry after {} seconds",
            self.message,
            self.delay.as_secs()
        )
    }
}
impl std::error::Error for Throttled {}

#[derive(Debug)]
pub(super) struct Exhausted(pub String);
impl std::fmt::Display for Exhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Exhausted {}

#[derive(Debug)]
pub(super) struct RefreshAfterWait {
    pub delay: Duration,
    pub reason: WaitReason,
}
impl std::fmt::Display for RefreshAfterWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}; content must be refreshed after waiting",
            self.reason
        )
    }
}
impl std::error::Error for RefreshAfterWait {}

#[cfg(test)]
#[path = "../tests/unit/limits.rs"]
mod unit;
