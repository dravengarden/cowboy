use std::collections::HashMap;

use super::{
    Attempt, ConvergencePolicy, ConvergenceState, DesiredComponentSource, DesiredComponents,
    parse_manifest, plan_machine,
};
use crate::machine_protocol::{
    ComponentConvergenceState, ComponentId, ComponentInventory, ComponentKind, ComponentProbe,
    ComponentState, DesiredComponent, MachineCapacity, MachineHealth, MachineHealthReason,
    MachineHealthState, MachineSummary,
};

const DIGEST: &str = "aa11bb22cc33dd44ee55ff6677889900aa11bb22cc33dd44ee55ff6677889900";
const OLD_DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn component_id() -> ComponentId {
    ComponentId {
        kind: ComponentKind::ZedServer,
        slot: "zed".to_owned(),
    }
}

fn desired(automatic: bool, digest: &str) -> DesiredComponent {
    DesiredComponent {
        id: component_id(),
        version: "1.2.3".to_owned(),
        generation: "gen-1".to_owned(),
        artifact_url: "https://releases.example/zed".to_owned(),
        digest: digest.to_owned(),
        artifact_format: crate::machine_protocol::ArtifactFormat::Raw,
        entrypoint: None,
        signature: Some("signature".to_owned()),
        probe: Some(ComponentProbe {
            args: vec!["--version".to_owned()],
            timeout_ms: 5_000,
        }),
        automatic,
    }
}

fn desired_set(components: Vec<DesiredComponent>) -> DesiredComponents {
    DesiredComponents {
        generation: 1,
        components,
        loaded_at_ms: 0,
        error: None,
    }
}

fn installed(digest: &str, state: ComponentState, leases: u64) -> ComponentInventory {
    ComponentInventory {
        id: component_id(),
        state,
        version: "1.2.2".to_owned(),
        generation: "gen-0".to_owned(),
        digest: digest.to_owned(),
        rollback_generation: None,
        active_leases: leases,
        auth: None,
        detail: None,
        update: None,
        superseded_by: None,
    }
}

fn summary(connected: bool, components: Vec<ComponentInventory>) -> MachineSummary {
    MachineSummary {
        id: "hawk".to_owned(),
        display_name: "hawk".to_owned(),
        platform: "linux".to_owned(),
        architecture: "x86_64".to_owned(),
        status: if connected { "online" } else { "offline" }.to_owned(),
        local: true,
        connected,
        schedulable: connected,
        health: MachineHealth {
            state: MachineHealthState::Ready,
            reason: MachineHealthReason::RuntimeConnected,
            observed_at_ms: 0,
            last_seen_at_ms: None,
        },
        fingerprint: None,
        workspaces: Vec::new(),
        workspace_revision: None,
        components,
        plugins: Vec::new(),
        provider_contracts: None,
        plugin_contracts: None,
        capacity: MachineCapacity::default(),
        active_sessions: 0,
        pending_updates: Vec::new(),
        convergence: Vec::new(),
    }
}

#[test]
fn an_outdated_automatic_component_is_dispatched() {
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert_eq!(plan.dispatch.len(), 1);
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Pending);
}

#[test]
fn a_converged_component_produces_no_work_at_all() {
    let plan = plan_machine(
        &summary(true, vec![installed(DIGEST, ComponentState::Active, 0)]),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert_eq!(plan, super::Plan::default());
}

#[test]
fn a_staged_but_inactive_generation_still_converges() {
    // Matching bytes are not an activation: a failed or staged component must
    // be reconciled again rather than counted as converged.
    let plan = plan_machine(
        &summary(true, vec![installed(DIGEST, ComponentState::Failed, 0)]),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert_eq!(plan.dispatch.len(), 1);
}

#[test]
fn convergence_never_replaces_a_leased_generation() {
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 2)]),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert!(plan.dispatch.is_empty());
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Draining);
}

#[test]
fn a_component_the_manifest_does_not_automate_is_left_to_its_owner() {
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]),
        &desired_set(vec![desired(false, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert_eq!(plan, super::Plan::default());
}

#[test]
fn a_disconnected_machine_is_never_dispatched_to() {
    let plan = plan_machine(
        &summary(false, Vec::new()),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert_eq!(plan, super::Plan::default());
}

#[test]
fn a_failing_component_backs_off_and_then_stops() {
    let policy = ConvergencePolicy::default();
    let state = ConvergenceState::default();
    let dispatched = vec![desired(true, DIGEST)];
    let set = desired_set(dispatched.clone());
    let machine = summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]);

    state.record_dispatch("hawk", &dispatched, policy, 1_000);
    state.record_outcome("hawk", &dispatched, Some("probe failed"));
    let attempts = state.attempts("hawk");
    let plan = plan_machine(&machine, &set, &attempts, false, 1_000);
    assert!(plan.dispatch.is_empty(), "the backoff must be respected");
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Retrying);
    assert_eq!(plan.reported[0].detail.as_deref(), Some("probe failed"));

    // Once the backoff elapses the same digest is retried.
    let elapsed = attempts[&component_id()].next_attempt_at_ms;
    assert_eq!(
        plan_machine(&machine, &set, &attempts, false, elapsed)
            .dispatch
            .len(),
        1
    );

    for attempt in 1..policy.max_attempts {
        let now = 1_000 * i64::from(attempt);
        state.record_dispatch("hawk", &dispatched, policy, now);
        state.record_outcome("hawk", &dispatched, Some("probe failed"));
    }
    let plan = plan_machine(&machine, &set, &state.attempts("hawk"), false, i64::MAX);
    assert!(
        plan.dispatch.is_empty(),
        "a blocked component must not be retried"
    );
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Blocked);
}

#[test]
fn an_acknowledged_reconcile_that_never_converges_still_stops() {
    // The acknowledgement says the Machine handled the command, not that the
    // component became active. Without counting it, a Machine that answers
    // every Reconcile but never activates would be retried forever.
    let policy = ConvergencePolicy::default();
    let state = ConvergenceState::default();
    let dispatched = vec![desired(true, DIGEST)];
    let set = desired_set(dispatched.clone());
    let machine = summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]);

    state.record_dispatch("hawk", &dispatched, policy, 1_000);
    state.record_outcome("hawk", &dispatched, None);
    let plan = plan_machine(&machine, &set, &state.attempts("hawk"), false, 1_000);
    assert!(plan.dispatch.is_empty());
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Verifying);
    assert_eq!(plan.reported[0].detail, None);

    for attempt in 1..policy.max_attempts {
        state.record_dispatch("hawk", &dispatched, policy, 1_000 * i64::from(attempt));
        state.record_outcome("hawk", &dispatched, None);
    }
    let plan = plan_machine(&machine, &set, &state.attempts("hawk"), false, i64::MAX);
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Blocked);
}

#[test]
fn a_new_digest_starts_its_own_history() {
    let policy = ConvergencePolicy::default();
    let state = ConvergenceState::default();
    let broken = vec![desired(true, OLD_DIGEST)];
    for attempt in 0..policy.max_attempts {
        state.record_dispatch("hawk", &broken, policy, i64::from(attempt));
        state.record_outcome("hawk", &broken, Some("probe failed"));
    }
    assert!(state.attempts("hawk")[&component_id()].blocked);

    let fixed = desired_set(vec![desired(true, DIGEST)]);
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]),
        &fixed,
        &state.attempts("hawk"),
        false,
        1_000,
    );
    assert_eq!(
        plan.dispatch.len(),
        1,
        "a published fix must not inherit the block"
    );
}

#[test]
fn a_converged_component_forgets_its_history() {
    let policy = ConvergencePolicy::default();
    let state = ConvergenceState::default();
    let dispatched = vec![desired(true, DIGEST)];
    state.record_dispatch("hawk", &dispatched, policy, 0);
    state.record_outcome("hawk", &dispatched, Some("probe failed"));

    // The inventory now proves the desired digest is active, so nothing is
    // reported and the history is pruned with it.
    let plan = plan_machine(
        &summary(true, vec![installed(DIGEST, ComponentState::Active, 0)]),
        &desired_set(dispatched.clone()),
        &state.attempts("hawk"),
        false,
        1_000,
    );
    assert!(plan.reported.is_empty());
    state.retain(
        "hawk",
        &plan.reported.iter().map(|entry| entry.id.clone()).collect(),
    );
    assert!(state.attempts("hawk").is_empty());
}

#[test]
fn a_still_pending_component_keeps_its_history_through_a_prune() {
    let policy = ConvergencePolicy::default();
    let state = ConvergenceState::default();
    let dispatched = vec![desired(true, DIGEST)];
    state.record_dispatch("hawk", &dispatched, policy, 0);
    state.record_outcome("hawk", &dispatched, Some("probe failed"));
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]),
        &desired_set(dispatched.clone()),
        &state.attempts("hawk"),
        false,
        1_000,
    );
    state.retain(
        "hawk",
        &plan.reported.iter().map(|entry| entry.id.clone()).collect(),
    );
    assert_eq!(state.attempts("hawk")[&component_id()].attempts, 1);
}

#[test]
fn one_machine_converges_at_a_time() {
    let state = ConvergenceState::default();
    assert!(state.begin("hawk"));
    assert!(
        !state.begin("hawk"),
        "a second pass must not race the first"
    );
    assert!(state.begin("falcon"));
    state.finish("hawk");
    assert!(state.begin("hawk"));
}

#[test]
fn an_unusable_record_is_refused_before_it_reaches_a_machine() {
    let signed = serde_json::to_vec(&vec![desired(true, DIGEST)]).expect("serialize");
    assert_eq!(parse_manifest(&signed).expect("accepted").len(), 1);

    let mut unsigned = desired(true, DIGEST);
    unsigned.signature = None;
    let error = parse_manifest(&serde_json::to_vec(&vec![unsigned]).expect("serialize"))
        .expect_err("unsigned");
    assert!(error.contains("unsigned"), "{error}");

    let mut unprobed = desired(true, DIGEST);
    unprobed.probe = None;
    let error = parse_manifest(&serde_json::to_vec(&vec![unprobed]).expect("serialize"))
        .expect_err("probeless");
    assert!(error.contains("health probe"), "{error}");

    let mut plaintext = desired(true, DIGEST);
    plaintext.artifact_url = "http://releases.example/zed".to_owned();
    let error = parse_manifest(&serde_json::to_vec(&vec![plaintext]).expect("serialize"))
        .expect_err("plaintext");
    assert!(error.contains("HTTPS"), "{error}");

    let mut short = desired(true, DIGEST);
    short.digest = "abc".to_owned();
    let error =
        parse_manifest(&serde_json::to_vec(&vec![short]).expect("serialize")).expect_err("digest");
    assert!(error.contains("digest"), "{error}");

    let duplicated = vec![desired(true, DIGEST), desired(true, OLD_DIGEST)];
    let error = parse_manifest(&serde_json::to_vec(&duplicated).expect("serialize"))
        .expect_err("duplicate");
    assert!(error.contains("duplicate"), "{error}");

    // A manifest that is not automatic at all may still omit its probe.
    let mut manual = desired(false, DIGEST);
    manual.probe = None;
    assert_eq!(
        parse_manifest(&serde_json::to_vec(&vec![manual]).expect("serialize"))
            .expect("accepted")
            .len(),
        1
    );
}

#[test]
fn a_broken_reload_keeps_the_last_accepted_desired_state() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("components.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&vec![desired(true, OLD_DIGEST)]).expect("serialize"),
    )
    .expect("write manifest");
    let source = DesiredComponentSource::load(Some(&path), 0).expect("load");
    assert_eq!(source.current().generation, 1);
    assert_eq!(source.current().automatic().len(), 1);

    // An unchanged manifest is not a new generation.
    assert_eq!(source.reload(1).expect("reload"), None);

    std::fs::write(&path, b"{ not json").expect("write manifest");
    let error = source.reload(2).expect_err("invalid manifest");
    assert!(error.contains("parsing manifest"), "{error}");
    let current = source.current();
    assert_eq!(
        current.generation, 1,
        "a bad manifest must not retire the good one"
    );
    assert_eq!(current.components[0].digest, OLD_DIGEST);
    assert!(current.error.is_some());

    std::fs::write(
        &path,
        serde_json::to_vec(&vec![desired(true, DIGEST)]).expect("serialize"),
    )
    .expect("write manifest");
    assert_eq!(source.reload(3).expect("reload"), Some(2));
    let current = source.current();
    assert_eq!(current.components[0].digest, DIGEST);
    assert!(current.error.is_none());
}

#[test]
fn an_unconfigured_controller_converges_nothing() {
    let source = DesiredComponentSource::load(None, 0).expect("load");
    assert!(!source.is_configured());
    assert!(source.current().is_empty());
    assert_eq!(source.reload(0).expect("reload"), None);
}

#[test]
fn the_reported_plan_survives_a_machine_with_no_inventory_yet() {
    let plan = plan_machine(
        &summary(true, Vec::new()),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        false,
        1_000,
    );
    assert_eq!(
        plan.dispatch.len(),
        1,
        "a first install has nothing to drain"
    );
}

#[test]
fn a_blocked_attempt_reports_without_a_retry_time() {
    let attempts = HashMap::from([(
        component_id(),
        Attempt {
            digest: DIGEST.to_owned(),
            attempts: 4,
            next_attempt_at_ms: 10,
            blocked: true,
            acknowledged: false,
            detail: Some("probe failed".to_owned()),
        },
    )]);
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]),
        &desired_set(vec![desired(true, DIGEST)]),
        &attempts,
        false,
        1_000,
    );
    assert!(plan.dispatch.is_empty());
    assert_eq!(plan.reported[0].next_attempt_at_ms, None);
}

#[test]
fn a_freeze_stops_every_dispatch_and_says_so() {
    let plan = plan_machine(
        &summary(true, vec![installed(OLD_DIGEST, ComponentState::Active, 0)]),
        &desired_set(vec![desired(true, DIGEST)]),
        &HashMap::new(),
        true,
        1_000,
    );
    assert!(plan.dispatch.is_empty());
    assert_eq!(plan.reported[0].state, ComponentConvergenceState::Frozen);
}

#[test]
fn a_freeze_is_recorded_reversible_and_fails_safe() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let freeze = super::ConvergenceFreeze::new(directory.path());
    assert!(!freeze.is_frozen());
    // Resuming something that is already running is not an error.
    freeze.thaw().expect("thaw");

    let record = freeze
        .freeze("unix-uid:1000", Some("investigating a bad release"), 42)
        .expect("freeze");
    assert_eq!(record.actor, "unix-uid:1000");
    assert_eq!(record.frozen_at_ms, 42);
    let current = freeze.current().expect("frozen");
    assert_eq!(
        current.reason.as_deref(),
        Some("investigating a bad release")
    );

    // A damaged stop is still a stop: the only safe reading of "somebody left
    // a stop here and it is unreadable" is to stay stopped.
    std::fs::write(
        directory.path().join(super::ConvergenceFreeze::FILE_NAME),
        b"{ not json",
    )
    .expect("damage the record");
    assert!(freeze.is_frozen());
    assert_eq!(freeze.current().expect("frozen").schema, 0);

    freeze.thaw().expect("thaw");
    assert!(!freeze.is_frozen());
}
