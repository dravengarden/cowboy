use super::*;
use crate::core::SessionOrigin;
use crate::mcp_binding::Binding;
use crate::store::Store;

pub(super) async fn fixture() -> (Hub, SessionMeta, Policies, Store, tempfile::TempDir) {
    let source = Hub::new();
    source.create_local_session(
        "mcp-session".into(),
        "codex".into(),
        "/tmp/fixture".into(),
        "MCP fixture".into(),
        SessionOrigin::Api,
        false,
    );
    let mut meta = source.session_info("mcp-session").unwrap().meta;
    meta.machine_id = "runtime".into();
    meta.workspace_id = Some("workspace".into());
    let hub = Hub::new();
    assert!(hub.project_persisted_session(meta.clone()));
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect(
        &format!(
            "sqlite://{}?mode=rwc",
            root.path().join("policy.sqlite").display()
        ),
        root.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    (hub, meta, Policies::default(), store, root)
}

pub(super) fn binding() -> Binding {
    serde_json::from_value(json!({"name":"suger","server":{"plugin_id":"suger","generation_digest":format!("sha256:{}", "a".repeat(64))},"tools":["suger_status","suger_read"]})).unwrap()
}

pub(super) fn update(epoch: Option<String>, bindings: Vec<Binding>) -> Update {
    Update {
        expected_epoch: epoch,
        bindings,
        apps: std::collections::BTreeMap::new(),
    }
}

#[tokio::test]
async fn view_grants_are_owner_policy_with_their_own_epoch() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let plain = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    // A policy without views keeps exactly the stored shape it had before.
    let raw = hub.setting(&key(&meta.id)).unwrap();
    assert!(raw.get("apps").is_none());
    let grant = || -> std::collections::BTreeMap<String, Vec<AppGrant>> {
        serde_json::from_value(json!({"suger":[{"uri":"ui://suger/status","capabilities":{"call_tools":["suger_refresh"],"open_links":true}}]})).unwrap()
    };
    let mut with_view = update(Some(plain.epoch.clone()), vec![binding()]);
    with_view.apps = grant();
    let granted = policies
        .replace(&hub, &store, &meta, with_view)
        .await
        .unwrap();
    assert_ne!(granted.epoch, plain.epoch);
    assert_eq!(granted.apps, grant());
    let view = view(&policies, &hub, &meta);
    assert_eq!(view["apps"]["suger"][0]["uri"], "ui://suger/status");
    // The process profile's binding never carries a grant.
    assert!(
        serde_json::to_value(&granted.bindings[0])
            .unwrap()
            .get("apps")
            .is_none()
    );
    // A grant for a binding that is not in the same update is refused.
    let mut dangling = update(Some(granted.epoch.clone()), vec![]);
    dangling.apps = grant();
    assert_eq!(
        policies
            .replace(&hub, &store, &meta, dangling)
            .await
            .unwrap_err(),
        "invalid_bindings"
    );
    // Revoking the bindings revokes the grants with them.
    let revoked = policies
        .replace(&hub, &store, &meta, update(Some(granted.epoch), vec![]))
        .await
        .unwrap();
    assert!(revoked.apps.is_empty());
    assert!(serde_json::from_value::<Update>(json!({"expected_epoch":null,"bindings":[],"apps":{"suger":[{"uri":"ui://suger/status","capabilities":{"connect":["https://x.invalid"]}}]}})).is_err());
}

#[tokio::test]
async fn explicit_binding_and_runtime_placement_are_required() {
    let (hub, meta, policies, store, _root) = fixture().await;
    assert!(
        policies
            .admit(&hub, &meta.id, "runtime", &"0".repeat(32), "suger")
            .is_err()
    );
    let stored = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    assert!(
        policies
            .admit(&hub, &meta.id, "other", &stored.epoch, "suger")
            .is_err()
    );
    assert!(
        policies
            .admit(&hub, &meta.id, "runtime", &stored.epoch, "unknown")
            .is_err()
    );
    let admitted = policies
        .admit(&hub, &meta.id, "runtime", &stored.epoch, "suger")
        .unwrap();
    admitted.current(&policies, &hub).unwrap();
    assert!(admitted.current(&Policies::default(), &hub).is_err());
    let view = view(&policies, &hub, &meta);
    assert_eq!(view["policy_current"], true);
    assert_eq!(view["runtime_ready"], false);
}

#[tokio::test]
async fn revoke_and_restore_identical_policy_never_revives_old_admissions() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let first = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let admitted = policies
        .admit(&hub, &meta.id, "runtime", &first.epoch, "suger")
        .unwrap();
    let revoked = policies
        .replace(
            &hub,
            &store,
            &meta,
            update(Some(first.epoch.clone()), vec![]),
        )
        .await
        .unwrap();
    assert!(admitted.current(&policies, &hub).is_err());
    let restored = policies
        .replace(
            &hub,
            &store,
            &meta,
            update(Some(revoked.epoch), vec![binding()]),
        )
        .await
        .unwrap();
    assert_ne!(first.epoch, restored.epoch);
    assert!(admitted.current(&policies, &hub).is_err());
    assert!(
        policies
            .admit(&hub, &meta.id, "runtime", &first.epoch, "suger")
            .is_err()
    );
    policies
        .admit(&hub, &meta.id, "runtime", &restored.epoch, "suger")
        .unwrap()
        .current(&policies, &hub)
        .unwrap();
    let restarted = Hub::new();
    assert!(restarted.project_persisted_session(meta.clone()));
    let reopened = Store::connect(
        &format!(
            "sqlite://{}?mode=rwc",
            _root.path().join("policy.sqlite").display()
        ),
        _root.path().join("artifacts"),
    )
    .await
    .unwrap();
    restarted.load_settings(reopened.load_settings().await.unwrap());
    assert_eq!(Stored::load(&restarted, &meta.id).unwrap(), restored);
}

#[tokio::test]
async fn concurrent_updates_require_the_current_epoch_and_persist_the_winner() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let first = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        policies.replace(
            &hub,
            &store,
            &meta,
            update(Some(first.epoch.clone()), vec![])
        ),
        policies.replace(
            &hub,
            &store,
            &meta,
            update(Some(first.epoch.clone()), vec![])
        ),
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let winner = left.or(right).unwrap();
    assert_eq!(Stored::load(&hub, &meta.id).unwrap(), winner);
    assert_eq!(
        store
            .load_settings()
            .await
            .unwrap()
            .into_iter()
            .find(|(k, _)| k == &key(&meta.id))
            .unwrap()
            .1,
        json!(winner)
    );
}

#[tokio::test]
async fn delivery_revalidation_rejects_changed_owner_workspace_and_managed_sessions() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let first = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let admitted = policies
        .admit(&hub, &meta.id, "runtime", &first.epoch, "suger")
        .unwrap();
    for mutation in [
        "owner",
        "workspace",
        "cwd",
        "system",
        "trash",
        "closing",
        "binding",
    ] {
        let mut moved = meta.clone();
        match mutation {
            "owner" => moved.owner_user_id = Some("different-user".into()),
            "workspace" => moved.workspace_id = Some("other".into()),
            "cwd" => moved.cwd = "/tmp/other".into(),
            "system" => moved.system = true,
            "trash" => moved.trashed_at_ms = Some(1),
            "closing" => moved.closing = true,
            _ => {
                moved.execution_binding =
                    Some(crate::execution_environment::ExecutionBinding::from_record(
                        json!({"schema":999}),
                    ))
            }
        }
        let changed = Hub::new();
        assert!(changed.project_persisted_session(moved));
        changed.load_settings(vec![(key(&meta.id), json!(first))]);
        assert!(admitted.current(&policies, &changed).is_err(), "{mutation}");
    }
}

#[tokio::test]
async fn admission_deadline_includes_scheduling_and_source_read_time() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let first = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    tokio::time::pause();
    let admitted = policies
        .admit(&hub, &meta.id, "runtime", &first.epoch, "suger")
        .unwrap();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    assert!(admitted.current(&policies, &hub).is_err());
}

#[tokio::test]
async fn corrupt_or_conflicting_storage_never_becomes_a_replaceable_empty_policy() {
    let (hub, meta, policies, store, _root) = fixture().await;
    store
        .put_setting(&key(&meta.id), &json!({"schema":999}))
        .await
        .unwrap();
    assert!(
        policies
            .replace(&hub, &store, &meta, update(None, vec![binding()]))
            .await
            .is_err()
    );
    assert!(hub.setting(&key(&meta.id)).is_none());
    hub.load_settings(store.load_settings().await.unwrap());
    assert!(
        policies
            .replace(&hub, &store, &meta, update(None, vec![binding()]))
            .await
            .is_err()
    );
    assert!(
        policies
            .admit(&hub, &meta.id, "runtime", &"0".repeat(32), "suger")
            .is_err()
    );
}

#[tokio::test]
async fn remote_policy_pins_execution_placement_and_excludes_read_only_reviewers() {
    let (_hub, mut meta, _policies, _store, _root) = fixture().await;
    let mut binding = crate::execution_environment::fixture().decode().unwrap();
    meta.machine_id = binding.runtime.machine_id.clone();
    meta.cwd = binding.runtime.cwd.clone();
    meta.execution_binding = Some(crate::execution_environment::ExecutionBinding::from_record(
        json!(binding),
    ));
    let placement = Placement::from_meta(&meta).unwrap();
    assert_eq!(placement.runtime_machine, "ovh");
    assert_eq!(placement.execution_machine, "hawk");
    assert_eq!(placement.workspace, "cowboy");
    meta.machine_id = "another-machine".into();
    assert!(Placement::from_meta(&meta).is_err());
    meta.machine_id = binding.runtime.machine_id.clone();
    binding.managed = Some(crate::execution_environment::ManagedBindingV1 {
        parent_session_id: "parent-session".into(),
        profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
    });
    meta.execution_binding = Some(crate::execution_environment::ExecutionBinding::from_record(
        json!(binding),
    ));
    assert_eq!(
        Placement::from_meta(&meta).unwrap_err(),
        "managed_session_denied"
    );
}

#[tokio::test]
async fn null_tombstone_and_corrupt_record_allow_only_safe_recovery() {
    let (hub, meta, policies, store, _root) = fixture().await;
    store
        .put_setting(&key(&meta.id), &serde_json::Value::Null)
        .await
        .unwrap();
    hub.load_settings(store.load_settings().await.unwrap());
    let first = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    assert!(!first.bindings.is_empty());
    let invalid = json!({"schema":999,"unknown":"retained"});
    store.put_setting(&key(&meta.id), &invalid).await.unwrap();
    hub.load_settings(store.load_settings().await.unwrap());
    assert_eq!(
        policies
            .replace(&hub, &store, &meta, update(None, vec![binding()]))
            .await
            .unwrap_err(),
        "invalid_stored_policy"
    );
    let revoked = policies
        .replace(&hub, &store, &meta, update(None, vec![]))
        .await
        .unwrap();
    assert!(revoked.bindings.is_empty());
    assert_eq!(
        store
            .load_settings()
            .await
            .unwrap()
            .into_iter()
            .find(|(k, _)| k == &key(&meta.id))
            .unwrap()
            .1,
        json!(revoked)
    );
}

#[tokio::test]
async fn queued_write_does_not_retarget_a_changed_owner() {
    let (_hub, meta, policies, store, _root) = fixture().await;
    let changed = Hub::new();
    let mut current = meta.clone();
    current.owner_user_id = Some("new-owner".into());
    assert!(changed.project_persisted_session(current));
    assert_eq!(
        policies
            .replace(&changed, &store, &meta, update(None, vec![binding()]))
            .await
            .unwrap_err(),
        "placement_changed"
    );
    assert!(store.load_settings().await.unwrap().is_empty());
}

#[tokio::test]
async fn mutation_response_keeps_its_own_committed_epoch() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let saved = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let revoked = policies
        .replace(
            &hub,
            &store,
            &meta,
            update(Some(saved.epoch.clone()), vec![]),
        )
        .await
        .unwrap();
    let response = saved_view(&policies, &hub, &meta, Some(&saved));
    assert_eq!(response["epoch"], saved.epoch);
    assert_eq!(response["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(response["policy_current"], false);
    assert_eq!(view(&policies, &hub, &meta)["epoch"], revoked.epoch);
}

#[tokio::test]
async fn deletion_tombstones_policy_and_refuses_a_queued_write() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let saved = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let guard = policies.mutation.lock().await;
    let pending = policies.replace(&hub, &store, &meta, update(Some(saved.epoch), vec![]));
    tokio::pin!(pending);
    tokio::select! {
        biased;
        _ = &mut pending => panic!("write bypassed the mutation lock"),
        () = tokio::task::yield_now() => {},
    }
    assert!(hub.delete_session(&meta.id));
    assert_eq!(
        hub.with_settings_mut(|settings| settings.get(&key(&meta.id)).cloned()),
        Some(serde_json::Value::Null)
    );
    drop(guard);
    assert_eq!(pending.await.unwrap_err(), "session_unavailable");
}

#[tokio::test]
async fn committed_revoke_notifies_existing_observer_without_polling() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let first = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let mut admission = policies
        .admit(&hub, &meta.id, "runtime", &first.epoch, "suger")
        .unwrap();
    assert!(!admission.changes.has_changed().unwrap());
    assert!(
        policies
            .replace(&hub, &store, &meta, update(None, vec![]))
            .await
            .is_err()
    );
    assert!(!admission.changes.has_changed().unwrap());
    let revoked = policies
        .replace(&hub, &store, &meta, update(Some(first.epoch), vec![]))
        .await
        .unwrap();
    policies
        .replace(
            &hub,
            &store,
            &meta,
            update(Some(revoked.epoch), vec![binding()]),
        )
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        admission.changes.changed(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(admission.current(&policies, &hub).is_err());
    drop(policies);
    assert!(admission.changes.changed().await.is_err());
}

#[tokio::test]
async fn configured_mcp_launch_never_silently_drops_bindings() {
    let (hub, meta, policies, store, _root) = fixture().await;
    assert!(launch_mcp(&hub, &meta).unwrap().is_none());
    let stored = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    assert!(launch_mcp(&hub, &meta).is_err());
    policies
        .replace(&hub, &store, &meta, update(Some(stored.epoch), vec![]))
        .await
        .unwrap();
    assert!(launch_mcp(&hub, &meta).unwrap().is_none());
    hub.load_settings(vec![(key(&meta.id), json!({"bindings":[]}))]);
    assert!(launch_mcp(&hub, &meta).is_err());
    hub.load_settings(vec![(key(&meta.id), serde_json::Value::Null)]);
    assert!(launch_mcp(&hub, &meta).unwrap().is_none());
}
