use super::*;
use crate::machine_protocol::plugin_step::{digest, fixture};
use std::time::Instant;

fn scope() -> PluginExecutionScope {
    PluginExecutionScope::new(Some("service-test"), "machine-test")
}

fn request(ttl_ms: i64) -> UninstallStep {
    UninstallStep {
        expires_at_ms: 1_000_000 + ttl_ms,
        ..fixture()
    }
}

fn sample(start: Instant, elapsed_ms: u64, wall_ms: i64) -> TimeSample {
    TimeSample::for_test(start + Duration::from_millis(elapsed_ms), wall_ms)
}

#[test]
fn lease_binds_the_complete_request_and_rejects_missing_or_wrong_owner() {
    let scope = scope();
    let request = request(60_000);
    let lease = scope.uninstall(&request).unwrap();
    assert!(lease.matches(&request));
    for field in [
        "operation_id",
        "service_id",
        "machine_id",
        "plugin_id",
        "plugin_version",
        "plan_digest",
        "generation_digest",
        "contract_fingerprint",
        "expires_at_ms",
    ] {
        let mut value = serde_json::to_value(&request).unwrap();
        value[field] = match field {
            "expires_at_ms" => (request.expires_at_ms + 1).into(),
            "plugin_version" => "1.0.1".into(),
            field if field.ends_with("digest") || field == "contract_fingerprint" => {
                digest(b"changed").into()
            }
            _ => "different-identity-0001".into(),
        };
        let changed = serde_json::from_value(value).unwrap();
        assert!(!lease.matches(&changed), "{field}");
    }
    let mut changed = request.clone();
    changed.schema = 2;
    changed.installation_revision = Some(
        format!("installation-{}", "a".repeat(64))
            .try_into()
            .unwrap(),
    );
    assert!(!lease.matches(&changed));
    for owner in [
        PluginExecutionScope::new(None, "machine-test"),
        PluginExecutionScope::new(Some("other-service"), "machine-test"),
        PluginExecutionScope::new(Some("service-test"), "other-machine"),
    ] {
        assert!(matches!(
            owner.uninstall(&request),
            Err(StepUnavailable::WrongOwner)
        ));
    }
    let mut invalid = request;
    invalid.plugin_id = "../invalid".into();
    assert!(matches!(
        scope.uninstall(&invalid),
        Err(StepUnavailable::InvalidRequest)
    ));
}

#[test]
fn dropping_the_connection_revokes_retained_leases_not_the_replacement_scope() {
    let old = scope();
    let first = old.uninstall(&request(60_000)).unwrap();
    let second = old.uninstall(&request(60_000)).unwrap();
    drop(old);
    let replacement = scope();
    let current = replacement.uninstall(&request(60_000)).unwrap();
    assert!(!first.connected());
    assert!(!second.connected());
    assert!(current.connected());
    drop(replacement);
    assert!(!current.connected());
}

#[test]
fn execution_budget_is_captured_on_receipt_and_capped_even_for_far_future_expiry() {
    let scope = scope();
    let start = Instant::now();
    let lease = scope
        .uninstall_at(&request(86_400_000), sample(start, 0, 1_000_000))
        .unwrap();
    assert!(!lease.expired_at(sample(start, 59_999, 1_000_001)));
    // The wall clock barely moved; the original process-monotonic cap still ends it.
    assert!(lease.expired_at(sample(start, 60_000, 1_000_002)));
    assert!(lease.before_effect().is_err());
}

#[test]
fn a_short_absolute_deadline_also_caps_monotonic_time() {
    let scope = scope();
    let start = Instant::now();
    let lease = scope
        .uninstall_at(&request(200), sample(start, 0, 1_000_000))
        .unwrap();
    assert!(!lease.expired_at(sample(start, 199, 1_000_001)));
    assert!(lease.expired_at(sample(start, 200, 1_000_002)));
    let wall = scope
        .uninstall_at(&request(200), sample(start, 0, 1_000_000))
        .unwrap();
    assert!(wall.expired_at(sample(start, 1, 1_000_200)));
}

#[test]
fn observed_wall_clock_rollback_or_forward_expiry_cannot_renew_a_lease() {
    let scope = scope();
    let start = Instant::now();
    let lease = scope
        .uninstall_at(&request(60_000), sample(start, 0, 1_000_000))
        .unwrap();
    assert!(!lease.expired_at(sample(start, 1, 1_000_100)));
    // Even a rollback that stays above the issue time fails closed.
    assert!(lease.expired_at(sample(start, 2, 1_000_050)));
    assert!(lease.expired_at(sample(start, 3, 1_000_200)));
    let forward = scope
        .uninstall_at(&request(60_000), sample(start, 0, 1_000_000))
        .unwrap();
    assert!(forward.expired_at(sample(start, 1, 1_060_000)));
    assert!(forward.expired_at(sample(start, 2, 1_000_001)));
}

#[test]
fn already_expired_or_invalid_clock_samples_never_gain_a_budget() {
    let scope = scope();
    let start = Instant::now();
    for wall in [1_000_000, 1_000_001, i64::MAX] {
        let lease = scope
            .uninstall_at(&request(0), sample(start, 0, wall))
            .unwrap();
        assert!(lease.expired_at(sample(start, 0, wall)));
    }
    for wall in [0, i64::MIN] {
        let lease = scope
            .uninstall_at(&request(60_000), sample(start, 0, wall))
            .unwrap();
        assert!(lease.expired_at(sample(start, 0, wall)));
        // The FIRST execution check may observe a repaired clock. The invalid
        // receipt-time sample must already have made rejection sticky.
        let repaired = scope
            .uninstall_at(&request(60_000), sample(start, 0, wall))
            .unwrap();
        assert!(repaired.expired_at(sample(start, 1, 1_000_001)));
    }
}
