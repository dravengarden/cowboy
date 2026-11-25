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
                Some(&expected),
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
    // The owner's share fence remains held while Navigate is in flight.
    assert!(fence.try_lock().is_err());

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
                None,
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
        Some(&identity_fox),
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
        Some(&identity_pika),
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
                Some(&expected),
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
                Some(&identity_pika),
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
        Some("own"),
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
async fn revocation_starts_immediately_and_defers_share_leave_while_navigation_pending() {
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
        Some(&identity_before),
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
    // immediately, while share membership & revision wait for settlement.
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
    let shares_while_pending = store.browser_shares(OWNER).await.unwrap();
    assert_eq!(
        shares_while_pending.share(&share_id).unwrap().revision,
        rev_before
    );
    assert!(
        shares_while_pending
            .membership(&Member::account(grok.clone()))
            .is_some()
    );
    assert!(
        !store
            .complete_provider_revocation(OWNER, "revoke-fox")
            .await
            .unwrap()
    );
    let readiness = store
        .provider_revocation_readiness(OWNER, "revoke-fox")
        .await
        .unwrap();
    assert!(
        readiness
            .missing
            .iter()
            .any(|item| item.kind == "browser_share_leave" && item.target == grok.as_str())
    );

    // Once the Machine settles the navigation, the deferred share leave commits
    // and bumps the share revision, allowing revocation completion.
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
    shares::reconcile_revoked_share_leaves(&state, Some(OWNER)).await;
    let shares_after = store.browser_shares(OWNER).await.unwrap();
    assert_eq!(
        shares_after.share(&share_id).unwrap().revision,
        rev_before + 1
    );
    assert!(
        shares_after
            .membership(&Member::account(grok.clone()))
            .is_none()
    );
    assert!(shares_after.membership(&Member::account(codex)).is_some());
    assert!(
        shares_after
            .binding(&share_id, "xai")
            .is_some_and(|b| b.member == Member::account(grok.clone()))
    );
    assert!(
        store
            .complete_provider_revocation(OWNER, "revoke-fox")
            .await
            .unwrap()
    );
}
