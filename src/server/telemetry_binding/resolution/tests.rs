use super::*;
use crate::server::telemetry_binding::tests::FixtureEffects;
use crate::server::{AuthenticatedProductRequest, operator_approval::OperatorApproval};
use crate::telemetry_binding::{resolution::tests::intent as resolution_intent, tests::applied};
use std::sync::atomic::AtomicUsize;

pub(in crate::server::telemetry_binding) fn request(
    before: &Operation,
    observation: Option<&BindingObservation>,
) -> ResolutionIntent {
    let mut request = resolution_intent(before, observation);
    request.actor = crate::plugin_operation::Actor::Product {
        user_id: crate::product_auth::local_product_principal().user_id,
    };
    request
}

pub(in crate::server::telemetry_binding) fn authority(
    auth: ProductRequestAuth<'_>,
    intent: &ResolutionIntent,
) -> TelemetryResolutionAuthority {
    let verified = AuthenticatedProductRequest {
        principal: crate::product_auth::local_product_principal(),
        cookie_session: None,
        device_identity: None,
    };
    OperatorApproval::capture(
        auth,
        &intent.service_id,
        Some(&verified),
        &axum::http::HeaderMap::new(),
    )
    .unwrap()
    .bind_telemetry_resolution(intent)
    .unwrap()
}

fn local() -> FixtureEffects {
    let mut intent = crate::telemetry_binding::fixture("resolution-local-auth");
    intent.actor = crate::plugin_operation::Actor::Product {
        user_id: crate::product_auth::local_product_principal().user_id,
    };
    FixtureEffects::new(&intent)
}

#[tokio::test]
async fn same_epoch_reconnect_cannot_become_the_original_observation_channel() {
    let control = Arc::new(MachineControl::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    control.install("machine-test".into(), "same-epoch".into(), false, 16, tx);
    let observer = LiveObservation::capture(control.clone(), "machine-test").unwrap();
    assert!(observer.current());
    let (tx, _replacement) = tokio::sync::mpsc::unbounded_channel();
    control.install("machine-test".into(), "same-epoch".into(), false, 16, tx);
    assert!(!observer.current());
    let step = crate::telemetry_binding::fixture("reconnected-resolution")
        .machine_step()
        .unwrap();
    assert!(observer.observe(&step).await.is_err());
    assert!(!observer.current());
}

struct Observer {
    value: BindingObservation,
    queries: AtomicUsize,
    ended: AtomicBool,
    disconnect: bool,
    hang: bool,
}
impl Observer {
    fn new(value: BindingObservation) -> Self {
        Self {
            value,
            queries: AtomicUsize::new(0),
            ended: AtomicBool::new(false),
            disconnect: false,
            hang: false,
        }
    }
}
impl Observation for Observer {
    fn current(&self) -> bool {
        !self.ended.load(Ordering::Acquire)
    }
    async fn observe(&self, _step: &BindingStep) -> Result<BindingObservation> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        if self.hang {
            std::future::pending::<()>().await;
        }
        if self.disconnect {
            self.ended.store(true, Ordering::Release);
        }
        Ok(self.value.clone())
    }
}

struct LossyJournal<'a> {
    store: &'a Store,
    commit: bool,
    reads: AtomicUsize,
    writes: AtomicUsize,
}
impl Journal for LossyJournal<'_> {
    async fn read(&self, service: &str) -> Result<Option<Ledger>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.store.read(service).await
    }
    async fn resolve(
        &self,
        permit: &ResolutionPermit,
        current: &(dyn Fn() -> bool + Sync),
    ) -> Result<Operation> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        if self.commit {
            self.store.resolve(permit, current).await?;
        }
        anyhow::bail!("hermetic commit acknowledgement lost")
    }
}

async fn store() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    (root, store)
}

async fn pending(store: &Store, dispatch: bool) -> Operation {
    let mut intent = crate::telemetry_binding::fixture("fresh-resolution");
    intent.expires_at_ms = 1; // Old mutation is expired; it is never resumed.
    let prepared = store
        .change_telemetry_binding(&Change::Begin(&intent), &|| true)
        .await
        .unwrap()
        .operation;
    if dispatch {
        super::super::advance(store, &prepared, Progress::Dispatching)
            .await
            .unwrap()
    } else {
        prepared
    }
}

#[tokio::test]
async fn offline_prepared_resolution_retains_legacy_fence_and_has_no_machine_query() {
    let (_root, store) = store().await;
    let before = pending(&store, false).await;
    let local = local();
    let auth = local.confirmation().auth;
    let request = request(&before, None);
    let observer = Observer::new(applied(&before.intent));
    observer.ended.store(true, Ordering::Release);
    let op = coordinate(
        &store,
        &request,
        authority(auth, &request),
        auth,
        Some(&observer),
    )
    .await
    .unwrap();
    assert_eq!(op.progress, Progress::Aborted);
    assert_eq!(observer.queries.load(Ordering::Relaxed), 0);
    assert!(
        !LegacyFence::recover(Some(&store), &request.service_id)
            .await
            .unwrap()
            .allows_legacy()
    );
    assert_eq!(
        coordinate(
            &store,
            &request,
            authority(auth, &request),
            auth,
            None::<&Observer>
        )
        .await
        .unwrap(),
        op
    );
    assert_eq!(
        store
            .read(&request.service_id)
            .await
            .unwrap()
            .unwrap()
            .resolutions
            .len(),
        1
    );
}

#[tokio::test]
async fn lost_local_commit_ack_only_reads_evidence_and_never_repeats_query_or_write() {
    for commit in [true, false] {
        let (_root, store) = store().await;
        let before = pending(&store, true).await;
        let local = local();
        let auth = local.confirmation().auth;
        let observation = applied(&before.intent);
        let request = request(&before, Some(&observation));
        let observer = Observer::new(observation);
        let journal = LossyJournal {
            store: &store,
            commit,
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
        };
        let result = coordinate(
            &journal,
            &request,
            authority(auth, &request),
            auth,
            Some(&observer),
        )
        .await;
        assert_eq!(result.is_ok(), commit);
        assert_eq!(journal.reads.load(Ordering::Relaxed), 2);
        assert_eq!(journal.writes.load(Ordering::Relaxed), 1);
        assert_eq!(observer.queries.load(Ordering::Relaxed), 1);
        let saved = store.read(&request.service_id).await.unwrap().unwrap();
        assert_eq!(saved.resolutions.len(), usize::from(commit));
        if !commit {
            assert_eq!(saved.operations, vec![before]);
        } else {
            observer.ended.store(true, Ordering::Release);
            coordinate(
                &store,
                &request,
                authority(auth, &request),
                auth,
                Some(&observer),
            )
            .await
            .unwrap();
            assert_eq!(observer.queries.load(Ordering::Relaxed), 1);
        }
    }
}

#[tokio::test]
async fn lost_connection_stale_evidence_and_original_deadline_preserve_unresolved_operation() {
    for mode in ["disconnect", "changed", "timeout", "expired"] {
        let (_root, store) = store().await;
        let before = pending(&store, true).await;
        let local = local();
        let auth = local.confirmation().auth;
        let observation = applied(&before.intent);
        let mut request = request(&before, Some(&observation));
        let mut observer = Observer::new(observation);
        match mode {
            "disconnect" => observer.disconnect = true,
            "changed" => {
                observer.value = crate::telemetry_binding::tests::observed(
                    &before.intent,
                    BindingOutcome::Unknown {},
                )
            }
            "timeout" => {
                observer.hang = true;
                request.expires_at_ms = chrono::Utc::now().timestamp_millis() + 20;
            }
            _ => request.expires_at_ms = 1,
        }
        assert!(
            coordinate(
                &store,
                &request,
                authority(auth, &request),
                auth,
                Some(&observer)
            )
            .await
            .is_err(),
            "{mode}"
        );
        let saved = store.read(&request.service_id).await.unwrap().unwrap();
        assert_eq!(saved.operations, vec![before]);
        assert!(saved.resolutions.is_empty());
        assert!(observer.queries.load(Ordering::Relaxed) <= 1);
    }
}

#[tokio::test]
async fn production_resolution_admission_stays_closed() {
    let (_root, store) = store().await;
    let before = pending(&store, false).await;
    let local = local();
    let auth = local.confirmation().auth;
    let request = request(&before, None);
    assert!(
        resolve(
            &store,
            &request,
            authority(auth, &request),
            auth,
            None::<&Observer>
        )
        .await
        .is_err()
    );
    assert_eq!(
        store
            .read(&request.service_id)
            .await
            .unwrap()
            .unwrap()
            .operations,
        vec![before]
    );
}
