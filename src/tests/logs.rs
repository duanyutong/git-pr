use super::*;
use std::time::Duration;

#[test]
fn timestamps_preserve_utc_and_fixed_truncated_millisecond_precision() {
    for (seconds, nanos, expected) in [
        (0, 0, "1970-01-01 00:00:00.000 UTC"),
        (1_717_237_800, 123_456_789, "2024-06-01 10:30:00.123 UTC"),
        (1_717_237_800, 999_999_999, "2024-06-01 10:30:00.999 UTC"),
    ] {
        assert_eq!(
            timestamp(UNIX_EPOCH + Duration::new(seconds, nanos)).unwrap(),
            expected
        );
    }
    assert!(timestamp(UNIX_EPOCH - Duration::from_nanos(1)).is_err());
}
