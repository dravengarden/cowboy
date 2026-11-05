use super::{ScheduledResetFailurePolicy, scheduled_reset_failure_policy};

#[test]
fn ambiguous_consume_is_never_retried() {
    assert_eq!(
        scheduled_reset_failure_policy(true, 0),
        ScheduledResetFailurePolicy::StopUnknown
    );
}

#[test]
fn only_preflight_failures_receive_two_bounded_retries() {
    assert_eq!(
        scheduled_reset_failure_policy(false, 0),
        ScheduledResetFailurePolicy::RetryPreflight
    );
    assert_eq!(
        scheduled_reset_failure_policy(false, 1),
        ScheduledResetFailurePolicy::RetryPreflight
    );
    assert_eq!(
        scheduled_reset_failure_policy(false, 2),
        ScheduledResetFailurePolicy::StopFailed
    );
}
