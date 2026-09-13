use super::*;
use crate::machine_protocol::telemetry_binding::{BindingObservationSnapshot, BindingRejection};
use crate::telemetry_binding::tests::{applied, observed};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) struct FixtureEffects {
    intent: Intent,
    result: BindingObservation,
    lose_ack: bool,
    lose_query: bool,
    revoke_on_dispatch: bool,
    alive: AtomicBool,
    sends: AtomicUsize,
    queries: AtomicUsize,
    checks: AtomicUsize,
    revoke_on_check: Option<usize>,
    authority: crate::server::operator_approval::TelemetryBindingAuthority,
    hub: crate::core::Hub,
    devices: crate::client_auth::DeviceAccessSessions,
    authentication: crate::auth_plugins::ProductAuthentication,
}

fn fixture(suffix: &str) -> Intent {
    let mut intent = crate::telemetry_binding::fixture(suffix);
    intent.actor = crate::plugin_operation::Actor::Product {
        user_id: crate::product_auth::local_product_principal().user_id,
    };
    intent
}

impl FixtureEffects {
    pub(super) fn new(intent: &Intent) -> Self {
        let hub = crate::core::Hub::new();
        let devices = crate::client_auth::DeviceAccessSessions::default();
        let authentication = crate::auth_plugins::ProductAuthentication::test_default(None);
        let auth = crate::server::ProductRequestAuth {
            product_auth_enabled: false,
            store: None,
            hub: &hub,
            device_access: &devices,
            product_authentication: &authentication,
        };
        let verified = crate::server::AuthenticatedProductRequest {
            principal: crate::product_auth::local_product_principal(),
            cookie_session: None,
            device_identity: None,
        };
        let approval = crate::server::operator_approval::OperatorApproval::capture(
            auth,
            &intent.service_id,
            Some(&verified),
            &axum::http::HeaderMap::new(),
        )
        .unwrap();
        let authority = approval.bind_telemetry(intent).unwrap();
        Self {
            intent: intent.clone(),
            result: applied(intent),
            lose_ack: false,
            lose_query: false,
            revoke_on_dispatch: false,
            alive: AtomicBool::new(true),
            sends: AtomicUsize::new(0),
            queries: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
            revoke_on_check: None,
            authority,
            hub,
            devices,
            authentication,
        }
    }

    pub(super) fn confirmation(&self) -> Confirmation<'_> {
        Confirmation {
            authority: &self.authority,
            auth: crate::server::ProductRequestAuth {
                product_auth_enabled: false,
                store: None,
                hub: &self.hub,
                device_access: &self.devices,
                product_authentication: &self.authentication,
            },
        }
    }
}

async fn coordinate(
    store: &Store,
    fence: &LegacyFence,
    intent: &Intent,
    effects: &FixtureEffects,
) -> Result<Operation> {
    // Exercise the production confirmation gate using actual explicit local
    // Operator capture. Cookie/device/token revocation is covered alongside it
    // in operator_approval's real credential-backed fixtures.
    super::coordinate(store, fence, intent, effects.confirmation(), effects).await
}

impl Effects for FixtureEffects {
    async fn authorized(&self, intent: &Intent) -> bool {
        let check = self.checks.fetch_add(1, Ordering::Relaxed) + 1;
        if self.revoke_on_check == Some(check) {
            self.alive.store(false, Ordering::Release);
        }
        intent == &self.intent && self.within_budget()
    }
    fn within_budget(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
    async fn dispatch(&self, step: &BindingStep) -> Result<BindingObservation> {
        assert_eq!(step, &self.intent.machine_step().unwrap());
        assert_eq!(
            self.sends.fetch_add(1, Ordering::Relaxed),
            0,
            "never retry a finite mutation"
        );
        if self.revoke_on_dispatch {
            self.alive.store(false, Ordering::Release);
        }
        if self.lose_ack {
            anyhow::bail!("hermetic lost ACK");
        }
        Ok(self.result.clone())
    }
    async fn observe(&self, step: &BindingStep) -> Result<BindingObservation> {
        assert_eq!(step, &self.intent.machine_step().unwrap());
        let count = self.queries.fetch_add(1, Ordering::Relaxed);
        if count == 0 {
            return Ok(BindingObservation::Observed {
                snapshot: Box::new(BindingObservationSnapshot {
                    request_digest: step.request_digest()?,
                    receipt: None,
                    current: self.intent.expected.clone(),
                    unresolved: false,
                }),
            });
        }
        if self.lose_query {
            anyhow::bail!("hermetic disconnected observation");
        }
        Ok(self.result.clone())
    }
}

async fn store(root: &tempfile::TempDir) -> Store {
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
}

#[tokio::test]
async fn lost_ack_queries_original_step_once_and_duplicates_never_dispatch() {
    for lose_ack in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root).await;
        let intent = fixture("lost-ack");
        let fence = LegacyFence::recover(Some(&store), &intent.service_id)
            .await
            .unwrap();
        let mut effects = FixtureEffects::new(&intent);
        effects.lose_ack = lose_ack;
        let op = coordinate(&store, &fence, &intent, &effects).await.unwrap();
        assert!(matches!(op.progress, Progress::Completed { .. }));
        assert!(!fence.allows_legacy());
        effects.alive.store(false, Ordering::Release);
        assert_eq!(
            coordinate(&store, &fence, &intent, &effects).await.unwrap(),
            op,
            "expired duplicate is history, not renewed authority"
        );
        assert_eq!(effects.sends.load(Ordering::Relaxed), 1);
        assert_eq!(
            effects.queries.load(Ordering::Relaxed),
            if lose_ack { 2 } else { 1 }
        );
        let recovered = LegacyFence::recover(Some(&store), &intent.service_id)
            .await
            .unwrap();
        assert!(!recovered.allows_legacy());
    }
}

#[tokio::test]
async fn ended_authority_retains_applied_evidence_without_adopting_service_head() {
    for lose_ack in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root).await;
        let intent = fixture("authority-ended");
        let fence = LegacyFence::unmanaged_fixture();
        let mut effects = FixtureEffects::new(&intent);
        effects.lose_ack = lose_ack;
        effects.revoke_on_dispatch = true;
        let op = coordinate(&store, &fence, &intent, &effects).await.unwrap();
        assert_eq!(
            op.progress,
            Progress::NeedsAttention {
                reason: Attention::AuthorizationEnded,
                observation: Some(applied(&intent))
            }
        );
        let ledger = store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap();
        assert!(ledger.current.is_none());
        assert!(!fence.allows_legacy());
        let fresh = FixtureEffects::new(&intent);
        assert_eq!(
            coordinate(&store, &fence, &intent, &fresh).await.unwrap(),
            op
        );
        assert_eq!(
            fresh.sends.load(Ordering::Relaxed),
            0,
            "fresh Operator cannot revive the old grant"
        );
        assert_eq!(fresh.queries.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn invalid_unknown_and_disconnected_receipts_remain_fenced() {
    for change in ["digest", "prepared", "unknown", "head", "query"] {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root).await;
        let intent = fixture("uncertain");
        let fence = LegacyFence::unmanaged_fixture();
        let mut effects = FixtureEffects::new(&intent);
        match change {
            "digest" => {
                if let BindingObservation::Observed { snapshot } = &mut effects.result {
                    snapshot.request_digest =
                        crate::machine_protocol::telemetry_binding::binding_digest(
                            b"another-request",
                        );
                }
            }
            "prepared" => effects.result = observed(&intent, BindingOutcome::Prepared {}),
            "unknown" => effects.result = observed(&intent, BindingOutcome::Unknown {}),
            "head" => {
                if let BindingObservation::Observed { snapshot } = &mut effects.result {
                    let mut next = intent.machine_step().unwrap();
                    next.expected = next.after().unwrap();
                    next.change =
                        crate::machine_protocol::telemetry_binding::BindingChange::Revoke {
                            policy_epoch: next.expected.policy_epoch.next().unwrap(),
                        };
                    snapshot.current = Some(next.after().unwrap());
                }
            }
            _ => {
                effects.lose_ack = true;
                effects.lose_query = true;
            }
        }
        let op = coordinate(&store, &fence, &intent, &effects).await.unwrap();
        assert!(
            matches!(op.progress, Progress::NeedsAttention { .. }),
            "{change}"
        );
        assert!(
            store
                .telemetry_binding_ledger(&intent.service_id)
                .await
                .unwrap()
                .unwrap()
                .current
                .is_none()
        );
        assert!(!fence.allows_legacy());
        assert_eq!(effects.sends.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn rejection_retains_managed_initial_head_and_preflight_expiry_creates_no_namespace() {
    let root = tempfile::tempdir().unwrap();
    let store = store(&root).await;
    let intent = fixture("rejection");
    let fence = LegacyFence::unmanaged_fixture();
    let effects = FixtureEffects::new(&intent);
    effects.alive.store(false, Ordering::Release);
    assert!(coordinate(&store, &fence, &intent, &effects).await.is_err());
    assert!(fence.allows_legacy());
    assert!(
        store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(effects.queries.load(Ordering::Relaxed), 0);
    let mut effects = FixtureEffects::new(&intent);
    effects.result = observed(
        &intent,
        BindingOutcome::Rejected {
            reason: BindingRejection::PolicyChanged,
        },
    );
    let op = coordinate(&store, &fence, &intent, &effects).await.unwrap();
    assert!(matches!(op.progress, Progress::Rejected { .. }));
    assert_eq!(
        store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap()
            .unwrap()
            .current,
        Some(crate::machine_protocol::telemetry_binding::BindingSnapshot::initial())
    );
    assert!(
        !fence.allows_legacy(),
        "rejection after namespace creation is not unmanaged absence"
    );
}

#[tokio::test]
async fn recovered_prepared_and_dispatching_do_not_issue_commands() {
    for dispatch in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root).await;
        let intent = fixture("restart");
        let mut op = store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .unwrap()
            .operation;
        if dispatch {
            op = advance(&store, &op, Progress::Dispatching).await.unwrap();
        }
        let fence = LegacyFence::recover(Some(&store), &intent.service_id)
            .await
            .unwrap();
        let effects = FixtureEffects::new(&intent);
        assert_eq!(
            coordinate(&store, &fence, &intent, &effects).await.unwrap(),
            op
        );
        assert_eq!(effects.sends.load(Ordering::Relaxed), 0);
        assert_eq!(effects.queries.load(Ordering::Relaxed), 0);
        assert!(!fence.allows_legacy());
    }
}

#[tokio::test]
async fn authorization_ends_at_each_pre_dispatch_boundary_without_a_command() {
    for checkpoint in 1..=4 {
        let root = tempfile::tempdir().unwrap();
        let store = store(&root).await;
        let intent = fixture("checkpoints");
        let fence = LegacyFence::unmanaged_fixture();
        let mut effects = FixtureEffects::new(&intent);
        effects.revoke_on_check = Some(checkpoint);
        let result = coordinate(&store, &fence, &intent, &effects).await;
        let ledger = store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap();
        match checkpoint {
            1 | 2 => {
                assert!(result.is_err());
                assert!(ledger.is_none());
                assert!(fence.allows_legacy());
            }
            3 => {
                assert_eq!(result.unwrap().progress, Progress::Aborted);
                assert!(ledger.is_some());
                assert!(!fence.allows_legacy());
            }
            _ => {
                assert!(matches!(
                    result.unwrap().progress,
                    Progress::NeedsAttention {
                        reason: Attention::AuthorizationEnded,
                        ..
                    }
                ));
                assert!(ledger.is_some());
                assert!(!fence.allows_legacy());
            }
        }
        assert_eq!(effects.sends.load(Ordering::Relaxed), 0);
        effects.alive.store(true, Ordering::Release);
        assert!(
            !effects.authority.within_budget(),
            "later connection repair cannot revive authority"
        );
    }
}
