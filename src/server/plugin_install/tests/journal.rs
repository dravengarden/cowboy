use super::*;
use crate::plugin_operation::installation::fixture;

struct CheckedEffects<'a> {
    store: &'a Store,
    intent: &'a InstallIntent,
    inner: MockEffects,
}

impl Effects for CheckedEffects<'_> {
    async fn authorized(&self) -> bool {
        self.inner.authorized().await
    }
    async fn needs_auth_sync(&self, before: bool) -> bool {
        self.inner.needs_auth_sync(before).await
    }
    async fn sync_auth(&self) -> bool {
        let op = self
            .store
            .plugin_install_operation(&self.intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            op.phase,
            if self.inner.syncs.load(Ordering::SeqCst) == 0 {
                InstallPhase::SyncingAuthentication
            } else {
                InstallPhase::MachineAcknowledged
            }
        );
        self.inner.sync_auth().await
    }
    async fn install(&self) -> Result<(), CommandRequestError> {
        assert_eq!(
            self.store
                .plugin_install_operation(&self.intent.operation_id)
                .await
                .unwrap()
                .unwrap()
                .phase,
            InstallPhase::Installing
        );
        self.inner.install().await
    }
}

#[tokio::test]
async fn each_live_effect_has_a_prior_durable_phase_and_completion_precedes_fence_release() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let intent = fixture("ordered-effects");
    store.begin_plugin_install(&intent).await.unwrap();
    let fences = fences(None);
    let mut fence = InstallationFence::acquire(&fences, slot()).unwrap();
    fence.disposition = Disposition::Uncertain;
    let effects = CheckedEffects {
        store: &store,
        intent: &intent,
        inner: MockEffects::default(),
    };
    let mut progress = Progress::new(&store, &intent);
    assert_eq!(
        super::super::coordinate(&effects, &mut fence, &mut progress)
            .await
            .unwrap(),
        Outcome::Installed
    );
    assert_eq!(
        store
            .plugin_install_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .phase,
        InstallPhase::Completed
    );
    drop(fence);
    assert!(fences.read().is_empty());
}

#[tokio::test]
async fn failed_progress_commit_never_sends_a_later_effect_or_releases_the_slot() {
    for denied in [
        "syncing_authentication",
        "installing",
        "machine_acknowledged",
        "completed",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("install.sqlite");
        let store = Store::connect(
            &format!("sqlite://{}", path.display()),
            root.path().join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let intent = fixture(denied);
        store.begin_plugin_install(&intent).await.unwrap();
        // Fault injection is confined to this test-owned database. No production
        // SQL/configuration is opened, and the denied phase is a closed fixture.
        rusqlite::Connection::open(&path).unwrap().execute_batch(&format!(
            "CREATE TRIGGER refuse_progress BEFORE UPDATE OF phase ON plugin_install_operations \
             WHEN NEW.phase = '{denied}' BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END;"
        )).unwrap();
        let fences = fences(None);
        let mut fence = InstallationFence::acquire(&fences, slot()).unwrap();
        fence.disposition = Disposition::Uncertain;
        let effects = CheckedEffects {
            store: &store,
            intent: &intent,
            inner: MockEffects::default(),
        };
        let mut progress = Progress::new(&store, &intent);
        assert!(
            super::super::coordinate(&effects, &mut fence, &mut progress)
                .await
                .is_err()
        );
        progress.storage_failure().await;
        drop(fence);
        assert_eq!(
            fences.read().get(&slot()),
            Some(&PluginFenceState::NeedsReconcile)
        );
        assert_eq!(
            effects.inner.installs.load(Ordering::SeqCst),
            usize::from(matches!(denied, "machine_acknowledged" | "completed"))
        );
        let op = store
            .plugin_install_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(op.phase, InstallPhase::NeedsAttention);
        assert_eq!(op.problem, Some(InstallProblem::StorageFailure));
        assert!(store.begin_plugin_install(&fixture("retry")).await.is_err());
    }
}

#[tokio::test]
async fn process_restart_reconstructs_install_fences_alongside_uninstall_fences() {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::connect(
            &format!("sqlite://{}", root.path().join("install.sqlite").display()),
            root.path().join("artifacts"),
        )
        .await
        .unwrap(),
    );
    store.migrate().await.unwrap();
    let intent = fixture("interrupted");
    store.begin_plugin_install(&intent).await.unwrap();
    let fences = fences(None);
    let mut fence = InstallationFence::acquire(&fences, slot()).unwrap();
    fence.disposition = Disposition::Uncertain;
    let task_store = Arc::clone(&store);
    let task_intent = intent.clone();
    let interrupted = tokio::spawn(async move {
        let effects = MockEffects {
            panic: true,
            ..MockEffects::default()
        };
        super::super::coordinate(
            &effects,
            &mut fence,
            &mut Progress::new(&task_store, &task_intent),
        )
        .await
    })
    .await;
    assert!(interrupted.unwrap_err().is_panic());
    let before = store
        .plugin_install_operation(&intent.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.phase, InstallPhase::Installing);
    let uninstall = crate::plugin_operation::fixture("uninstall-other-machine");
    store.begin_plugin_uninstall(&uninstall).await.unwrap();
    let recovered = crate::server::plugin_uninstall::recover_fences(Some(&store), "service-test")
        .await
        .unwrap();
    assert_eq!(recovered.read().len(), 2);
    assert_eq!(
        recovered.read().get(&slot()),
        Some(&PluginFenceState::NeedsReconcile)
    );
    assert!(InstallationFence::acquire(&recovered, slot()).is_err());
    assert_eq!(
        store
            .plugin_install_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap()
            .attention_from,
        Some(InstallPhase::Installing)
    );
}

#[tokio::test]
async fn duplicate_observation_is_exact_and_contains_no_actor_envelope_or_replay_authority() {
    let intent = fixture("observation");
    let request = PluginInstallRequest {
        operation_id: intent.operation_id.clone(),
        version: intent.plugin_version.clone(),
        digest: intent.generation_digest.clone(),
    };
    let op = crate::plugin_operation::installation::InstallOperation {
        intent: intent.clone(),
        phase: InstallPhase::Completed,
        problem: None,
        attention_from: None,
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    let response = super::super::journal::duplicate_response(
        &op,
        &intent.service_id,
        &intent.actor,
        &intent.machine_id,
        &intent.plugin_id,
        &request,
    );
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["execution_authorized"], false);
    assert_eq!(json["operation"]["phase"], "completed");
    for private in [
        "actor",
        "user-test",
        "envelope_digest",
        "request_id",
        "expires_at_ms",
    ] {
        assert!(!String::from_utf8_lossy(&bytes).contains(private));
    }
    let mut changed = request;
    changed.digest = format!("sha256:{}", "f".repeat(64));
    let response = super::super::journal::duplicate_response(
        &op,
        &intent.service_id,
        &intent.actor,
        &intent.machine_id,
        &intent.plugin_id,
        &changed,
    );
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("completed"));
}
