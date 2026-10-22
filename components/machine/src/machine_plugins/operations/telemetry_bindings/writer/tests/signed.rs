use super::*;

struct Fixture {
    _root: tempfile::TempDir,
    store: Arc<MachinePluginStore>,
    desired: DesiredPlugin,
    step: BindingStep,
    policy: PathBuf,
}

impl Fixture {
    async fn new(version: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = tempfile::tempdir().unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
                .unwrap();
        let desired = crate::machine_plugins::tests::telemetry_release(&publisher, version);
        let machine = root.path().join("machine");
        let store =
            Arc::new(MachinePluginStore::new(&machine, Platform::Linux, "x86_64".into()).unwrap());
        store.install(&desired).await.unwrap();
        store.enable_installation_tracking().await.unwrap();
        let active = store.inventory_one("victoria").unwrap().unwrap();
        let mut step = fixture();
        step.change = BindingChange::Select {
            installation: BindingInstallation {
                plugin_id: active.plugin_id,
                plugin_version: active.plugin_version,
                generation_digest: active.generation_digest.try_into().unwrap(),
                installation_revision: active.installation_revision.unwrap(),
                contract_fingerprint: active.contract_fingerprint.try_into().unwrap(),
            },
            policy_epoch: "1".to_owned().try_into().unwrap(),
        };
        enable(&store.operations.telemetry_bindings);
        Self {
            _root: root,
            store,
            desired,
            step,
            policy: machine.join("telemetry.json"),
        }
    }

    fn configure(&self) {
        let selection = self.step.after().unwrap().selection.unwrap();
        atomic_write(&self.policy, &serde_json::to_vec(&serde_json::json!({
            "plugin": {
                "plugin_id": selection.plugin_id, "plugin_version": selection.plugin_version,
                "generation_digest": selection.generation_digest
            },
            "logs": {"base_url": "http://127.0.0.1:1", "bearer_token": "fixture-secret-not-for-receipts"},
            "metrics": null, "traces": null
        })).unwrap(), 0o600).unwrap();
    }

    async fn commit(&self, step: &BindingStep) -> WriteResult<BindingObservation> {
        let owner = scope(step);
        self.store
            .commit_telemetry_binding(step, owner.telemetry_binding(step).unwrap())
            .await
    }
}

#[tokio::test]
async fn signed_selection_and_restoration_require_fresh_machine_policy_not_old_receipts() {
    for version in ["1.0.0", "1.1.0"] {
        let fixture = Fixture::new(version).await;
        assert_eq!(
            fixture.commit(&fixture.step).await,
            Err(WriteError::Rejected(BindingRejection::PolicyChanged))
        );
        assert!(!fixture.store.operations.telemetry_bindings.path.exists());
        fixture.configure();
        let policy_before = fs::read(&fixture.policy).unwrap();
        fixture.commit(&fixture.step).await.unwrap();
        let revoke = next(&fixture.step, 2);
        fs::rename(
            &fixture.policy,
            fixture.policy.with_extension("retained-fixture"),
        )
        .unwrap();
        fixture.commit(&revoke).await.unwrap();
        let restoration = restore(&revoke, 3);
        assert_eq!(
            fixture.commit(&restoration).await,
            Err(WriteError::Rejected(BindingRejection::PolicyChanged))
        );
        fixture.configure();
        let observed = result(fixture.commit(&restoration).await.unwrap());
        assert_eq!(observed.current, Some(restoration.after().unwrap()));
        assert_eq!(
            fs::read(&fixture.policy).unwrap(),
            policy_before,
            "never rewrites private policy"
        );
        let evidence =
            fs::read_to_string(&fixture.store.operations.telemetry_bindings.path).unwrap();
        for forbidden in [
            "fixture-secret",
            "127.0.0.1",
            "bearer_token",
            "base_url",
            "auth_generation",
        ] {
            assert!(!evidence.contains(forbidden));
        }
        // Committing configuration is NOT an export lease. The existing host
        // command remains fenced, even after a successful managed selection.
        let installation = restoration.after().unwrap().selection.unwrap();
        let owner = scope(&restoration);
        let invocation = owner.host(crate::machine_plugins::PluginHostRequest {
            plugin_id: installation.plugin_id,
            plugin_version: installation.plugin_version,
            generation_digest: installation.generation_digest.into(),
            auth_generation: None,
            operation: PluginHostOperation::ExportOtlp,
            payload: serde_json::json!({}),
        });
        assert!(
            !fixture
                .store
                .invoke_host(invocation)
                .await
                .unwrap_err()
                .started
        );
    }
}

#[tokio::test]
async fn signed_writer_rejects_installation_aba_and_never_reinstalls_to_make_restore_work() {
    let fixture = Fixture::new("1.1.0").await;
    fixture.configure();
    fixture.commit(&fixture.step).await.unwrap();
    let revoke = next(&fixture.step, 2);
    fixture.commit(&revoke).await.unwrap();
    let before = fs::read(&fixture.store.operations.telemetry_bindings.path).unwrap();
    let replacement = fixture.store.install(&fixture.desired).await.unwrap();
    let restoration = restore(&revoke, 3);
    assert_eq!(
        fixture.commit(&restoration).await,
        Err(WriteError::Rejected(BindingRejection::TargetChanged))
    );
    assert_eq!(
        fs::read(&fixture.store.operations.telemetry_bindings.path).unwrap(),
        before
    );
    assert_eq!(
        fixture
            .store
            .inventory_one("victoria")
            .unwrap()
            .unwrap()
            .installation_revision,
        replacement.installation_revision
    );
}

#[tokio::test]
async fn queued_binding_commit_cannot_cross_a_disconnected_or_expired_connection() {
    let fixture = Fixture::new("1.1.0").await;
    fixture.configure();
    let owner = scope(&fixture.step);
    let lease = owner.telemetry_binding(&fixture.step).unwrap();
    let lifecycle = fixture.store.lifecycle.lock().await;
    let store = Arc::clone(&fixture.store);
    let step = fixture.step.clone();
    let task = tokio::spawn(async move { store.commit_telemetry_binding(&step, lease).await });
    drop(owner);
    let replacement = scope(&fixture.step);
    drop(lifecycle);
    assert_eq!(
        task.await.unwrap(),
        Err(WriteError::Rejected(BindingRejection::AuthorizationEnded))
    );
    let lease = replacement.telemetry_binding(&fixture.step).unwrap();
    lease.expire_for_test();
    let _locked = fixture.store.lifecycle.lock().await;
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(1),
            fixture.store.commit_telemetry_binding(&fixture.step, lease)
        )
        .await
        .unwrap(),
        Err(WriteError::Rejected(BindingRejection::Expired))
    );
    assert!(!fixture.store.operations.telemetry_bindings.path.exists());
}

#[tokio::test]
async fn actual_private_policy_or_signed_bytes_changed_after_prepare_never_commit_selection() {
    for mutation in ["replace_same", "make_public", "tamper_signed_release"] {
        let fixture = Fixture::new("1.1.0").await;
        fixture.configure();
        let _lifecycle = fixture.store.lifecycle.lock().await;
        let bindings = &fixture.store.operations.telemetry_bindings;
        let owner = scope(&fixture.step);
        let lease = owner.telemetry_binding(&fixture.step).unwrap();
        let after = fixture.step.after().unwrap();
        let mut policy = None;
        let mut wrote_intent = false;
        let observed = bindings
            .commit_with_io(
                &fixture.step,
                &lease,
                &mut || fixture.store.check_binding_target(&after, &mut policy),
                |bytes| {
                    durable(&bindings.path, bytes)?;
                    if !wrote_intent {
                        wrote_intent = true;
                        match mutation {
                            "replace_same" => fixture.configure(),
                            "make_public" => fs::set_permissions(
                                &fixture.policy,
                                fs::Permissions::from_mode(0o644),
                            )?,
                            "tamper_signed_release" => {
                                let digest = String::from(
                                    after.selection.as_ref().unwrap().generation_digest.clone(),
                                );
                                let (_, _, content) = fixture
                                    .store
                                    .verified_plugin_generation("victoria", &digest)?;
                                atomic_write(&content.join("package.cowboy-plugin"), b"{}", 0o600)?;
                            }
                            _ => unreachable!(),
                        }
                    }
                    Ok(())
                },
            )
            .unwrap();
        let snapshot = result(observed);
        assert_eq!(snapshot.current, Some(BindingSnapshot::initial()));
        assert_eq!(
            snapshot.receipt.unwrap().outcome,
            BindingOutcome::Rejected {
                reason: if mutation == "tamper_signed_release" {
                    BindingRejection::TargetChanged
                } else {
                    BindingRejection::PolicyChanged
                }
            }
        );
        assert!(bindings.ensure_legacy_allowed().is_err());
    }
}
