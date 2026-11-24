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

fn input(control: Option<&Control>, now_ms: i64) -> GateInput<'_> {
    GateInput {
        admitted: true,
        control,
        uncertain: None,
        quota_group: None,
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
    assert!(AccountState::NeedsAttention.stops_units());
    assert!(!AccountState::RateLimited.stops_units());
    // Settlement never lowers an owner's block or a suspension.
    assert!(AccountState::Blocked.severity() > AccountState::Suspended.severity());
    assert!(AccountState::Suspended.severity() > AccountState::RateLimited.severity());
}

#[test]
fn every_non_ok_state_parks_dispatch_and_admission_comes_first() {
    assert_eq!(
        gate(&input(None, 0)),
        Gate::Proceed {
            clears_rate_limit: false
        }
    );
    let mut refused = input(None, 0);
    refused.admitted = false;
    assert!(matches!(gate(&refused), Gate::Park(reason) if reason.contains("not admitted")));
    for state in AccountState::ALL {
        if matches!(state, AccountState::Ok | AccountState::RateLimited) {
            continue;
        }
        let record = control(state, None);
        assert!(
            matches!(gate(&input(Some(&record), 0)), Gate::Park(_)),
            "{state:?}"
        );
        assert!(revival_allowed(true, Some(&record), None, 0).is_err());
    }
    let uncertain = GateInput {
        uncertain: Some(("fs/writeFile", 2)),
        ..input(None, 0)
    };
    assert!(matches!(gate(&uncertain), Gate::Park(reason) if reason.contains("fs/writeFile")));
}

#[test]
fn a_rate_limit_parks_until_it_ends_then_the_next_dispatch_resumes() {
    let limited = control(AccountState::RateLimited, Some(1_000));
    // Before the end, every dispatch stays queued.
    assert!(matches!(gate(&input(Some(&limited), 999)), Gate::Park(r) if r.contains("until")));
    // After it, any dispatch (owner, notice or agent) resumes the account.
    assert_eq!(
        gate(&input(Some(&limited), 1_000)),
        Gate::Proceed {
            clears_rate_limit: true
        }
    );
    assert!(revival_allowed(true, Some(&limited), None, 1_000).is_ok());
    assert!(revival_allowed(true, Some(&limited), None, 999).is_err());
}

#[test]
fn models_map_to_their_quota_groups() {
    assert_eq!(quota_group("gemini-4-neon"), Some(GEMINI_GROUP));
    assert_eq!(quota_group("Gemini-4-Hydrogen"), Some(GEMINI_GROUP));
    assert_eq!(quota_group("claude-opus-5-5"), Some(CLAUDE_GPT_GROUP));
    assert_eq!(quota_group("gpt-oss-120b"), Some(CLAUDE_GPT_GROUP));
    assert_eq!(quota_group("default"), None);
    // Argon has a quota of its own that no group tracks.
    assert_eq!(quota_group("gemini-4-argon"), None);
    assert_eq!(rate_limit_reason(Some(GEMINI_GROUP)), "rate_limited:gemini");
    assert_eq!(rate_limit_reason(None), "rate_limited");
    assert_eq!(
        rate_limit_scope("rate_limited:claude_gpt"),
        Some(CLAUDE_GPT_GROUP)
    );
    assert_eq!(rate_limit_scope("rate_limited"), None);
    assert_eq!(rate_limit_scope("rate_limited:"), None);
}

#[test]
fn a_scoped_rate_limit_parks_only_its_own_and_unknown_groups() {
    let mut limited = control(AccountState::RateLimited, Some(1_000));
    limited.reason = "rate_limited:claude_gpt".into();
    let group = |quota_group| GateInput {
        quota_group,
        ..input(Some(&limited), 999)
    };
    assert!(
        matches!(gate(&group(Some(CLAUDE_GPT_GROUP))), Gate::Park(r) if r.contains("Claude and GPT"))
    );
    assert!(matches!(gate(&group(None)), Gate::Park(_)));
    assert_eq!(
        gate(&group(Some(GEMINI_GROUP))),
        Gate::Proceed {
            clears_rate_limit: false
        }
    );
    assert!(revival_allowed(true, Some(&limited), Some(GEMINI_GROUP), 999).is_ok());
    assert!(revival_allowed(true, Some(&limited), Some(CLAUDE_GPT_GROUP), 999).is_err());
    // An account-wide limit parks every group.
    let wide = control(AccountState::RateLimited, Some(1_000));
    let gemini = GateInput {
        quota_group: Some(GEMINI_GROUP),
        ..input(Some(&wide), 999)
    };
    assert!(matches!(gate(&gemini), Gate::Park(_)));
}

#[test]
fn a_groups_reset_is_its_latest_exhausted_window() {
    let limits = serde_json::json!({"rateLimitsByLimitId": {
        "0000-gemini-models": {"limitName": "Gemini Models", "primary": {"usedPercent": 3.5, "resetsAt": 9_000}},
        "0001-gemini-models": {"limitName": "Gemini Models", "primary": {"usedPercent": 0, "resetsAt": 8_000}},
        "0100-claude-and-gpt-models": {"limitName": "Claude and GPT models", "primary": {"usedPercent": 100, "resetsAt": 7_000}},
        "0101-claude-and-gpt-models": {"limitName": "Claude and GPT models", "primary": {"usedPercent": 100.0, "resetsAt": 5_000.5}},
        "0200-argon": {"limitName": "Gemini 4 Argon", "primary": {"usedPercent": 100, "resetsAt": 9_500}},
    }});
    assert_eq!(
        group_reset_ms(&limits, CLAUDE_GPT_GROUP, 0),
        Some(7_000_000)
    );
    // A window that already reset no longer holds.
    assert_eq!(group_reset_ms(&limits, CLAUDE_GPT_GROUP, 7_000_000), None);
    assert_eq!(
        group_reset_ms(&limits, CLAUDE_GPT_GROUP, 6_000_000),
        Some(7_000_000)
    );
    assert_eq!(group_reset_ms(&limits, GEMINI_GROUP, 0), None);
    assert_eq!(
        group_reset_ms(&serde_json::json!({}), GEMINI_GROUP, 0),
        None
    );
}

#[test]
fn an_implausible_reset_is_not_believed() {
    let far = |resets: serde_json::Value| {
        serde_json::json!({"rateLimitsByLimitId": {"0101-claude-and-gpt-models": {
            "limitName": "Claude and GPT models", "primary": {"usedPercent": 100, "resetsAt": resets}}}})
    };
    let now = 1_795_486_000_000;
    let within = (now + MAX_QUOTA_RESET_MS) / 1000;
    assert_eq!(
        group_reset_ms(&far(serde_json::json!(within)), CLAUDE_GPT_GROUP, now),
        Some(within * 1000)
    );
    assert_eq!(
        group_reset_ms(&far(serde_json::json!(within + 1)), CLAUDE_GPT_GROUP, now),
        None
    );
    assert_eq!(
        group_reset_ms(
            &far(serde_json::json!(253_402_300_799_i64)),
            CLAUDE_GPT_GROUP,
            now
        ),
        None
    );
    assert_eq!(
        group_reset_ms(&far(serde_json::json!(1e300)), CLAUDE_GPT_GROUP, now),
        None
    );
}

#[test]
fn an_ended_limit_holds_on_for_each_group_still_used_up() {
    let bucket = |name: &str, used: f64, resets: i64| serde_json::json!({"limitName": name, "primary": {"usedPercent": used, "resetsAt": resets}});
    let both = serde_json::json!({"rateLimitsByLimitId": {
        "g": bucket("Gemini Models", 100.0, 50), "c": bucket("Claude and GPT models", 100.0, 70)}});
    let claude = serde_json::json!({"rateLimitsByLimitId": {
        "g": bucket("Gemini Models", 10.0, 50), "c": bucket("Claude and GPT models", 100.0, 70)}});
    assert_eq!(
        hold_after_expiry(None, Some(&both), 0),
        Some(("rate_limited".into(), 50_000))
    );
    assert_eq!(
        hold_after_expiry(None, Some(&claude), 0),
        Some(("rate_limited:claude_gpt".into(), 70_000))
    );
    assert_eq!(
        hold_after_expiry(Some(GEMINI_GROUP), Some(&claude), 0),
        None
    );
    assert_eq!(
        hold_after_expiry(Some(CLAUDE_GPT_GROUP), Some(&claude), 0),
        Some(("rate_limited:claude_gpt".into(), 70_000))
    );
    assert_eq!(hold_after_expiry(None, None, 0), None);
}

#[test]
fn a_second_groups_rate_limit_widens_a_running_one_to_the_account() {
    let claude = "rate_limited:claude_gpt";
    let gemini = "rate_limited:gemini";
    assert_eq!(
        merge_rate_limit((claude, Some(5)), (claude, Some(9)), 0),
        (claude.to_owned(), Some(9))
    );
    assert_eq!(
        merge_rate_limit((claude, Some(9)), (gemini, Some(5)), 0),
        ("rate_limited".to_owned(), Some(9))
    );
    assert_eq!(
        merge_rate_limit(("rate_limited", Some(9)), (gemini, Some(5)), 0),
        ("rate_limited".to_owned(), Some(9))
    );
    // An ended limit is simply replaced.
    assert_eq!(
        merge_rate_limit((claude, Some(5)), (gemini, Some(9)), 5),
        (gemini.to_owned(), Some(9))
    );
}

/// The worker's stop reason of a proxy error: `error: <message>: <data>`.
fn proxy_error(message: &str, cowboy: &serde_json::Value) -> String {
    let data = serde_json::to_string_pretty(&serde_json::json!({ "cowboy": cowboy })).unwrap();
    format!("error: {message}: {data}")
}

#[test]
fn stop_reasons_are_classified_from_the_proxys_structured_result_only() {
    assert_eq!(classify_stop_reason("EndTurn"), Signal::None);
    assert_eq!(classify_stop_reason("Cancelled"), Signal::None);
    let backend = |class: &str| serde_json::json!({"antigravity_backend": {"class": class}});
    assert_eq!(
        classify_stop_reason(&proxy_error(
            "antigravity_backend_refusal:verification_required",
            &backend("verification_required"),
        )),
        Signal::Refused(AccountState::VerificationRequired)
    );
    assert_eq!(
        classify_stop_reason(&proxy_error(
            "antigravity_backend_refusal:suspended",
            &backend("suspended"),
        )),
        Signal::Refused(AccountState::Suspended)
    );
    assert_eq!(
        classify_stop_reason(&proxy_error(
            "antigravity_policy_violation:read_file",
            &serde_json::json!({"policy_violation": "read_file"}),
        )),
        Signal::PolicyViolation("read_file".into())
    );
    assert_eq!(
        classify_stop_reason("error: connection closed"),
        Signal::Unclassified
    );
}

/// The proxy's turn error as the worker renders it (`agy-proxy` code
/// `-32603`, `data.message` for display beside `data.cowboy`).
fn displayed_proxy_error(message: &str, text: &str, cowboy: &serde_json::Value) -> String {
    let data = serde_json::json!({ "message": text, "cowboy": cowboy });
    let data = serde_json::to_string_pretty(&data).unwrap();
    format!("error: {message}: {data}")
}

#[test]
fn the_proxys_displayed_turn_errors_keep_their_classification() {
    // sess-1795107755038: a policy violation maps to needs_attention, never
    // unknown_refusal, when the error also carries its display text.
    let violation = classify_stop_reason(&displayed_proxy_error(
        "antigravity_policy_violation:search_web",
        "A built-in Antigravity tool ran despite Cowboy's gate.",
        &serde_json::json!({
            "antigravity_backend": {"class": "ok", "successes": 1, "barrier": true},
            "policy_violation": "search_web",
        }),
    ));
    assert_eq!(violation, Signal::PolicyViolation("search_web".into()));
    assert_eq!(
        signal_transition(&violation).map(|t| (t.state, t.reason)),
        Some((
            AccountState::NeedsAttention,
            "policy_violation:search_web".to_owned()
        ))
    );
    assert_eq!(
        classify_stop_reason(&displayed_proxy_error(
            "antigravity_backend_refusal:rate_limited",
            "Google rate-limited this Antigravity account.",
            &serde_json::json!({"antigravity_backend": {"class": "rate_limited"}}),
        )),
        Signal::Refused(AccountState::RateLimited)
    );
    // A built-in the hook denied never ran: the proxy settles the turn as
    // an ordinary ending, which raises nothing.
    assert_eq!(signal_transition(&classify_stop_reason("EndTurn")), None);
    // The worker presents ACP's `-32000` (auth_required) as rejected
    // credentials and keeps only the data; that rewrite is not the proxy's
    // closed code, which is why the proxy no longer uses `-32000`.
    assert_eq!(
        classify_stop_reason(&format!(
            "error: Provider authentication required - Antigravity credentials expired or \
             were rejected.: {}",
            serde_json::to_string_pretty(&serde_json::json!({"cowboy": {
                "antigravity_backend": {"class": "ok"},
                "policy_violation": "search_web",
            }}))
            .unwrap()
        )),
        Signal::Unclassified
    );
}

#[test]
fn vendor_text_never_selects_a_transition() {
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
            Signal::Unclassified,
            "{forged}"
        );
    }
}

#[test]
fn signals_map_to_account_transitions() {
    assert_eq!(signal_transition(&Signal::None), None);
    let suspended = signal_transition(&Signal::Refused(AccountState::Suspended)).unwrap();
    assert!(suspended.stop_units);
    assert_eq!(suspended.state, AccountState::Suspended);
    let limited = signal_transition(&Signal::Refused(AccountState::RateLimited)).unwrap();
    assert!(!limited.stop_units);
    let violation = signal_transition(&Signal::PolicyViolation("run_command".into())).unwrap();
    assert!(violation.stop_units);
    assert_eq!(
        (violation.state, violation.reason.as_str()),
        (AccountState::NeedsAttention, "policy_violation:run_command")
    );
    let unknown = signal_transition(&Signal::Unclassified).unwrap();
    assert_eq!(unknown.state, AccountState::UnknownRefusal);
    assert!(!unknown.stop_units);
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
fn a_fence_carries_exactly_the_revision_that_wrote_it() {
    use crate::provider_account::AccountFenceAction::Stop;
    let fence = Fence {
        account_id: "a".into(),
        machine_id: "ovh".into(),
        action: Stop,
        operation_id: fence_operation_id("a", "ovh", Stop, 3),
        state: FenceState::Pending,
        detail: String::new(),
        updated_at_ms: 0,
    };
    // Later revisions without a new fence never relabel it.
    assert_eq!(fence_revision(&fence, 3), Some(3));
    assert_eq!(fence_revision(&fence, 9), Some(3));
    assert_eq!(fence_revision(&fence, 2), None);
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
