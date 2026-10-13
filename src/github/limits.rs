//! Request scheduling policy; all time and server observations are supplied by callers.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(super) const RETRIES: usize = 3;
pub(super) const WAIT_BUDGET: Duration = Duration::from_mins(15);
pub(super) const CONTENT_BATCH: usize = 5;

#[derive(Clone, Debug, Default)]
pub(crate) struct Quota {
    pub remaining: Option<u64>,
    pub limit: Option<u64>,
    pub reset: Option<Duration>,
    pub retry_after: Option<Duration>,
}

impl Quota {
    pub(super) fn delay(&self) -> Option<Duration> {
        self.retry_after.max(
            (self.remaining == Some(0))
                .then_some(self.reset)
                .flatten()
                .map(|delay| delay.saturating_add(Duration::from_secs(1))),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WaitReason {
    Mutation,
    Primary,
    Secondary,
    Minute,
    Hour,
}
impl WaitReason {
    pub(super) const fn label(self) -> &'static str {
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
