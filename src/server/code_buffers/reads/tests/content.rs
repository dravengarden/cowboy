use super::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../plugins/zed/adapter/fixtures/content.json"
    ))
    .unwrap()
}

#[tokio::test]
async fn content_handler_preserves_the_shared_wire_and_owner_without_exposing_native_references() {
    for (wire, support, mismatch) in [
        (fixture(), "bufferLeaseContentSupport", false),
        (fixture(), "bufferLeaseContentSupport", true),
        (
            serde_json::from_str(include_str!(
                "../../../../../plugins/zed/adapter/fixtures/text.json"
            ))
            .unwrap(),
            "bufferLeaseTextSupport",
            false,
        ),
        (
            serde_json::from_str(include_str!(
                "../../../../../plugins/zed/adapter/fixtures/text.json"
            ))
            .unwrap(),
            "bufferLeaseTextSupport",
            true,
        ),
    ] {
        let (mut state, id) = opened().await;
        let task = start(
            &state,
            &id,
            serde_json::from_value(wire["request"].clone()).unwrap(),
        );
        let probe = command(&mut state).await;
        assert_eq!(payload(&probe), &json!({"type":support}));
        reply(
            &state.context,
            &state.connection,
            probe,
            json!({"type":support,"api_version":1}),
        );
        let command = command(&mut state).await;
        assert_eq!(
            payload(&command),
            &json!({"type":"readBufferLease","lease":native(1),"request":wire["request"]})
        );
        let mut expected = wire["response"].clone();
        expected["resourceId"] = json!(id);
        if mismatch {
            expected["result"]["result"] = json!({"kind":"mismatch"});
        }
        reply(
            &state.context,
            &state.connection,
            command,
            json!({"type":"bufferLeaseRead","api_version":1,
            "lease":native(1),"opened_version":expected["openedVersion"],"result":expected["result"]}),
        );
        let response = task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(json_response(response).await, expected);
        assert!(state.commands.try_recv().is_err());
        assert_eq!(
            state
                .operation(&id, Action::Release, "released")
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn old_content_host_refuses_without_fallback_and_invalid_content_sends_nothing() {
    let (mut state, id) = opened().await;
    let mut wire = fixture()["request"].clone();
    let task = start(&state, &id, serde_json::from_value(wire.clone()).unwrap());
    let probe = command(&mut state).await;
    reply(
        &state.context,
        &state.connection,
        probe,
        json!({"type":"bufferLeaseReadSupport","api_version":1}),
    );
    assert_eq!(task.await.unwrap().status(), StatusCode::NOT_IMPLEMENTED);
    assert!(state.commands.try_recv().is_err());
    wire["content"]["utf8Bytes"] = json!(4_194_305);
    assert_eq!(
        start(&state, &id, serde_json::from_value(wire).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(state.commands.try_recv().is_err());
    assert_eq!(
        state
            .operation(&id, Action::Release, "released")
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn content_support_cannot_stand_in_for_native_text_support() {
    let (mut state, id) = opened().await;
    let wire: Value = serde_json::from_str(include_str!(
        "../../../../../plugins/zed/adapter/fixtures/text.json"
    ))
    .unwrap();
    let task = start(
        &state,
        &id,
        serde_json::from_value(wire["request"].clone()).unwrap(),
    );
    let probe = command(&mut state).await;
    assert_eq!(payload(&probe), &json!({"type":"bufferLeaseTextSupport"}));
    reply(
        &state.context,
        &state.connection,
        probe,
        json!({"type":"bufferLeaseContentSupport","api_version":1}),
    );
    assert_eq!(task.await.unwrap().status(), StatusCode::NOT_IMPLEMENTED);
    assert!(state.commands.try_recv().is_err());
}
