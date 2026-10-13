use super::*;

#[test]
fn server_deadlines_take_precedence_and_missing_deadlines_use_bounded_backoff() {
    let quota = Quota {
        remaining: Some(0),
        reset: Some(Duration::from_secs(90)),
        retry_after: Some(Duration::from_secs(2)),
        ..Quota::default()
    };
    assert_eq!(quota.delay(), Some(Duration::from_secs(91)));
    assert_eq!(
        Quota {
            retry_after: Some(Duration::from_secs(100)),
            ..quota
        }
        .delay(),
        Some(Duration::from_secs(100))
    );
    assert_eq!(
        Quota {
            remaining: Some(1),
            ..quota
        }
        .delay(),
        Some(Duration::from_secs(2))
    );
    assert_eq!(Quota::default().delay(), None);
    assert_eq!(backoff(0, true), Duration::from_secs(60));
    assert_eq!(backoff(2, true), Duration::from_secs(240));
    assert_eq!(backoff(999, false), Duration::from_secs(8));
    for message in [
        "RATE_LIMITED",
        "API rate limit exceeded",
        "secondary rate limit",
        "Abuse detection mechanism",
    ] {
        assert!(rate_message(message));
    }
    assert!(!rate_message("resource inaccessible"));
    let error = Throttled {
        delay: Duration::from_secs(4),
        partial: true,
        backoff: false,
        message: "limited".into(),
    };
    assert!(error.to_string().contains("4 seconds"));
    assert_eq!(Exhausted("stopped".into()).to_string(), "stopped");
}

#[test]
fn scheduling_separates_primary_resources_and_shares_secondary_cooldown() {
    let now = Instant::now();
    let mut schedule = Schedule::default();
    schedule.observe(now, true, Quota::default());
    assert_eq!(schedule.ready(now, true, 0).0, Duration::ZERO);
    schedule.observe(
        now,
        true,
        Quota {
            remaining: Some(0),
            reset: Some(Duration::from_secs(10)),
            ..Quota::default()
        },
    );
    assert_eq!(schedule.ready(now, true, 0).0, Duration::from_secs(11));
    assert_eq!(schedule.ready(now, false, 0).0, Duration::ZERO);
    schedule.throttle(now, Duration::from_secs(20));
    assert_eq!(schedule.ready(now, false, 0).0, Duration::from_secs(20));
    assert_eq!(
        schedule.ready(now + Duration::from_secs(30), true, 0).0,
        Duration::ZERO
    );
    schedule.completed(now, 0);
    schedule.completed(now, 1);
    assert_eq!(
        schedule.ready(now + Duration::from_secs(30), false, 1).0,
        Duration::ZERO
    );
    schedule.completed(now + Duration::from_secs(30), 1);
    assert_eq!(
        schedule
            .ready(now + Duration::from_millis(30_500), false, 1)
            .0,
        Duration::from_millis(500)
    );
}

#[test]
fn content_windows_bound_alias_work_and_expire_without_delaying_reads() {
    let now = Instant::now();
    let mut schedule = Schedule::default();
    schedule.reserve(now, 0);
    for _ in 0..12 {
        schedule.reserve(now, 5);
    }
    assert_eq!(schedule.ready(now, false, 0).0, Duration::ZERO);
    assert_eq!(schedule.ready(now, false, 1).0, Duration::from_secs(60));
    assert_eq!(
        schedule.ready(now + Duration::from_secs(60), false, 1).0,
        Duration::ZERO
    );
    for minute in 1..8 {
        for _ in 0..11 {
            schedule.reserve(now + Duration::from_secs(minute * 60), 5);
        }
    }
    // 445 recent writes leave five units; the next five are permitted, six are not.
    let at = now + Duration::from_secs(600);
    assert_eq!(schedule.ready(at, false, 5).0, Duration::ZERO);
    assert_eq!(schedule.ready(at, false, 6).0, Duration::from_mins(50));
    assert_eq!(
        schedule.window_wait(now, 1000, 60, Duration::from_secs(60)),
        Duration::ZERO
    );
    schedule.reserve(now + Duration::from_secs(5000), 1);
    assert_eq!(schedule.writes.len(), 1);
}

#[test]
fn clock_defaults_and_wait_diagnostics_remain_available_to_boundary_adapters() {
    use crate::commands::Clock as _;
    let before = Instant::now();
    assert!(crate::commands::SystemClock.now() >= before);
    for reason in [
        WaitReason::Mutation,
        WaitReason::Primary,
        WaitReason::Secondary,
        WaitReason::Minute,
        WaitReason::Hour,
    ] {
        let error = RefreshAfterWait {
            delay: Duration::from_secs(60),
            reason,
        };
        assert!(error.to_string().contains(reason.label()));
        assert_eq!(
            reason.retry_budget(),
            !matches!(reason, WaitReason::Minute | WaitReason::Hour)
        );
    }
}
