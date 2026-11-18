use super::*;

fn control(state: AccountState, until_ms: Option<i64>) -> Control {
    Control {
        account_id: "11111111-1111-4111-8111-111111111111".into(),
        owner_user_id: "owner".into(),
        provider_id: "antigravity".into(),
        state,
        reason: state.as_str().into(),
        until_ms,
        updated_at_ms: 1,
        revision: 1,
    }
}

fn input(control: Option<&Control>, explicit: bool, now_ms: i64) -> GateInput<'_> {
    GateInput {
        admitted: true,
        control,
        uncertain: None,
        explicit,
        now_ms,
    }
}

#[test]
fn states_round_trip_and_backend_classes_map_to_states() {
    for state in AccountState::ALL {
        assert_eq!(AccountState::parse(state.as_str()), Some(state));
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!(state.as_str())
        );
    }
    for (class, state) in [
        ("ok", AccountState::Ok),
        ("verification_required", AccountState::VerificationRequired),
        ("location_unsupported", AccountState::LocationUnsupported),
        ("rate_limited", AccountState::RateLimited),
        ("suspended", AccountState::Suspended),
        ("unknown_refusal", AccountState::UnknownRefusal),
        ("a_future_class", AccountState::UnknownRefusal),
    ] {
        assert_eq!(AccountState::from_backend_class(class), state);
    }
    assert!(AccountState::Suspended.stops_units() && AccountState::Blocked.stops_units());
    assert!(!AccountState::RateLimited.stops_units());
    // Settlement never lowers an owner's block or a suspension.
    assert!(AccountState::Blocked.severity() > AccountState::Suspended.severity());
    assert!(AccountState::Suspended.severity() > AccountState::RateLimited.severity());
}

#[test]
fn every_non_ok_state_parks_dispatch_and_admission_comes_first() {
    assert_eq!(
        gate(&input(None, false, 0)),
        Gate::Proceed {
            clears_rate_limit: false
        }
    );
    let mut refused = input(None, true, 0);
    refused.admitted = false;
    assert!(matches!(gate(&refused), Gate::Park(reason) if reason.contains("not admitted")));
    for state in AccountState::ALL {
        if matches!(state, AccountState::Ok | AccountState::RateLimited) {
            continue;
        }
        let record = control(state, None);
        assert!(
            matches!(gate(&input(Some(&record), true, 0)), Gate::Park(_)),
            "{state:?}"
        );
        assert!(revival_allowed(true, Some(&record), 0).is_err());
    }
    let uncertain = GateInput {
        uncertain: Some(("fs/writeFile", 2)),
        ..input(None, true, 0)
    };
    assert!(matches!(gate(&uncertain), Gate::Park(reason) if reason.contains("fs/writeFile")));
}

#[test]
fn a_rate_limit_needs_a_new_owner_send_after_it_ends() {
    let limited = control(AccountState::RateLimited, Some(1_000));
    // Before the end, even an explicit send stays queued.
    assert!(
        matches!(gate(&input(Some(&limited), true, 999)), Gate::Park(r) if r.contains("until"))
    );
    // After it, an automatic drain still waits for the owner.
    assert!(
        matches!(gate(&input(Some(&limited), false, 1_000)), Gate::Park(r) if r.contains("send again"))
    );
    assert_eq!(
        gate(&input(Some(&limited), true, 1_000)),
        Gate::Proceed {
            clears_rate_limit: true
        }
    );
    assert!(revival_allowed(true, Some(&limited), 1_000).is_ok());
    assert!(revival_allowed(true, Some(&limited), 999).is_err());
}

/// The worker's stop reason of a proxy error: `error: <message>: <data>`.
fn proxy_error(message: &str, cowboy: &serde_json::Value) -> String {
    let data = serde_json::to_string_pretty(&serde_json::json!({ "cowboy": cowboy })).unwrap();
    format!("error: {message}: {data}")
}

#[test]
fn stop_reasons_are_classified_from_the_proxys_structured_result_only() {
    assert_eq!(classify_stop_reason("EndTurn"), TurnOutcome::Completed);
    assert_eq!(classify_stop_reason("Cancelled"), TurnOutcome::Completed);
    let backend = |class: &str| serde_json::json!({"antigravity_backend": {"class": class}});
    assert_eq!(
        classify_stop_reason(&proxy_error(
            "antigravity_backend_refusal:verification_required",
            &backend("verification_required"),
        )),
        TurnOutcome::Refused(AccountState::VerificationRequired)
    );
    assert_eq!(
        classify_stop_reason(&proxy_error(
            "antigravity_backend_refusal:suspended",
            &backend("suspended"),
        )),
        TurnOutcome::Refused(AccountState::Suspended)
    );
    assert_eq!(
        classify_stop_reason(&proxy_error(
            "antigravity_policy_violation:read_file",
            &serde_json::json!({"policy_violation": "read_file"}),
        )),
        TurnOutcome::PolicyViolation("read_file".into())
    );
    assert_eq!(
        classify_stop_reason("error: connection closed"),
        TurnOutcome::Unproven
    );
}

#[test]
fn vendor_text_never_selects_a_settlement() {
    let backend = |class: &str| serde_json::json!({"antigravity_backend": {"class": class}});
    for forged in [
        // A code without the proxy's structured data.
        "error: antigravity_backend_refusal:suspended".to_owned(),
        "error: Internal error: antigravity_backend_refusal:verification_required".to_owned(),
        "error: antigravity_policy_violation:read_file".to_owned(),
        // A code inside vendor text, even with data.
        proxy_error(
            "upstream said antigravity_backend_refusal:ok",
            &backend("ok"),
        ),
        proxy_error(
            "agy failed: antigravity_backend_refusal:suspended",
            &backend("suspended"),
        ),
        // Data that disagrees with the code, or no backend evidence.
        proxy_error(
            "antigravity_backend_refusal:suspended",
            &backend("rate_limited"),
        ),
        proxy_error(
            "antigravity_backend_refusal:suspended",
            &serde_json::json!({}),
        ),
        proxy_error(
            "antigravity_policy_violation:read_file",
            &serde_json::json!({"policy_violation": "write_file"}),
        ),
        // The proxy never reports `ok` as an error.
        proxy_error("antigravity_backend_refusal:ok", &backend("ok")),
        // Unknown stop reasons are not success.
        "something the vendor printed".to_owned(),
    ] {
        assert_eq!(
            classify_stop_reason(&forged),
            TurnOutcome::Unproven,
            "{forged}"
        );
    }
}

#[test]
fn only_proven_termination_settles_a_lost_unit() {
    assert_eq!(unit_loss_outcome(None), None);
    let evidence = |terminated, class: Option<&str>| TargetTermination {
        terminated,
        closed_log_class: class.map(str::to_owned),
    };
    assert_eq!(unit_loss_outcome(Some(&evidence(false, Some("ok")))), None);
    assert_eq!(
        unit_loss_outcome(Some(&evidence(true, None))),
        Some(TurnOutcome::UnitLost {
            evidence_complete: false
        })
    );
    assert_eq!(
        unit_loss_outcome(Some(&evidence(true, Some("ok")))),
        Some(TurnOutcome::UnitLost {
            evidence_complete: true
        })
    );
    // A refusal in the closed log always wins.
    assert_eq!(
        unit_loss_outcome(Some(&evidence(true, Some("suspended")))),
        Some(TurnOutcome::Refused(AccountState::Suspended))
    );
}

#[test]
fn only_a_reserved_turn_settles_not_dispatched() {
    let reserved = settle(ReservationState::Reserved, &TurnOutcome::NotDispatched);
    assert!(reserved.release && reserved.state.is_none());
    assert_eq!(reserved.settled, "not_dispatched");
    let dispatched = settle(
        ReservationState::MayHaveDispatched,
        &TurnOutcome::NotDispatched,
    );
    assert!(!dispatched.release);
    assert_eq!(dispatched.state.unwrap().0, AccountState::NeedsAttention);
}

#[test]
fn settlement_maps_evidence_to_the_account_state() {
    let state = ReservationState::MayHaveDispatched;
    let completed = settle(state, &TurnOutcome::Completed);
    assert!(completed.release && completed.state.is_none() && !completed.stop_units);
    let suspended = settle(state, &TurnOutcome::Refused(AccountState::Suspended));
    assert!(suspended.release && suspended.stop_units);
    assert_eq!(suspended.state.unwrap().0, AccountState::Suspended);
    let limited = settle(state, &TurnOutcome::Refused(AccountState::RateLimited));
    assert!(!limited.stop_units);
    let violation = settle(state, &TurnOutcome::PolicyViolation("run_command".into()));
    assert!(violation.stop_units);
    assert_eq!(
        violation.state.unwrap(),
        (
            AccountState::NeedsAttention,
            "policy_violation:run_command".to_owned()
        )
    );
    // Unit loss settles interrupted only with complete evidence (R3-10).
    let interrupted = settle(
        state,
        &TurnOutcome::UnitLost {
            evidence_complete: true,
        },
    );
    assert_eq!(
        (interrupted.settled, interrupted.state),
        ("interrupted", None)
    );
    let lost = settle(
        state,
        &TurnOutcome::UnitLost {
            evidence_complete: false,
        },
    );
    assert!(lost.release);
    assert_eq!(lost.state.unwrap().0, AccountState::UnknownRefusal);
    assert_eq!(
        settle(state, &TurnOutcome::Unproven).state.unwrap().0,
        AccountState::UnknownRefusal
    );
}

#[test]
fn owner_actions_follow_the_kill_switch_rules() {
    use AccountState::{Blocked, NeedsAttention, Ok as Ready};
    assert_eq!(
        owner_transition(OwnerAction::Block, Ready),
        Ok(Some(Blocked))
    );
    assert_eq!(owner_transition(OwnerAction::Block, Blocked), Ok(None));
    assert_eq!(
        owner_transition(OwnerAction::Unblock, Blocked),
        Ok(Some(Ready))
    );
    assert!(owner_transition(OwnerAction::Unblock, NeedsAttention).is_err());
    assert!(owner_transition(OwnerAction::Clear, Blocked).is_err());
    assert_eq!(
        owner_transition(OwnerAction::Clear, NeedsAttention),
        Ok(Some(Ready))
    );
}

#[test]
fn fence_operations_are_stable_per_request_revision() {
    use crate::provider_account::AccountFenceAction::{Admit, Stop};
    let id = fence_operation_id("a", "ovh", Stop, 3);
    assert_eq!(id, fence_operation_id("a", "ovh", Stop, 3));
    assert_ne!(id, fence_operation_id("a", "ovh", Admit, 3));
    assert_ne!(id, fence_operation_id("a", "ovh", Stop, 4));
    assert_ne!(id, fence_operation_id("a", "hawk", Stop, 3));
    assert!(id.starts_with("account-fence-") && id.len() == 46);
}

#[test]
fn logout_needs_the_kill_switch_and_is_idempotent_per_revision() {
    use AccountState::{Blocked, Ok as Ready, Suspended};
    assert_eq!(owner_transition(OwnerAction::Logout, Blocked), Ok(None));
    assert_eq!(
        owner_transition(OwnerAction::Logout, Ready),
        Err("account_not_blocked")
    );
    assert!(owner_transition(OwnerAction::Logout, Suspended).is_err());
    let id = logout_operation_id("a", 3);
    assert_eq!(id, logout_operation_id("a", 3));
    assert_ne!(id, logout_operation_id("a", 4));
    assert_ne!(id, logout_operation_id("b", 3));
    assert!(id.starts_with("account-logout-") && id.len() <= 160);
    assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
}

#[test]
fn logout_state_summarizes_its_machine_targets() {
    use LogoutTargetState::{Cancelled, Failed, Pending, SignedOut, Unconfirmed};
    assert_eq!(logout_state(&[]), "not_signed_in");
    assert_eq!(logout_state(&[SignedOut, Pending]), "pending");
    assert_eq!(logout_state(&[SignedOut, Failed]), "failed");
    assert_eq!(logout_state(&[Unconfirmed, SignedOut]), "unconfirmed");
    assert_eq!(logout_state(&[SignedOut, SignedOut]), "signed_out");
    assert_eq!(
        logout_state(&[SignedOut, Cancelled]),
        "partially_signed_out"
    );
    assert_eq!(logout_state(&[Cancelled]), "cancelled");
}
