use super::*;

#[tokio::test]
async fn actual_navigation_handlers_match_browser_wire_without_native_references() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../../../../contracts/code-buffer-navigation.fixture.json"
    ))
    .unwrap();
    let mut fixture = fixture(21);
    let resource = opened(&fixture);
    let prepared = tokio::spawn(prepare(
        AxumState(fixture.context.clone()),
        Path(resource.clone()),
        Extension(authenticated()),
        HeaderMap::new(),
        Json(Prepare {
            content: serde_json::from_value(golden["content"].clone()).unwrap(),
            position: serde_json::from_value(golden["position"].clone()).unwrap(),
            query: Kind::Definition,
        }),
    ));
    let sent = command(&mut fixture).await;
    reply(
        &fixture,
        sent,
        serde_json::to_value(observed(Phase::Prepared)).unwrap(),
    );
    let response = prepared.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let value = json_response(response).await;
    let id = value["navigationId"].as_str().unwrap().to_owned();
    let normalize = |mut value: Value| {
        assert_eq!(value["sourceResourceId"], resource);
        assert_eq!(value["navigationId"], id);
        value["sourceResourceId"] = golden["sourceResourceId"].clone();
        value["navigationId"] = golden["navigationId"].clone();
        value
    };
    let mut expected = golden.clone();
    expected["state"] = json!("prepared");
    expected["locations"] = json!([]);
    assert_eq!(normalize(value), expected);

    for (action, phase, state) in [
        (Action::Execute, Phase::Retained, "retained"),
        (Action::Query, Phase::Retained, "retained"),
        (Action::Release, Phase::Released, "released"),
    ] {
        let task = start(&fixture, &id, action);
        let sent = command(&mut fixture).await;
        let mut native = observed(phase);
        native.locations = serde_json::from_value(golden["locations"].clone()).unwrap();
        reply(&fixture, sent, serde_json::to_value(native).unwrap());
        let response = task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let mut expected = golden.clone();
        expected["state"] = json!(state);
        assert_eq!(normalize(json_response(response).await), expected);
    }
}
