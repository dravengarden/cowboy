use super::*;

#[tokio::test]
async fn actual_destination_handoff_matches_browser_wire_and_retains_an_independent_owner() {
    let golden: Value = serde_json::from_str(include_str!(
        "../../../../../contracts/code-buffer-destination.fixture.json"
    ))
    .unwrap();
    let mut fixture = fixture(21);
    let resource = opened(&fixture);
    let preparing = tokio::spawn(prepare(
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
    let response = preparing.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let prepared = json_response(response).await;
    let id = prepared["navigationId"].as_str().unwrap();
    let mut snapshot = observed(Phase::Retained);
    snapshot.locations = serde_json::from_value(golden["locations"].clone()).unwrap();
    let execute = start(&fixture, id, Action::Execute);
    let sent = command(&mut fixture).await;
    reply(&fixture, sent, serde_json::to_value(&snapshot).unwrap());
    assert_eq!(execute.await.unwrap().status(), StatusCode::OK);

    let handoff = tokio::spawn(destination(
        AxumState(fixture.context.clone()),
        Path(id.into()),
        Extension(authenticated()),
        HeaderMap::new(),
        Json(Destination {
            destination: 0,
            content: snapshot.locations[0].content.clone(),
        }),
    ));
    let sent = command(&mut fixture).await;
    snapshot.destinations.push(
        crate::machine_protocol::code_buffer_navigation::Destination {
            destination: 0,
            lease: serde_json::from_value(native(2)).unwrap(),
        },
    );
    reply(&fixture, sent, serde_json::to_value(&snapshot).unwrap());
    let response = handoff.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let mut value = json_response(response).await;
    let target = value["destinations"][0]["resourceId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(target, resource);
    assert_eq!(value["sourceResourceId"], resource);
    assert_eq!(value["navigationId"], id);
    value["sourceResourceId"] = golden["sourceResourceId"].clone();
    value["navigationId"] = golden["navigationId"].clone();
    value["destinations"][0]["resourceId"] = golden["destinations"][0]["resourceId"].clone();
    assert_eq!(value, golden);
    let owners = &fixture.context.code_buffers;
    assert!(owners.admit_read("local", &target).is_err()); // handoff does not Open
    let crate::server::code_buffers::registry::Admission::Run(job) = owners
        .admit("local", &target, remote::Action::Open)
        .unwrap()
    else {
        panic!("independent ordinary Open");
    };
    job.begin().unwrap();
    job.finish(remote::LeaseState::Open).unwrap();
    let release = start(&fixture, id, Action::Release);
    let sent = command(&mut fixture).await;
    snapshot.phase = Phase::Released;
    reply(&fixture, sent, serde_json::to_value(snapshot).unwrap());
    assert_eq!(release.await.unwrap().status(), StatusCode::OK);
    assert!(
        fixture
            .context
            .code_buffers
            .admit_read("local", &target)
            .is_ok()
    );
}

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
