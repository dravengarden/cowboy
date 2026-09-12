use super::*;
use crate::machine_protocol::telemetry_recovery::{RecoverySnapshot, observed_fixture, prepared};
use crate::server::telemetry_binding::tests::FixtureEffects;
use crate::server::{AuthenticatedProductRequest, operator_approval::OperatorApproval};
use std::sync::atomic::AtomicUsize;

fn authority(
    auth: ProductRequestAuth<'_>,
    request: &RecoveryRequest,
    before: &Operation,
) -> TelemetryRecoveryAuthority {
    let verified = AuthenticatedProductRequest {
        principal: crate::product_auth::local_product_principal(),
        cookie_session: None,
        device_identity: None,
    };
    OperatorApproval::capture(
        auth,
        &request.step.service_id,
        Some(&verified),
        &axum::http::HeaderMap::new(),
    )
    .unwrap()
    .bind_telemetry_recovery(request, before)
    .unwrap()
}

struct Remote {
    behavior: &'static str,
    sends: AtomicUsize,
    queries: AtomicUsize,
    ended: AtomicBool,
}
impl Effects for Remote {
    fn current(&self) -> bool {
        !self.ended.load(Ordering::Acquire)
    }
    async fn observe(
        &self,
        request: &RecoveryRequest,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        let count = self.queries.fetch_add(1, Ordering::Relaxed);
        if self.behavior == "hang" {
            std::future::pending::<()>().await;
        }
        if matches!(self.behavior, "disconnect-query" | "history-ended") {
            self.ended.store(true, Ordering::Release);
        }
        if (count > 0 && self.behavior != "missing")
            || matches!(self.behavior, "history" | "history-ended")
        {
            return Ok(observed_fixture(request));
        }
        Ok(RecoveryObservation::Observed {
            snapshot: Box::new(RecoverySnapshot {
                request_digest: request.digest().unwrap(),
                receipt: None,
                binding: prepared(&request.step).unwrap(),
            }),
        })
    }
    async fn recover(
        &self,
        request: &RecoveryRequest,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        assert_eq!(self.sends.fetch_add(1, Ordering::Relaxed), 0);
        if self.behavior == "disconnect-commit" {
            self.ended.store(true, Ordering::Release);
        }
        match self.behavior {
            "unknown" | "missing" | "disconnect-commit" => Err(CommandRequestError {
                certainty: CommandFailure::Unknown,
                detail: "fixture lost ACK".into(),
            }),
            "rejected" => Err(CommandRequestError {
                certainty: CommandFailure::Rejected,
                detail: "closed gate".into(),
            }),
            "not-sent" => Err(CommandRequestError {
                certainty: CommandFailure::NotSent,
                detail: "ended connection".into(),
            }),
            "forged" => {
                let mut changed = request.clone();
                changed.resolution_id.push('x');
                Ok(observed_fixture(&changed))
            }
            _ => Ok(observed_fixture(request)),
        }
    }
}

#[tokio::test]
async fn recovery_never_replays_or_changes_service_and_rechecks_history_authority() {
    for behavior in [
        "ack",
        "unknown",
        "missing",
        "rejected",
        "not-sent",
        "forged",
        "history",
        "history-ended",
        "disconnect-query",
        "disconnect-commit",
        "hang",
    ] {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let mut intent = crate::telemetry_binding::fixture("recovery-service");
        intent.schema = 2;
        intent.actor = crate::plugin_operation::Actor::Product {
            user_id: crate::product_auth::local_product_principal().user_id,
        };
        let local = FixtureEffects::new(&intent);
        let auth = local.confirmation().auth;
        let before = store
            .change_telemetry_binding(&Change::Begin(&intent), &|| true)
            .await
            .unwrap()
            .operation;
        let before = super::super::advance(
            &store,
            &before,
            Progress::NeedsAttention {
                reason: Attention::Uncertain,
                observation: None,
            },
        )
        .await
        .unwrap();
        let original = store
            .telemetry_binding_ledger(&intent.service_id)
            .await
            .unwrap();
        let mut request = request_fixture(&before, &intent.actor);
        if behavior == "hang" {
            request.expires_at_ms = chrono::Utc::now().timestamp_millis() + 100;
        }
        let remote = Remote {
            behavior,
            sends: AtomicUsize::new(0),
            queries: AtomicUsize::new(0),
            ended: AtomicBool::new(false),
        };
        let result = coordinate(
            &store,
            &request,
            authority(auth, &request, &before),
            auth,
            &remote,
        )
        .await;
        assert_eq!(
            result.is_ok(),
            matches!(behavior, "ack" | "unknown" | "history"),
            "{behavior}: {result:?}"
        );
        assert_eq!(
            remote.sends.load(Ordering::Relaxed),
            usize::from(!matches!(
                behavior,
                "history" | "history-ended" | "disconnect-query" | "hang"
            )),
            "{behavior}"
        );
        assert_eq!(
            remote.queries.load(Ordering::Relaxed),
            if matches!(behavior, "unknown" | "missing") {
                2
            } else {
                1
            }
        );
        assert_eq!(
            store
                .telemetry_binding_ledger(&intent.service_id)
                .await
                .unwrap(),
            original
        );
        assert!(
            !LegacyFence::recover(Some(&store), &intent.service_id)
                .await
                .unwrap()
                .allows_legacy()
        );
        assert!(
            recover_machine(
                &store,
                &request,
                authority(auth, &request, &before),
                auth,
                Arc::new(MachineControl::default())
            )
            .await
            .is_err(),
            "production gate stays closed"
        );
    }
}

#[tokio::test]
async fn same_epoch_replacement_cannot_renew_recovery_connection() {
    let control = Arc::new(MachineControl::default());
    let request = crate::machine_protocol::telemetry_recovery::fixture();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    control.install("machine-test".into(), "same-epoch".into(), false, 17, tx);
    let live = Live::bind(control.clone(), &request).unwrap();
    assert!(live.current());
    let (tx, mut replacement) = tokio::sync::mpsc::unbounded_channel();
    control.install("machine-test".into(), "same-epoch".into(), false, 17, tx);
    assert!(!live.current());
    assert!(live.observe(&request).await.is_err());
    assert!(live.recover(&request).await.is_err());
    assert!(replacement.try_recv().is_err());
}
