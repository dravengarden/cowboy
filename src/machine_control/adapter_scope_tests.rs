use super::*;

#[tokio::test]
async fn a_foreign_registry_token_cannot_enqueue_or_create_a_waiter() {
    let first = MachineControl::default();
    let second = MachineControl::default();
    let (sender, mut first_commands) = mpsc::unbounded_channel();
    let foreign = first.install("machine".into(), "epoch".into(), false, 19, sender);
    let (sender, mut second_commands) = mpsc::unbounded_channel();
    second.install("machine".into(), "epoch".into(), false, 19, sender);
    assert_eq!(
        second
            .adapter_request_on_connection(&foreign, "zed", serde_json::json!({"type":"health"}))
            .await
            .unwrap_err(),
        "Machine operation connection is no longer current"
    );
    assert!(first_commands.try_recv().is_err());
    assert!(second_commands.try_recv().is_err());
    assert!(first.live.read().pending.is_empty());
    assert!(second.live.read().pending.is_empty());
}

#[tokio::test]
async fn cancelled_bound_adapter_wait_removes_only_its_own_correlation() {
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let connection = control.install("machine".into(), "epoch".into(), false, 19, sender);
    let payload = serde_json::json!({"type":"health"});
    let mut retained =
        std::pin::pin!(control.adapter_request_on_connection(&connection, "zed", payload.clone()));
    assert!(futures::poll!(&mut retained).is_pending());
    let MachineCommand::AdapterRequest {
        request_id: retained_id,
        ..
    } = commands.try_recv().unwrap()
    else {
        panic!("adapter request expected")
    };
    let cancelled_id = {
        let mut cancelled = std::pin::pin!(control.adapter_request_on_connection(
            &connection,
            "zed",
            payload.clone()
        ));
        assert!(futures::poll!(&mut cancelled).is_pending());
        let MachineCommand::AdapterRequest { request_id, .. } = commands.try_recv().unwrap() else {
            panic!("adapter request expected")
        };
        assert_eq!(control.live.read().pending.len(), 2);
        request_id
    };
    assert_eq!(control.live.read().pending.len(), 1);
    assert!(control.live.read().pending.contains_key(&retained_id));
    for request_id in [cancelled_id, retained_id] {
        control.record_remote(
            &connection,
            MachineEvent::AdapterResponse {
                request_id,
                accepted: true,
                payload: Some(payload.clone()),
                detail: None,
                refusal: None,
            },
        );
    }
    assert_eq!(retained.await.unwrap(), payload);
    assert!(control.live.read().pending.is_empty());
}

#[tokio::test(start_paused = true)]
async fn bound_adapter_timeout_removes_the_waiter_without_followup() {
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let connection = control.install("machine".into(), "epoch".into(), false, 19, sender);
    let mut request = std::pin::pin!(control.adapter_request_on_connection(
        &connection,
        "zed",
        serde_json::json!({"type":"health"})
    ));
    assert!(futures::poll!(&mut request).is_pending());
    commands.try_recv().unwrap();
    assert_eq!(control.live.read().pending.len(), 1);
    tokio::time::advance(DEFAULT_ADAPTER_TIMEOUT + std::time::Duration::from_secs(1)).await;
    assert_eq!(
        request.await.unwrap_err(),
        "Machine adapter request timed out"
    );
    assert!(commands.try_recv().is_err());
    assert!(control.live.read().pending.is_empty());
}

#[tokio::test]
async fn disconnect_after_completion_still_rejects_the_parked_reply() {
    let control = MachineControl::default();
    let (sender, mut commands) = mpsc::unbounded_channel();
    let connection = control.install("machine".into(), "epoch".into(), false, 19, sender);
    let mut request = std::pin::pin!(control.adapter_request_on_connection(
        &connection,
        "zed",
        serde_json::json!({"type":"health"})
    ));
    assert!(futures::poll!(&mut request).is_pending());
    let MachineCommand::AdapterRequest { request_id, .. } = commands.try_recv().unwrap() else {
        panic!("adapter request expected")
    };
    control.record_remote(
        &connection,
        MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(serde_json::json!({"type":"health"})),
            detail: None,
            refusal: None,
        },
    );
    control.remove_if_current(&connection);
    assert_eq!(
        request.await.unwrap_err(),
        "Machine operation connection is no longer current"
    );
    assert!(control.live.read().pending.is_empty());
}
