use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use crate::store::provider_accounts::browser_shares::{Member, share_key};
use crate::store::provider_accounts::{Change, Mutation, MutationResult};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const OWNER: &str = "owner";

async fn fixture_state(root: &std::path::Path) -> (Arc<AccountsState>, Store) {
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let config_path = root.join("service.toml");
    std::fs::write(
        &config_path,
        "schema = 1\n[accounts]\nmachine_browser = true\n",
    )
    .unwrap();
    let config = crate::config::load(crate::config::Scope::Service, &config_path).unwrap();
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
        provider_auth_sync: Arc::new(provider_auth_sync::Coordinator::default()),
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
            &Mutation {
                expected_revision: rev,
                change: Change::Create {
                    provider_id: provider.into(),
                    label: label.into(),
                },
            },
        )
        .await
        .unwrap();
    let MutationResult::Applied(receipt) = res else {
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
            &Mutation {
                expected_revision: receipt.registry.revision,
                change: Change::SetBrowserEnabled {
                    id: account.clone(),
                    enabled: true,
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(enable, MutationResult::Applied(_)));
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
    let share_id = created.browser_share_id.clone().expect("created share id");
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-fox",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Fox to join share");
    };
    let rev_before = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_before = format!("share:{share_id}:{rev_before}");

    // Another device adds Pika to the same share while Fox's browser stays
    // running and attached with the same instance and attachment_id.
    let MutationResult::Applied(_) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-pika",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Pika to join share");
    };
    let rev_after = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
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
            generation_digest: "digest".into(),
            available: true,
            reason: None,
            max_instances: 2,
            network: Some(crate::machine_protocol::machine_browser::BrowserNetwork::Ipv4OnlyV1),
        }),
    );
    let share_account = share_key(OWNER, &share_id);
    let running_share = InstanceRef {
        account_id: share_account,
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
        IdentityFence::Expected(&identity_before),
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
        IdentityFence::Expected("own"),
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
    let nav = {
        let state = Arc::clone(&state);
        let grok = grok.clone();
        let client_instance = client_instance.clone();
        let expected = identity_after.clone();
        tokio::spawn(async move {
            instance_command(
                &state,
                OWNER,
                (&grok, Authority::Launch),
                ("hawk", &client_instance),
                IdentityFence::Expected(&expected),
                |instance| Command::Navigate {
                    instance,
                    url: "https://grok.com/?_s=usage".into(),
                    attachment_id: Some("att-1".into()),
                },
                Duration::from_secs(5),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!nav.is_finished());
    assert!(rx.try_recv().is_err());

    drop(external_guard);
    let MachineCommand::Browser {
        request_id,
        browser:
            Command::Navigate {
                instance,
                url,
                attachment_id,
            },
    } = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("dispatched navigate command within timeout")
        .expect("channel open")
    else {
        panic!("expected Command::Navigate");
    };
    assert_eq!(instance, running_share);
    assert_eq!(url, "https://grok.com/?_s=usage");
    assert_eq!(attachment_id.as_deref(), Some("att-1"));
    // Share lock is released right after enqueue so revocation never blocks for
    // 20s, while the durable navigation fence keeps share edits blocked.
    assert!(fence.try_lock().is_ok());
    assert!(
        state
            .browsers
            .has_pending_navigation(OWNER, &running_share.account_id)
    );
    assert!(
        store
            .has_pending_browser_navigation(OWNER, &running_share.account_id, now_ms())
            .await
            .unwrap()
    );

    state.machine_control.record_remote(
        &connection,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(request_id),
                ..observed_event.clone()
            }),
        },
    );
    let reply = nav.await.unwrap().expect("matching navigate succeeds");
    assert_eq!(reply["browser_identity"], identity_after);
    // Fence is released once Navigate completes.
    assert!(fence.try_lock().is_ok());

    // 4. Unbound commands (`expected_browser: None`) still succeed without
    // blocking on the share fence.
    let _held = fence.lock().await;
    let unbound = {
        let state = Arc::clone(&state);
        let grok = grok.clone();
        let client_instance = client_instance.clone();
        tokio::spawn(async move {
            instance_command(
                &state,
                OWNER,
                (&grok, Authority::Launch),
                ("hawk", &client_instance),
                IdentityFence::None,
                |instance| Command::Navigate {
                    instance,
                    url: "https://grok.com/?_s=usage".into(),
                    attachment_id: Some("att-1".into()),
                },
                Duration::from_secs(5),
            )
            .await
        })
    };
    let MachineCommand::Browser { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("unbound navigate dispatches without fence")
            .expect("channel open")
    else {
        panic!("expected Browser command");
    };
    state.machine_control.record_remote(
        &connection,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(request_id),
                ..observed_event
            }),
        },
    );
    assert!(unbound.await.unwrap().is_ok());
}

#[tokio::test]
async fn timed_out_navigate_fences_share_membership_until_machine_settles() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    // Install the production browser observers (State + late CommandResult).
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;
    let deepseek = add_account(&store, "acc-ds", "claude-deepseek", "Mole").await;

    // Browser state read advertises identity-fenced navigation support.
    let read = state_value(&state, OWNER, &grok, None).await.unwrap();
    assert_eq!(read["navigate_identity_fenced"], true);

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
    let share_id = created.browser_share_id.clone().expect("created share id");
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-fox",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Fox to join share");
    };
    let rev_fox = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_fox = format!("share:{share_id}:{rev_fox}");

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
            generation_digest: "digest".into(),
            available: true,
            reason: None,
            max_instances: 2,
            network: Some(crate::machine_protocol::machine_browser::BrowserNetwork::Ipv4OnlyV1),
        }),
    );
    let share_account = share_key(OWNER, &share_id);
    let running_share = InstanceRef {
        account_id: share_account,
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
    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };

    // 1. Controller wait times out while Machine is still busy with a snapshot.
    let timed_out = instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        IdentityFence::Expected(&identity_fox),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_millis(30),
    )
    .await;
    assert_eq!(timed_out, Err("browser_command_failed"));
    let MachineCommand::Browser {
        request_id: timed_out_req,
        ..
    } = rx.try_recv().expect("command was sent before timeout")
    else {
        panic!("expected Browser command");
    };

    // Share membership mutations (join, leave, delete) stay fenced until the
    // Machine settles `timed_out_req`.
    let join_blocked = shares::update(
        &state,
        OWNER,
        "join-pika-while-fenced",
        &Mutation {
            expected_revision: joined_fox.registry.revision,
            change: Change::SetBrowserShare {
                member: Member::account(codex.clone()),
                share_id: Some(share_id.clone()),
            },
        },
    )
    .await;
    assert_eq!(join_blocked, Err("browser_navigation_pending"));

    let delete_blocked = shares::update(
        &state,
        OWNER,
        "delete-while-fenced",
        &Mutation {
            expected_revision: joined_fox.registry.revision,
            change: Change::DeleteBrowserShare {
                share_id: share_id.clone(),
            },
        },
    )
    .await;
    assert_eq!(delete_blocked, Err("browser_navigation_pending"));

    // Delayed Machine state reply settles the fence and unblocks membership.
    state.machine_control.record_remote(
        &connection,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(timed_out_req),
                ..observed_event.clone()
            }),
        },
    );
    let (join_ok, noop) = shares::update(
        &state,
        OWNER,
        "join-pika-after-settle",
        &Mutation {
            expected_revision: joined_fox.registry.revision,
            change: Change::SetBrowserShare {
                member: Member::account(codex.clone()),
                share_id: Some(share_id.clone()),
            },
        },
    )
    .await
    .expect("share join succeeds after Machine settles timed-out navigation");
    assert!(!noop);
    let MutationResult::Applied(joined_pika) = join_ok else {
        panic!("expected Applied receipt, got {join_ok:?}");
    };
    let rev_pika = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_pika = format!("share:{share_id}:{rev_pika}");

    // 2. A second timed-out navigation is settled by a late Machine
    // `CommandResult` refusal (e.g. superseded attachment after snapshot).
    let timed_out_2 = instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        IdentityFence::Expected(&identity_pika),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_millis(30),
    )
    .await;
    assert_eq!(timed_out_2, Err("browser_command_failed"));
    let MachineCommand::Browser {
        request_id: refused_req,
        ..
    } = rx.try_recv().expect("second command sent before timeout")
    else {
        panic!("expected Browser command");
    };
    assert_eq!(
        shares::update(
            &state,
            OWNER,
            "join-mole-while-fenced",
            &Mutation {
                expected_revision: joined_pika.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(deepseek.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await,
        Err("browser_navigation_pending")
    );
    // Late CommandResult arrives after the HTTP waiter already dropped.
    state.machine_control.record_remote(
        &connection,
        MachineEvent::CommandResult {
            request_id: refused_req,
            accepted: false,
            detail: Some("browser_attachment_superseded".into()),
        },
    );
    let (join_mole_ok, _) = shares::update(
        &state,
        OWNER,
        "join-mole-after-refusal",
        &Mutation {
            expected_revision: joined_pika.registry.revision,
            change: Change::SetBrowserShare {
                member: Member::account(deepseek.clone()),
                share_id: Some(share_id.clone()),
            },
        },
    )
    .await
    .expect("share join succeeds after late CommandResult refusal settles fence");
    assert!(matches!(join_mole_ok, MutationResult::Applied(_)));
}

#[tokio::test]
async fn disconnected_navigate_stays_fenced_and_reconciles_after_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;
    let deepseek = add_account(&store, "acc-ds", "claude-deepseek", "Mole").await;

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
    let share_id = created.browser_share_id.clone().expect("created share id");
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-fox",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Fox to join share");
    };
    let rev_fox = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_fox = format!("share:{share_id}:{rev_fox}");
    let share_account = share_key(OWNER, &share_id);

    let cap = || crate::machine_protocol::machine_browser::Capability {
        plugin_id: "machine-browser".into(),
        plugin_version: "1.0.1".into(),
        generation_digest: "digest".into(),
        available: true,
        reason: None,
        max_instances: 2,
        network: Some(crate::machine_protocol::machine_browser::BrowserNetwork::Ipv4OnlyV1),
    };

    // Connect epoch-1 and observe running instance (b-heron, inc=3).
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let conn1 = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx1,
    );
    state.browsers.connected(&conn1, Some(cap()));
    let running_3 = InstanceRef {
        account_id: share_account.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    let observed_3 = StateEvent {
        request_id: None,
        instance: running_3.clone(),
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
        &conn1,
        &crate::machine_protocol::machine_browser::Event::State(observed_3.clone()),
    );
    let client_3 = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };

    // Dispatch Navigate and immediately drop the connection before any reply.
    let nav = {
        let state = Arc::clone(&state);
        let grok = grok.clone();
        let client_3 = client_3.clone();
        let expected = identity_fox.clone();
        tokio::spawn(async move {
            instance_command(
                &state,
                OWNER,
                (&grok, Authority::Launch),
                ("hawk", &client_3),
                IdentityFence::Expected(&expected),
                |instance| Command::Navigate {
                    instance,
                    url: "https://grok.com/?_s=usage".into(),
                    attachment_id: Some("att-1".into()),
                },
                Duration::from_secs(5),
            )
            .await
        })
    };
    let MachineCommand::Browser { .. } = tokio::time::timeout(Duration::from_secs(2), rx1.recv())
        .await
        .expect("navigate dispatched")
        .expect("channel open")
    else {
        panic!("expected Browser command");
    };
    state.machine_control.disconnect("hawk");
    state.browsers.disconnected(&conn1);
    assert_eq!(nav.await.unwrap(), Err("browser_command_failed"));

    // Fence MUST stay raised across disconnect and while offline.
    assert!(state.browsers.has_pending_navigation(OWNER, &share_account));
    assert_eq!(
        shares::update(
            &state,
            OWNER,
            "join-pika-offline",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await,
        Err("browser_navigation_pending")
    );

    // Reconnect on epoch-2: fence STILL stays raised until reconciled.
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let conn2 = state.machine_control.install(
        "hawk".into(),
        "epoch-2".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx2,
    );
    state.browsers.connected(&conn2, Some(cap()));
    assert!(state.browsers.has_pending_navigation(OWNER, &share_account));

    // Subcase A: reconcile sees the same instance still running -> sends Hibernate.
    let reconcile = tokio::spawn({
        let state = Arc::clone(&state);
        async move { shares::reconcile_orphaned_navigations(&state, Some("hawk")).await }
    });
    let MachineCommand::Browser {
        request_id: query_req,
        browser: Command::Query { account_id },
    } = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
        .await
        .expect("reconcile queries orphaned key")
        .expect("channel open")
    else {
        panic!("expected Command::Query");
    };
    assert_eq!(account_id, share_account);
    state.machine_control.record_remote(
        &conn2,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(query_req),
                observed_at_ms: 200,
                ..observed_3.clone()
            }),
        },
    );
    let MachineCommand::Browser {
        request_id: hib_req,
        browser: Command::Hibernate { instance },
    } = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
        .await
        .expect("reconcile hibernates surviving orphaned instance")
        .expect("channel open")
    else {
        panic!("expected Command::Hibernate");
    };
    assert_eq!(instance, running_3);
    // `Stopping` alone must NOT settle the fence!
    state.machine_control.record_remote(
        &conn2,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(hib_req),
                state: BrowserState::Stopping,
                observed_at_ms: 210,
                ..observed_3.clone()
            }),
        },
    );
    reconcile.await.unwrap();
    assert!(state.browsers.has_pending_navigation(OWNER, &share_account));

    // Confirmed exit of (b-heron, inc=3) settles the fence and unblocks membership.
    state.machine_control.record_remote(
        &conn2,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: None,
                state: BrowserState::Hibernated,
                reason: Some(Reason::OwnerRequest),
                observed_at_ms: 220,
                ..observed_3
            }),
        },
    );
    assert!(!state.browsers.has_pending_navigation(OWNER, &share_account));
    let (MutationResult::Applied(joined_pika), _) = shares::update(
        &state,
        OWNER,
        "join-pika-after-reconcile",
        &Mutation {
            expected_revision: joined_fox.registry.revision,
            change: Change::SetBrowserShare {
                member: Member::account(codex.clone()),
                share_id: Some(share_id.clone()),
            },
        },
    )
    .await
    .expect("share join succeeds after confirmed exit") else {
        panic!("expected Applied receipt");
    };
    let rev_pika = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_pika = format!("share:{share_id}:{rev_pika}");

    // Subcase B: after another disconnect+reconnect, authoritative Query reporting
    // empty-id Hibernated (e.g. instance already failed/exited or Machine rebooted)
    // settles immediately without sending Hibernate.
    let running_4 = InstanceRef {
        account_id: share_account.clone(),
        instance_id: "b-heron".into(),
        incarnation: 4,
    };
    let observed_4 = StateEvent {
        request_id: None,
        instance: running_4,
        state: BrowserState::Attached,
        reason: None,
        geometry: None,
        scale: None,
        deadline_ms: None,
        attachment_id: Some("att-2".into()),
        snapshot_generation: None,
        snapshot_at_ms: None,
        profile_revision: Some(1),
        unsaved_state_lost: false,
        observed_at_ms: 300,
    };
    state.browsers.observe(
        &conn2,
        &crate::machine_protocol::machine_browser::Event::State(observed_4),
    );
    let client_4 = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 4,
    };
    let nav2 = {
        let state = Arc::clone(&state);
        let grok = grok.clone();
        tokio::spawn(async move {
            instance_command(
                &state,
                OWNER,
                (&grok, Authority::Launch),
                ("hawk", &client_4),
                IdentityFence::Expected(&identity_pika),
                |instance| Command::Navigate {
                    instance,
                    url: "https://grok.com/?_s=usage".into(),
                    attachment_id: Some("att-2".into()),
                },
                Duration::from_secs(5),
            )
            .await
        })
    };
    let MachineCommand::Browser { .. } = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
        .await
        .expect("second navigate dispatched")
        .expect("channel open")
    else {
        panic!("expected Browser command");
    };
    state.machine_control.disconnect("hawk");
    state.browsers.disconnected(&conn2);
    assert_eq!(nav2.await.unwrap(), Err("browser_command_failed"));

    let (tx3, mut rx3) = mpsc::unbounded_channel();
    let conn3 = state.machine_control.install(
        "hawk".into(),
        "epoch-3".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx3,
    );
    state.browsers.connected(&conn3, Some(cap()));
    let reconcile2 = tokio::spawn({
        let state = Arc::clone(&state);
        async move { shares::reconcile_orphaned_navigations(&state, Some("hawk")).await }
    });
    let MachineCommand::Browser {
        request_id: query_req2,
        browser: Command::Query { .. },
    } = tokio::time::timeout(Duration::from_secs(2), rx3.recv())
        .await
        .expect("reconcile queries on epoch-3")
        .expect("channel open")
    else {
        panic!("expected Command::Query");
    };
    state.machine_control.record_remote(
        &conn3,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(query_req2),
                instance: InstanceRef {
                    account_id: share_account.clone(),
                    instance_id: String::new(),
                    incarnation: 0,
                },
                state: BrowserState::Hibernated,
                reason: Some(Reason::Failed),
                geometry: None,
                scale: None,
                deadline_ms: None,
                attachment_id: None,
                snapshot_generation: None,
                snapshot_at_ms: None,
                profile_revision: Some(1),
                unsaved_state_lost: false,
                observed_at_ms: 400,
            }),
        },
    );
    reconcile2.await.unwrap();
    assert!(!state.browsers.has_pending_navigation(OWNER, &share_account));
    assert!(rx3.try_recv().is_err());
    let (MutationResult::Applied(_), _) = shares::update(
        &state,
        OWNER,
        "join-mole-after-query-hibernated",
        &Mutation {
            expected_revision: joined_pika.registry.revision,
            change: Change::SetBrowserShare {
                member: Member::account(deepseek),
                share_id: Some(share_id),
            },
        },
    )
    .await
    .expect("share join succeeds after authoritative absent Query") else {
        panic!("expected Applied receipt");
    };
}

#[tokio::test]
async fn unrelated_hibernated_and_stopping_events_do_not_clear_pending_navigation() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(
        &conn,
        Some(crate::machine_protocol::machine_browser::Capability {
            plugin_id: "machine-browser".into(),
            plugin_version: "1.0.1".into(),
            generation_digest: "digest".into(),
            available: true,
            reason: None,
            max_instances: 2,
            network: Some(crate::machine_protocol::machine_browser::BrowserNetwork::Ipv4OnlyV1),
        }),
    );
    let running_3 = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-fox".into(),
        incarnation: 3,
    };
    let observed_3 = StateEvent {
        request_id: None,
        instance: running_3.clone(),
        state: BrowserState::Attached,
        reason: None,
        geometry: None,
        scale: None,
        deadline_ms: None,
        attachment_id: Some("att-1".into()),
        snapshot_generation: None,
        snapshot_at_ms: None,
        profile_revision: Some(2),
        unsaved_state_lost: false,
        observed_at_ms: 100,
    };
    state.browsers.observe(
        &conn,
        &crate::machine_protocol::machine_browser::Event::State(observed_3.clone()),
    );
    let timed_out = instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &running_3),
        IdentityFence::Expected("own"),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_millis(20),
    )
    .await;
    assert_eq!(timed_out, Err("browser_command_failed"));
    assert!(rx.try_recv().is_ok());
    assert!(state.browsers.has_pending_navigation(OWNER, &grok));

    // Unrelated refused reset (empty instance_id Hibernated) must NOT clear the fence.
    for reason in [Reason::Superseded, Reason::Failed, Reason::Busy] {
        state.machine_control.record_remote(
            &conn,
            MachineEvent::Browser {
                browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                    request_id: Some("other-reset-req".into()),
                    instance: InstanceRef {
                        account_id: grok.clone(),
                        instance_id: String::new(),
                        incarnation: 0,
                    },
                    state: BrowserState::Hibernated,
                    reason: Some(reason),
                    geometry: None,
                    scale: None,
                    deadline_ms: None,
                    attachment_id: None,
                    snapshot_generation: None,
                    snapshot_at_ms: None,
                    profile_revision: Some(2),
                    unsaved_state_lost: false,
                    observed_at_ms: 110,
                }),
            },
        );
        assert!(state.browsers.has_pending_navigation(OWNER, &grok));
    }

    // `Stopping` for the tracked incarnation does not confirm unit exit.
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: None,
                state: BrowserState::Stopping,
                observed_at_ms: 120,
                ..observed_3.clone()
            }),
        },
    );
    assert!(state.browsers.has_pending_navigation(OWNER, &grok));

    // An older incarnation's Hibernated or CapabilityLost must NOT clear the fence.
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: None,
                instance: InstanceRef {
                    account_id: grok.clone(),
                    instance_id: "b-fox".into(),
                    incarnation: 2,
                },
                state: BrowserState::Hibernated,
                reason: Some(Reason::Grace),
                observed_at_ms: 130,
                ..observed_3.clone()
            }),
        },
    );
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::CapabilityLost,
        },
    );
    assert!(state.browsers.has_pending_navigation(OWNER, &grok));

    // Confirmed exit of incarnation 3 clears the fence.
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: None,
                state: BrowserState::Hibernated,
                reason: Some(Reason::OwnerRequest),
                observed_at_ms: 140,
                ..observed_3
            }),
        },
    );
    assert!(!state.browsers.has_pending_navigation(OWNER, &grok));
}

#[tokio::test]
async fn revocation_starts_immediately_keeps_rollback_safe_plan_and_keeps_navigation_fence() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;

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
    let share_id = created.browser_share_id.clone().expect("created share id");
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-fox",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Fox to join share");
    };
    let MutationResult::Applied(joined_pika) = store
        .mutate_browser_shares(
            OWNER,
            "share-join-pika",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Pika to join share");
    };
    let rev_before = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_before = format!("share:{share_id}:{rev_before}");
    let share_account = share_key(OWNER, &share_id);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(
        &conn,
        Some(crate::machine_protocol::machine_browser::Capability {
            plugin_id: "machine-browser".into(),
            plugin_version: "1.0.1".into(),
            generation_digest: "digest".into(),
            available: true,
            reason: None,
            max_instances: 2,
            network: Some(crate::machine_protocol::machine_browser::BrowserNetwork::Ipv4OnlyV1),
        }),
    );
    let running_share = InstanceRef {
        account_id: share_account.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    let observed_event = StateEvent {
        request_id: None,
        instance: running_share,
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
        &conn,
        &crate::machine_protocol::machine_browser::Event::State(observed_event.clone()),
    );
    let mut fox_viewer = state.browsers.register_viewer(
        (share_account.as_str(), OWNER, grok.as_str()),
        "att-1",
        3,
        &conn,
    );

    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    let timed_out = instance_command(
        &state,
        OWNER,
        (&grok, Authority::Launch),
        ("hawk", &client_instance),
        IdentityFence::Expected(&identity_before),
        |instance| Command::Navigate {
            instance,
            url: "https://grok.com/?_s=usage".into(),
            attachment_id: Some("att-1".into()),
        },
        Duration::from_millis(20),
    )
    .await;
    assert_eq!(timed_out, Err("browser_command_failed"));
    let MachineCommand::Browser {
        request_id: nav_req,
        ..
    } = rx.try_recv().expect("navigate sent")
    else {
        panic!("expected Browser command");
    };

    // Start revocation of Fox while navigation is unsettled: access is revoked
    // immediately, Fox leaves the share while keeping its family binding, Plan
    // stays 100% rollback-compatible (no unknown fields), and the pending
    // navigation keeps fencing share mutations and resets until settled.
    let revoked = super::update_value(
        &state,
        OWNER,
        "revoke-fox",
        &Mutation {
            expected_revision: joined_pika.registry.revision,
            change: Change::BeginRevocation { id: grok.clone() },
        },
    )
    .await
    .expect("BeginRevocation starts immediately");
    assert_eq!(revoked["status"], "revoking");
    assert!(*fox_viewer.borrow_and_update());
    assert_eq!(
        authorize(&state, OWNER, &grok, Authority::Launch)
            .await
            .err(),
        Some("account_unavailable")
    );
    let shares_after_revoke = store.browser_shares(OWNER).await.unwrap();
    let rev_after = shares_after_revoke.share(&share_id).unwrap().revision;
    assert_eq!(rev_after, rev_before + 1);
    assert!(
        shares_after_revoke
            .membership(&Member::account(grok.clone()))
            .is_none()
    );
    assert!(
        shares_after_revoke
            .membership(&Member::account(codex.clone()))
            .is_some()
    );
    assert!(
        shares_after_revoke
            .binding(&share_id, "xai")
            .is_some_and(|b| b.member == Member::account(grok.clone()))
    );
    let identity_after = format!("share:{share_id}:{rev_after}");

    // Revocation Plan has no unknown fields (older Controllers deserialize cleanly
    // on rollback) and completes independently of unrelated share Machines.
    let receipt = store
        .provider_account_receipt(OWNER, "revoke-fox")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&receipt)
            .unwrap()
            .contains("browser_share_leave_pending")
    );
    assert!(
        store
            .complete_provider_revocation(OWNER, "revoke-fox")
            .await
            .unwrap()
    );

    // Even after Fox revoked, the pending navigation still fences shared-profile
    // resets and share edits until settled.
    store
        .admit_shared_browser_placement(
            OWNER,
            (&Member::account(codex.clone()), &share_id),
            "hawk",
            "b-heron",
            1,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reset(&state, OWNER, &codex, ("hawk", 1), &identity_after).await,
        Err("browser_navigation_pending")
    );
    let rev_now = store.provider_accounts(OWNER).await.unwrap().revision;
    assert_eq!(
        shares::update(
            &state,
            OWNER,
            "delete-share-while-nav",
            &Mutation {
                expected_revision: rev_now,
                change: Change::DeleteBrowserShare {
                    share_id: share_id.clone(),
                },
            },
        )
        .await,
        Err("browser_navigation_pending")
    );

    // Revoking Fox cannot hibernate (Authority::Stop) the share's browser.
    assert_eq!(
        instance_command(
            &state,
            OWNER,
            (&grok, Authority::Stop),
            ("hawk", &client_instance),
            IdentityFence::None,
            |instance| Command::Hibernate { instance },
            Duration::from_secs(5),
        )
        .await,
        Err("browser_not_running")
    );
    assert!(rx.try_recv().is_err());

    // Once the Machine settles the navigation, the fence clears.
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(nav_req),
                observed_at_ms: 150,
                ..observed_event
            }),
        },
    );
    assert!(
        !shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );
}

async fn state_with_store(root: &std::path::Path, store: Store) -> Arc<AccountsState> {
    let config_path = root.join("service.toml");
    std::fs::write(
        &config_path,
        "schema = 1\n[accounts]\nmachine_browser = true\n",
    )
    .unwrap();
    let config = crate::config::load(crate::config::Scope::Service, &config_path).unwrap();
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
    Arc::new(AccountsState {
        service_id: "svc-test".into(),
        service_config_path: config_path,
        service_config,
        hub,
        store: Some(store),
        supervisor,
        shutdown,
        runtime_router,
        machine_control,
        browsers: coordinator::Coordinator::default(),
        account_controls: Arc::default(),
        plugin_catalog,
        provider_catalog,
        provider_auth,
        provider_auth_sync: Arc::new(provider_auth_sync::Coordinator::default()),
        provider_auth_executors: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        plugin_lifecycle_fences: PluginLifecycleFences::default(),
        usage,
        public_origins: Arc::new(Vec::new()),
        product_auth_enabled: false,
        device_access: Arc::new(crate::client_auth::DeviceAccessSessions::default()),
        product_authentication: Arc::new(crate::auth_plugins::ProductAuthentication::test_default(
            None,
        )),
    })
}

fn ipv4_cap() -> crate::machine_protocol::machine_browser::Capability {
    crate::machine_protocol::machine_browser::Capability {
        plugin_id: "machine-browser".into(),
        plugin_version: "1.0.1".into(),
        generation_digest: "digest".into(),
        available: true,
        reason: None,
        max_instances: 2,
        network: Some(crate::machine_protocol::machine_browser::BrowserNetwork::Ipv4OnlyV1),
    }
}

#[tokio::test]
async fn controller_restart_keeps_durable_navigation_fence_and_reconciles_without_url_replay() {
    let root = tempfile::tempdir().unwrap();
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        root.path().join("restart.sqlite").display()
    );
    let store1 = Store::connect(&db_url, root.path().join("artifacts"))
        .await
        .unwrap();
    store1.migrate().await.unwrap();
    let state1 = state_with_store(root.path(), store1.clone()).await;
    install_observer(&state1);
    let grok = add_account(&store1, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store1, "acc-codex", "codex", "Pika").await;
    let rev = store1.provider_accounts(OWNER).await.unwrap().revision;
    let MutationResult::Applied(created) = store1
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
        panic!("share create");
    };
    let share_id = created.browser_share_id.clone().unwrap();
    let MutationResult::Applied(joined_fox) = store1
        .mutate_browser_shares(
            OWNER,
            "share-join-fox",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("fox join");
    };
    let rev_fox = store1
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_fox = format!("share:{share_id}:{rev_fox}");
    let share_account = share_key(OWNER, &share_id);
    store1
        .admit_shared_browser_placement(
            OWNER,
            (&Member::account(grok.clone()), &share_id),
            "hawk",
            "b-heron",
            1,
        )
        .await
        .unwrap()
        .unwrap();

    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let conn1 = state1.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx1,
    );
    state1.browsers.connected(&conn1, Some(ipv4_cap()));
    let running = InstanceRef {
        account_id: share_account.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    let observed = StateEvent {
        request_id: None,
        instance: running.clone(),
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
    state1.browsers.observe(
        &conn1,
        &crate::machine_protocol::machine_browser::Event::State(observed.clone()),
    );
    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    assert_eq!(
        instance_command(
            &state1,
            OWNER,
            (&grok, Authority::Launch),
            ("hawk", &client_instance),
            IdentityFence::Expected(&identity_fox),
            |instance| Command::Navigate {
                instance,
                url: "https://grok.com/?_s=usage".into(),
                attachment_id: Some("att-1".into())
            },
            Duration::from_millis(20),
        )
        .await,
        Err("browser_command_failed"),
    );
    assert!(rx1.try_recv().is_ok());

    // Simulate full Controller restart with fresh empty Coordinator on the same DB.
    drop(state1);
    drop(store1);
    let store2 = Store::connect(&db_url, root.path().join("artifacts"))
        .await
        .unwrap();
    let state2 = state_with_store(root.path(), store2.clone()).await;
    install_observer(&state2);

    assert_eq!(
        shares::update(
            &state2,
            OWNER,
            "join-after-restart",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex.clone()),
                    share_id: Some(share_id.clone())
                },
            }
        )
        .await,
        Err("browser_navigation_pending"),
    );
    assert_eq!(
        reset(&state2, OWNER, &grok, ("hawk", 1), &identity_fox).await,
        Err("browser_navigation_pending")
    );

    // Machine connects to restarted Controller -> reconciles via Query + Hibernate (no URL replay).
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let conn2 = state2.machine_control.install(
        "hawk".into(),
        "epoch-2".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx2,
    );
    state2.browsers.connected(&conn2, Some(ipv4_cap()));
    let reconcile = tokio::spawn({
        let state2 = Arc::clone(&state2);
        async move { shares::reconcile_orphaned_navigations(&state2, Some("hawk")).await }
    });
    let MachineCommand::Browser {
        request_id: q_req,
        browser: Command::Query { account_id },
    } = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected Command::Query");
    };
    assert_eq!(account_id, share_account);
    state2.machine_control.record_remote(
        &conn2,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(q_req),
                observed_at_ms: 200,
                ..observed.clone()
            }),
        },
    );
    let MachineCommand::Browser {
        request_id: h_req,
        browser: Command::Hibernate { instance },
    } = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected Command::Hibernate");
    };
    assert_eq!(instance, running);
    state2.machine_control.record_remote(
        &conn2,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(h_req),
                state: BrowserState::Hibernated,
                reason: Some(Reason::OwnerRequest),
                observed_at_ms: 210,
                ..observed
            }),
        },
    );
    reconcile.await.unwrap();
    assert!(
        !store2
            .has_pending_browser_navigation(OWNER, &share_account, now_ms())
            .await
            .unwrap()
    );
    assert!(
        shares::update(
            &state2,
            OWNER,
            "join-after-reconcile",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex),
                    share_id: Some(share_id)
                },
            }
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn matching_stopping_reply_keeps_fence_until_confirmed_exit() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;
    let rev = store.provider_accounts(OWNER).await.unwrap().revision;
    let MutationResult::Applied(created) = store
        .mutate_browser_shares(
            OWNER,
            "sc",
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
        panic!("sc");
    };
    let share_id = created.browser_share_id.clone().unwrap();
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "jf",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("jf");
    };
    let rev_fox = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_fox = format!("share:{share_id}:{rev_fox}");
    let share_account = share_key(OWNER, &share_id);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(&conn, Some(ipv4_cap()));
    let observed = StateEvent {
        request_id: None,
        instance: InstanceRef {
            account_id: share_account.clone(),
            instance_id: "b-heron".into(),
            incarnation: 3,
        },
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
        &conn,
        &crate::machine_protocol::machine_browser::Event::State(observed.clone()),
    );
    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };
    let nav = tokio::spawn({
        let state = Arc::clone(&state);
        let grok = grok.clone();
        let expected = identity_fox.clone();
        async move {
            instance_command(
                &state,
                OWNER,
                (&grok, Authority::Launch),
                ("hawk", &client_instance),
                IdentityFence::Expected(&expected),
                |instance| Command::Navigate {
                    instance,
                    url: "https://grok.com/?_s=usage".into(),
                    attachment_id: Some("att-1".into()),
                },
                Duration::from_secs(5),
            )
            .await
        }
    });
    let MachineCommand::Browser { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("expected Browser command");
    };
    // Matching request_id answering `Stopping` must NOT clear the fence!
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(request_id),
                state: BrowserState::Stopping,
                reason: Some(Reason::Failed),
                observed_at_ms: 110,
                ..observed.clone()
            }),
        },
    );
    assert!(nav.await.unwrap().is_ok());
    assert!(state.browsers.has_pending_navigation(OWNER, &share_account));
    assert!(
        store
            .has_pending_browser_navigation(OWNER, &share_account, now_ms())
            .await
            .unwrap()
    );
    assert_eq!(
        shares::update(
            &state,
            OWNER,
            "join-while-stopping",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex.clone()),
                    share_id: Some(share_id.clone())
                },
            }
        )
        .await,
        Err("browser_navigation_pending"),
    );
    // Confirmed exit clears both in-memory and durable fence.
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: None,
                state: BrowserState::Hibernated,
                reason: Some(Reason::Failed),
                observed_at_ms: 120,
                ..observed
            }),
        },
    );
    assert!(
        shares::update(
            &state,
            OWNER,
            "join-after-exit",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex),
                    share_id: Some(share_id)
                },
            }
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn cached_web_attachment_navigate_enforces_recorded_viewer_identity_and_fences_share() {
    let root = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(root.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;
    let deepseek = add_account(&store, "acc-ds", "claude-deepseek", "Mole").await;
    let rev = store.provider_accounts(OWNER).await.unwrap().revision;
    let MutationResult::Applied(created) = store
        .mutate_browser_shares(
            OWNER,
            "sc",
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
        panic!("sc");
    };
    let share_id = created.browser_share_id.clone().unwrap();
    let MutationResult::Applied(joined_fox) = store
        .mutate_browser_shares(
            OWNER,
            "jf",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("jf");
    };
    let rev_before = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_before = format!("share:{share_id}:{rev_before}");
    let share_account = share_key(OWNER, &share_id);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(&conn, Some(ipv4_cap()));
    let observed = StateEvent {
        request_id: None,
        instance: InstanceRef {
            account_id: share_account.clone(),
            instance_id: "b-heron".into(),
            incarnation: 3,
        },
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
        &conn,
        &crate::machine_protocol::machine_browser::Event::State(observed),
    );
    let _v1 = state.browsers.register_viewer_with_identity(
        (share_account.as_str(), OWNER, grok.as_str()),
        "att-1",
        3,
        &conn,
        identity_before,
    );
    let MutationResult::Applied(joined_pika) = store
        .mutate_browser_shares(
            OWNER,
            "jp",
            &Mutation {
                expected_revision: joined_fox.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("jp");
    };
    let rev_after = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share_id)
        .unwrap()
        .revision;
    let identity_after = format!("share:{share_id}:{rev_after}");
    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };

    // Cached Web bundle omits expected_browser, sends attachment_id -> refused on stale share rev.
    assert_eq!(
        instance_command(
            &state,
            OWNER,
            (&grok, Authority::Launch),
            ("hawk", &client_instance),
            IdentityFence::Attachment("att-1"),
            |instance| Command::Navigate {
                instance,
                url: "https://grok.com/?_s=usage".into(),
                attachment_id: Some("att-1".into())
            },
            Duration::from_secs(5),
        )
        .await,
        Err("browser_identity_changed"),
    );
    assert!(rx.try_recv().is_err());

    // Current attachment dispatches AND engages the durable navigation fence.
    let _v2 = state.browsers.register_viewer_with_identity(
        (share_account.as_str(), OWNER, grok.as_str()),
        "att-2",
        3,
        &conn,
        identity_after,
    );
    assert_eq!(
        instance_command(
            &state,
            OWNER,
            (&grok, Authority::Launch),
            ("hawk", &client_instance),
            IdentityFence::Attachment("att-2"),
            |instance| Command::Navigate {
                instance,
                url: "https://grok.com/?_s=usage".into(),
                attachment_id: Some("att-2".into())
            },
            Duration::from_millis(20),
        )
        .await,
        Err("browser_command_failed"),
    );
    assert!(rx.try_recv().is_ok());
    assert_eq!(
        shares::update(
            &state,
            OWNER,
            "jm",
            &Mutation {
                expected_revision: joined_pika.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(deepseek),
                    share_id: Some(share_id)
                },
            }
        )
        .await,
        Err("browser_navigation_pending"),
    );
}

#[tokio::test]
async fn settled_tombstone_prevents_orphan_reconciler_from_hibernating_live_browser() {
    let dir = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(dir.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(&conn, Some(ipv4_cap()));
    let running = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-live".into(),
        incarnation: 5,
    };
    let observed = StateEvent {
        request_id: None,
        instance: running.clone(),
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
        &conn,
        &crate::machine_protocol::machine_browser::Event::State(observed),
    );
    let observed_handle = state.browsers.state_on("hawk", grok.as_str()).unwrap();
    // Simulate a navigation recorded in SQL and settled in memory before the
    // SQL DELETE has committed (tombstone stays active in settled_navigation_ids).
    shares::begin_durable_navigation(&state, OWNER, &grok, &observed_handle, "nav-tombstone")
        .await
        .unwrap();
    state.browsers.settle_navigation("nav-tombstone");
    assert!(state.browsers.is_navigation_settled("nav-tombstone"));
    // Even if seed_durable_navigation is called while the SQL row still exists,
    // the tombstone rejects re-seeding.
    state
        .browsers
        .seed_durable_navigation(coordinator::PendingNavigation {
            request_id: "nav-tombstone".into(),
            owner: OWNER.into(),
            browser_key: grok.clone(),
            machine_id: "hawk".into(),
            connection: None,
            instance: running.clone(),
            observed_at_ms: 100,
            started_at_ms: now_ms(),
        });
    assert!(!state.browsers.is_navigation_pending("nav-tombstone"));
    // Reconciler drains the settled row without ever sending Query or Hibernate.
    shares::reconcile_orphaned_navigations(&state, Some("hawk")).await;
    assert!(rx.try_recv().is_err());
    assert!(
        !store
            .has_pending_browser_navigation_request("nav-tombstone")
            .await
            .unwrap()
    );
    // The tombstone stays retained for SETTLED_TOMBSTONE_TTL even after the SQL
    // DELETE commits, so a late-landing cancelled INSERT is deleted cleanly on
    // the next pass without seeding or hibernating the live browser.
    assert!(state.browsers.is_navigation_settled("nav-tombstone"));
    store
        .record_browser_pending_navigation(
            &crate::store::provider_accounts::browser_pending_navigations::PendingNavigationRecord {
                request_id: "nav-tombstone".into(),
                owner_user_id: OWNER.into(),
                browser_key: grok.clone(),
                machine_id: "hawk".into(),
                instance_id: running.instance_id.clone(),
                incarnation: running.incarnation,
                observed_at_ms: 100,
                started_at_ms: now_ms(),
            },
        )
        .await
        .unwrap();
    shares::reconcile_orphaned_navigations(&state, Some("hawk")).await;
    assert!(rx.try_recv().is_err());
    assert!(!state.browsers.is_navigation_pending("nav-tombstone"));
    assert!(
        !store
            .has_pending_browser_navigation_request("nav-tombstone")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn pending_navigation_expires_on_ttl_and_clears_when_machine_revoked_or_placement_removed() {
    let dir = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(dir.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;
    let rev = store.provider_accounts(OWNER).await.unwrap().revision;
    let MutationResult::Applied(created) = store
        .mutate_browser_shares(
            OWNER,
            "sc",
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
        panic!("sc");
    };
    let share_id = created.browser_share_id.clone().unwrap();
    let share_account = share_key(OWNER, &share_id);
    let MutationResult::Applied(joined) = store
        .mutate_browser_shares(
            OWNER,
            "jf",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("jf");
    };

    // 1. Expired row (> PENDING_NAVIGATION_TTL_MS old) on an offline Machine
    // does not permanently wedge share edits or resets, and reconciler prunes it.
    let expired_started = now_ms()
        - crate::store::provider_accounts::browser_pending_navigations::PENDING_NAVIGATION_TTL_MS
        - 10_000;
    store
        .record_browser_pending_navigation(
            &crate::store::provider_accounts::browser_pending_navigations::PendingNavigationRecord {
                request_id: "nav-expired".into(),
                owner_user_id: OWNER.into(),
                browser_key: share_account.clone(),
                machine_id: "offline-gone".into(),
                instance_id: "b-heron".into(),
                incarnation: 1,
                observed_at_ms: 100,
                started_at_ms: expired_started,
            },
        )
        .await
        .unwrap();
    assert!(
        !shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );
    // An expired in-memory seeded entry (with no SQL row left) also obeys TTL
    // and is pruned by reconciler without wedging share edits or resets.
    state
        .browsers
        .seed_durable_navigation(coordinator::PendingNavigation {
            request_id: "nav-mem-expired".into(),
            owner: OWNER.into(),
            browser_key: share_account.clone(),
            machine_id: "offline-gone".into(),
            connection: None,
            instance: InstanceRef {
                account_id: share_account.clone(),
                instance_id: "b-heron".into(),
                incarnation: 1,
            },
            observed_at_ms: 100,
            started_at_ms: expired_started,
        });
    assert!(
        !shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );
    shares::reconcile_orphaned_navigations(&state, None).await;
    assert!(!state.browsers.is_navigation_pending("nav-mem-expired"));
    assert!(
        !store
            .has_pending_browser_navigation_request("nav-expired")
            .await
            .unwrap()
    );

    // 2. Removing the browser placement atomically clears its pending navigation.
    store
        .admit_shared_browser_placement(
            OWNER,
            (&Member::account(grok.clone()), &share_id),
            "hawk",
            "b-heron",
            now_ms(),
        )
        .await
        .unwrap()
        .unwrap();
    store
        .record_browser_pending_navigation(
            &crate::store::provider_accounts::browser_pending_navigations::PendingNavigationRecord {
                request_id: "nav-placement".into(),
                owner_user_id: OWNER.into(),
                browser_key: share_account.clone(),
                machine_id: "hawk".into(),
                instance_id: "b-heron".into(),
                incarnation: 2,
                observed_at_ms: 100,
                started_at_ms: now_ms(),
            },
        )
        .await
        .unwrap();
    assert!(
        shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );
    store
        .remove_browser_placement(OWNER, &share_account, "hawk")
        .await
        .unwrap();
    assert!(
        !shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );

    // 3. Revoking a Machine enrollment immediately unblocks share edits and
    // reconciler prunes the row without needing the Machine to reconnect.
    let token = store
        .create_machine_enrollment("gone-machine", "Gone", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(
            &token,
            "ssh-ed25519 CQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "CQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        )
        .await
        .unwrap();
    store
        .record_browser_pending_navigation(
            &crate::store::provider_accounts::browser_pending_navigations::PendingNavigationRecord {
                request_id: "nav-revoked-machine".into(),
                owner_user_id: OWNER.into(),
                browser_key: share_account.clone(),
                machine_id: "gone-machine".into(),
                instance_id: "b-heron".into(),
                incarnation: 3,
                observed_at_ms: 100,
                started_at_ms: now_ms(),
            },
        )
        .await
        .unwrap();
    assert!(
        shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );
    store.revoke_machine("gone-machine").await.unwrap();
    assert!(
        !shares::has_pending_navigation(&state, OWNER, &share_account)
            .await
            .unwrap()
    );
    shares::reconcile_orphaned_navigations(&state, None).await;
    assert!(
        !store
            .has_pending_browser_navigation_request("nav-revoked-machine")
            .await
            .unwrap()
    );
    assert!(
        shares::update(
            &state,
            OWNER,
            "jp",
            &Mutation {
                expected_revision: joined.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex),
                    share_id: Some(share_id),
                },
            },
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn unbound_navigate_still_takes_share_lock_and_fences_share_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let (state, store) = fixture_state(dir.path()).await;
    install_observer(&state);
    let grok = add_account(&store, "acc-grok", "grok", "Fox").await;
    let codex = add_account(&store, "acc-codex", "codex", "Pika").await;
    let rev = store.provider_accounts(OWNER).await.unwrap().revision;
    let MutationResult::Applied(created) = store
        .mutate_browser_shares(
            OWNER,
            "sc",
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
        panic!("sc");
    };
    let share_id = created.browser_share_id.clone().unwrap();
    let share_account = share_key(OWNER, &share_id);
    let MutationResult::Applied(joined) = store
        .mutate_browser_shares(
            OWNER,
            "jf",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(grok.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("jf");
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = state.machine_control.install(
        "hawk".into(),
        "epoch-1".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    state.browsers.connected(&conn, Some(ipv4_cap()));
    let observed = StateEvent {
        request_id: None,
        instance: InstanceRef {
            account_id: share_account.clone(),
            instance_id: "b-heron".into(),
            incarnation: 3,
        },
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
        &conn,
        &crate::machine_protocol::machine_browser::Event::State(observed.clone()),
    );
    let client_instance = InstanceRef {
        account_id: grok.clone(),
        instance_id: "b-heron".into(),
        incarnation: 3,
    };

    // Unbound navigate (no expected_browser, no attachment_id — e.g. MCP or
    // address bar) still tracks a durable fence on the shared browser.
    assert_eq!(
        instance_command(
            &state,
            OWNER,
            (&grok, Authority::Launch),
            ("hawk", &client_instance),
            IdentityFence::UnboundNavigate,
            |instance| Command::Navigate {
                instance,
                url: "https://grok.com/?_s=usage".into(),
                attachment_id: None,
            },
            Duration::from_millis(20),
        )
        .await,
        Err("browser_command_failed"),
    );
    let MachineCommand::Browser {
        request_id: nav_req,
        ..
    } = rx.try_recv().expect("navigate sent")
    else {
        panic!("expected Browser command");
    };
    assert_eq!(
        shares::update(
            &state,
            OWNER,
            "jp",
            &Mutation {
                expected_revision: joined.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex.clone()),
                    share_id: Some(share_id.clone()),
                },
            },
        )
        .await,
        Err("browser_navigation_pending"),
    );

    // Settling the unbound navigate lifts the fence.
    state.machine_control.record_remote(
        &conn,
        MachineEvent::Browser {
            browser: crate::machine_protocol::machine_browser::Event::State(StateEvent {
                request_id: Some(nav_req),
                observed_at_ms: 150,
                ..observed
            }),
        },
    );
    assert!(
        shares::update(
            &state,
            OWNER,
            "jp",
            &Mutation {
                expected_revision: joined.registry.revision,
                change: Change::SetBrowserShare {
                    member: Member::account(codex),
                    share_id: Some(share_id),
                },
            },
        )
        .await
        .is_ok()
    );
}
