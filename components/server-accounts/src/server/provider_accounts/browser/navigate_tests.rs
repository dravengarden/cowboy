use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::store::provider_accounts::browser_shares::{Change, Member, Mutation, MutationResult};
use std::pin::pin;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const OWNER: &str = "owner";

async fn fixture_state(root: &std::path::Path) -> (Arc<AccountsState>, Store) {
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let config_path = root.join("service.toml");
    std::fs::write(&config_path, "[accounts]\nmachine_browser = true\n").unwrap();
    let config = crate::config::load(crate::config::Scope::Service, Some(&config_path))
        .config
        .expect("valid test service config");
    let service_config = crate::config::Handle::new(config);
    let hub = Hub::new();
    let runtime_router = RuntimeRouter::new();
    let supervisor = Arc::new(Supervisor::new(
        hub.clone(),
        root.to_path_buf(),
        0,
        Arc::clone(&runtime_router),
    ));
    let machine_control = Arc::new(MachineControl::default());
    let plugin_catalog = Arc::new(crate::plugin_catalog::PluginCatalog::open(root, None).unwrap());
    let provider_catalog = Arc::new(
        crate::provider_catalog::ProviderCatalog::open(root, Arc::clone(&plugin_catalog)).unwrap(),
    );
    let provider_auth = Arc::new(
        crate::provider_service::ProviderAuthService::open(&root.join("provider-auth")).unwrap(),
    );
    let usage = UsageService::with_plugin_catalog(
        Some(store.clone()),
        None,
        Arc::clone(&plugin_catalog),
        Arc::clone(&machine_control),
    )
    .with_config(service_config.clone());
    let (_shutdown_tx, shutdown) = watch::channel(false);
    let state = Arc::new(AccountsState {
        service_id: "svc-test".into(),
        service_config_path: config_path,
        service_config,
        hub,
        store: Some(store.clone()),
        supervisor,
        shutdown,
        runtime_router,
        machine_control,
        browsers: coordinator::Coordinator::default(),
        account_controls: Arc::default(),
        plugin_catalog,
        provider_catalog,
        provider_auth,
        provider_auth_sync: Arc::new(crate::server::provider_auth_sync::Coordinator::default()),
        provider_auth_executors: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        plugin_lifecycle_fences: PluginLifecycleFences::default(),
        usage,
        public_origins: Arc::new(Vec::new()),
        product_auth_enabled: false,
        device_access: Arc::new(crate::client_auth::DeviceAccessSessions::default()),
        product_authentication: Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
            None,
        )),
    });
    (state, store)
}

async fn add_account(store: &Store, op: &str, provider: &str, label: &str) -> ProviderAccountId {
    let rev = store.provider_accounts(OWNER).await.unwrap().revision;
    let res = store
        .mutate_provider_accounts(
            OWNER,
            op,
            &crate::store::provider_accounts::Mutation {
                expected_revision: rev,
                change: crate::store::provider_accounts::Change::Create {
                    provider_id: provider.into(),
                    label: label.into(),
                },
            },
        )
        .await
        .unwrap();
    let crate::store::provider_accounts::MutationResult::Applied(receipt) = res else {
        panic!("expected account create receipt, got {res:?}");
    };
    let account = receipt
        .registry
        .accounts
        .iter()
        .find(|a| a.label == label)
        .expect("created account")
        .id
        .clone();
    let enable = store
        .mutate_provider_accounts(
            OWNER,
            &format!("{op}-browser"),
            &crate::store::provider_accounts::Mutation {
                expected_revision: receipt.registry.revision,
                change: crate::store::provider_accounts::Change::SetBrowserEnabled {
                    account_id: account.clone(),
                    enabled: true,
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        enable,
        crate::store::provider_accounts::MutationResult::Applied(_)
    ));
    account
}

#[test]
fn navigate_body_accepts_expected_browser_and_rejects_unknown_fields() {
    let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let with_expected: NavigateBody = serde_json::from_value(json!({
        "dataset": "d",
        "machine_id": "hawk",
        "instance": {
            "account_id": account.as_str(),
            "instance_id": "b-1",
            "incarnation": 1
        },
        "url": "https://grok.com/?_s=usage",
        "attachment_id": "att-1",
        "expected_browser": "share:s1:2"
    }))
    .unwrap();
    assert_eq!(
        with_expected.expected_browser.as_deref(),
        Some("share:s1:2")
    );
    assert_eq!(with_expected.attachment_id.as_deref(), Some("att-1"));

    let without_expected: NavigateBody = serde_json::from_value(json!({
        "machine_id": "hawk",
        "instance": {
            "account_id": account.as_str(),
            "instance_id": "b-1",
            "incarnation": 1
        },
        "url": "https://grok.com/?_s=usage"
    }))
    .unwrap();
    assert_eq!(without_expected.expected_browser, None);

    let unknown = serde_json::from_value::<NavigateBody>(json!({
        "machine_id": "hawk",
        "instance": {
            "account_id": account.as_str(),
            "instance_id": "b-1",
            "incarnation": 1
        },
        "url": "https://grok.com/?_s=usage",
        "unexpected": true
    }));
    assert!(unknown.is_err());
}

#[tokio::test]
async fn identity_bound_navigate_refuses_stale_share_revision_and_holds_share_fence() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;

    // Create a share and add Fox (rev 1 -> rev 2).
    let rev = store.provider_accounts(OWNER).await.unwrap().revision;
    let MutationResult::Applied(created) = store
        .mutate_browser_shares(
            OWNER,
            "share-create",
            &Mutation {
                expected_revision: rev,
                change: Change::CreateBrowserShare {
                    label: "Heron".into(),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected share creation");
    };
    let share_id = created.shares.shares[0].share_id.clone();
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-fox",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::AddBrowserShareMember {
                    share_id: share_id.clone(),
                    member: Member::account(grok.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Fox to join share");
    };
    let rev_before = joined_fox.shares.share(&share_id).unwrap().revision;
    let identity_before = format!("share:{share_id}:{rev_before}");

    // Another device adds Pika to the same share while Fox's browser stays
    // running and attached with the same instance and attachment_id.
    let MutationResult::Applied(joined_pika) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-pika",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::AddBrowserShareMember {
                    share_id: share_id.clone(),
                    member: Member::account(codex.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Pika to join share");
    };
    let rev_after = joined_pika.shares.share(&share_id).unwrap().revision;
    assert_ne!(rev_before, rev_after);
    let identity_after = format!("share:{share_id}:{rev_after}");

    // Connect a fake Machine reporting the share's browser running & attached.
    let (tx, mut rx) = mpsc::unbounded_channel();
    let connection = state.machine_control.install(
        "hawk".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(
        &connection,
        Some(crate::machine_protocol::machine_browser::Capability {
            plugin_id: "machine-browser".into(),
            plugin_version: "1.0.1".into(),
            available: true,
            reason: None,
            enforces_ipv4_only: true,
        }),
    );
    let share_key = crate::store::provider_accounts::browser_shares::share_key(&share_id);
    let running_share = InstanceRef {
        account_id: share_key.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    let observed_event = StateEvent {
        request_id: None,
        instance: running_share.clone(),
        state: BrowserState::Attached,
        reason: None,
        geometry: None,
        scale: None,
        deadline_ms: None,
        attachment_id: Some("att-1".into()),
        snapshot_generation: None,
        snapshot_at_ms: None,
        profile_revision: Some(1),
        unsaved_state_lost: false,
        observed_at_ms: 100,
    };
    state.browsers.observe(
        &connection,
        &crate::machine_protocol::machine_browser::Event::State(observed_event.clone()),
    );

    // Client-side instance ref names the card's account id (`grok`), as HTTP does.
    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };

    // 1. Stale share revision is refused BEFORE any Machine dispatch, even
    // though the running instance and attachment_id are still valid.
    let stale = instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        Some(&identity_before),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(stale, Err("browser_identity_changed"));
    assert!(rx.try_recv().is_err());

    // 2. Wrong own/share identity is also refused with browser_identity_changed.
    let wrong_own = instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        Some("own"),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(wrong_own, Err("browser_identity_changed"));
    assert!(rx.try_recv().is_err());

    // 3. Identity-bound navigate waits for an active share fence, then holds
    // the fence across dispatch until the Machine answers.
    let fence = state.browsers.share_lock(OWNER);
    let external_guard = fence.lock().await;
    let mut nav = pin!(instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        Some(&identity_after),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_secs(5),
    ));
    assert!(futures::poll!(&mut nav).is_pending());
    assert!(rx.try_recv().is_err());

    drop(external_guard);
    assert!(futures::poll!(&mut nav).is_pending());
    let MachineCommand::Browser {
        request_id,
        browser:
            Command::Navigate {
                instance,
                url,
                attachment_id,
            },
    } = rx.try_recv().expect("dispatched navigate command")
    else {
        panic!("expected Command::Navigate");
    };
    assert_eq!(instance, running_share);
    assert_eq!(url, "https://grok.com/?_s=usage");
    assert_eq!(attachment_id.as_deref(), Some("att-1"));
    // The owner's share fence remains held while Navigate is in flight.
    assert!(fence.try_lock().is_err());

    state.machine_control.record_remote(
        &connection,
        MachineEvent::Browser {
            event: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(request_id),
                ..observed_event.clone()
            }),
        },
    );
    let reply = nav.await.expect("matching navigate succeeds");
    assert_eq!(reply["browser_identity"], identity_after);
    // Fence is released once Navigate completes.
    assert!(fence.try_lock().is_ok());

    // 4. Unbound commands (`expected_browser: None`) still succeed without
    // blocking on the share fence.
    let _held = fence.lock().await;
    let mut unbound = pin!(instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        None,
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_secs(5),
    ));
    assert!(futures::poll!(&mut unbound).is_pending());
    let MachineCommand::Browser { request_id, .. } = rx
        .try_recv()
        .expect("unbound navigate dispatches without fence")
    else {
        panic!("expected Browser command");
    };
    state.machine_control.record_remote(
        &connection,
        MachineEvent::Browser {
            event: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(request_id),
                ..observed_event
            }),
        },
    );
    assert!(unbound.await.is_ok());
}
