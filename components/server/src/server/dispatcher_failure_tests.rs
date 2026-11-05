
use super::*;

#[test]
fn a_permanent_refusal_ends_an_earlier_retried_wait() {
    let hub = Hub::new();
    hub.create_local_session(
        "s".into(),
        "claude-code".into(),
        "/tmp".into(),
        "test".into(),
        crate::core::SessionOrigin::Web,
        false,
    );
    let key = format!(
        "{}s",
        crate::core::settings_keys::SESSION_RESOURCE_WAIT_PREFIX
    );
    let not_ready = format!(
        "{}{}",
        session_maintenance::WAIT_PREFIX,
        session_maintenance::TARGET_WAIT_REASON
    );
    retain_failed_dispatch(
        &hub,
        "s".into(),
        "p".into(),
        vec![],
        Some("c1".into()),
        &not_ready,
    );
    assert!(hub.setting(&key).is_some());
    retain_failed_dispatch(
        &hub,
        "s".into(),
        "p".into(),
        vec![],
        Some("c1".into()),
        "Execution target unavailable or identity changed; retained prompt was not delivered",
    );
    assert!(hub.setting(&key).is_none());
    assert_eq!(hub.session_info("s").unwrap().queue_count, 1);
    // A transient disconnect alone keeps relying on the reconnect edge.
    retain_failed_dispatch(
        &hub,
        "s".into(),
        "p".into(),
        vec![],
        Some("c1".into()),
        "Machine is not connected",
    );
    assert!(hub.setting(&key).is_none());
}

#[test]
fn an_explicit_send_keeps_its_intent_across_a_transient_machine_wait() {
    let hub = Hub::new();
    hub.create_local_session(
        "s".into(),
        "claude-code".into(),
        "/tmp".into(),
        "test".into(),
        crate::core::SessionOrigin::Web,
        false,
    );
    let key = format!(
        "{}s",
        crate::core::settings_keys::SESSION_RESOURCE_WAIT_PREFIX
    );
    // A rebooting runtime is a transient wait for an explicit send...
    retain_failed_dispatch(
        &hub,
        "s".into(),
        "p".into(),
        vec![],
        Some("c1".into()),
        "Machine is not connected",
    );
    retain_explicit_intent(&hub, "s", "Machine is not connected");
    assert!(hub.setting(&key).is_some());
    assert!(hub.explicit_wait("s"));
    assert_eq!(hub.session_info("s").unwrap().queue_count, 1);
    // ...a resource wait keeps it too...
    let busy = format!(
        "{}{}",
        session_maintenance::WAIT_PREFIX,
        session_maintenance::MACHINE_CONNECTION_WAIT
    );
    hub.set_explicit_wait("s", false);
    retain_failed_dispatch(
        &hub,
        "s".into(),
        "p".into(),
        vec![],
        Some("c1".into()),
        &busy,
    );
    retain_explicit_intent(&hub, "s", &busy);
    assert!(hub.explicit_wait("s"));
    // ...and a permanent refusal ends both the wait and the intent.
    let refused =
        "Execution target unavailable or identity changed; retained prompt was not delivered";
    retain_failed_dispatch(
        &hub,
        "s".into(),
        "p".into(),
        vec![],
        Some("c1".into()),
        refused,
    );
    retain_explicit_intent(&hub, "s", refused);
    assert!(hub.setting(&key).is_none());
    assert!(!hub.explicit_wait("s"));
}

#[tokio::test]
async fn shutdown_returns_a_preflight_owned_prompt_before_writer_drain() {
    let hub = Hub::new();
    hub.create_local_session(
        "s".into(),
        "codex".into(),
        "/tmp".into(),
        "test".into(),
        crate::core::SessionOrigin::Web,
        false,
    );
    let (tx, mut requests) = mpsc::channel(4);
    hub.set_dispatch_tx(tx);
    hub.set_status("s", Status::Running, None);
    hub.submit(
        "s",
        "never delivered".into(),
        vec![],
        Some("original-cmid".into()),
    );
    let request = requests.recv().await.unwrap();
    let (shutdown, mut stopping) = watch::channel(false);
    let observed = Arc::new(tokio::sync::Notify::new());
    let started = Arc::clone(&observed);
    let owned_hub = hub.clone();
    let task = tokio::spawn(async move {
        let preparation = async {
            started.notify_one();
            std::future::pending::<Result<(), String>>().await
        };
        let error = dispatch_preflight(&mut stopping, preparation)
            .await
            .unwrap_err();
        retain_failed_dispatch(
            &owned_hub,
            request.session_id.clone(),
            request.text.clone(),
            request.content.clone(),
            request.cmid.clone(),
            &error,
        );
    });
    observed.notified().await;
    shutdown.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("s") else {
        panic!("retained queue missing")
    };
    assert_eq!(value["queue"][0]["text"], "never delivered");
    assert_eq!(value["queue"][0]["cmid"], "original-cmid");
    assert!(!hub.session_has_in_flight_prompt("s"));
}

#[test]
fn failed_remote_dispatch_retains_the_original_prompt() {
    let hub = Hub::new();
    hub.create_local_session(
        "remote-session".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "Remote session".to_owned(),
        crate::core::SessionOrigin::Web,
        false,
    );

    retain_failed_dispatch(
        &hub,
        "remote-session".to_owned(),
        "current status?".to_owned(),
        Vec::new(),
        Some("client-message-1".to_owned()),
        "machine is not connected",
    );

    assert_eq!(hub.session_info("remote-session").unwrap().queue_count, 1);
    let Some(Outbound::SyncPatch { value, .. }) = hub.queue_resync("remote-session") else {
        panic!("queued prompt should be available to reconnecting clients");
    };
    assert_eq!(value["queue"][0]["text"], "current status?");
    assert_eq!(value["queue"][0]["cmid"], "client-message-1");
}

#[test]
fn restore_timeout_hold_does_not_look_like_a_send_failure() {
    let hub = Hub::new();
    hub.create_local_session(
        "restore-session".to_owned(),
        "codex".to_owned(),
        "/tmp".to_owned(),
        "Restore session".to_owned(),
        crate::core::SessionOrigin::Web,
        false,
    );

    retain_failed_dispatch(
        &hub,
        "restore-session".to_owned(),
        "再上传 tempfiles 吧".to_owned(),
        Vec::new(),
        Some("client-message-hold".to_owned()),
        crate::provider_behavior::NATIVE_RESTORE_TIMEOUT_HOLD,
    );

    assert_eq!(hub.session_info("restore-session").unwrap().queue_count, 1);
}

#[test]
fn controller_restart_disconnect_is_transient() {
    assert!(is_transient_machine_disconnect(
        r#"machine "hawk" is not connected"#
    ));
    assert!(is_transient_machine_disconnect(
        r#"machine "hawk" disconnected"#
    ));
    assert!(!is_transient_machine_disconnect(
        "agent subprocess exited mid-session"
    ));
}
