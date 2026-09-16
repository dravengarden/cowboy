use super::*;

fn fixture() -> (Request, Value) {
    let wire: Value = serde_json::from_str(include_str!(
        "../../../plugins/zed/adapter/fixtures/content.json"
    ))
    .unwrap();
    let request = serde_json::from_value(wire["request"].clone()).unwrap();
    let reply = json!({"type":"bufferLeaseRead", "api_version":1, "lease":"original",
        "opened_version":wire["response"]["openedVersion"], "result":wire["response"]["result"]});
    (request, reply)
}

#[test]
fn content_requests_and_replies_are_closed_bounded_and_correlated() {
    let (request, value) = fixture();
    assert_eq!(
        serde_json::to_value(parse(&value, request.clone()).unwrap()).unwrap(),
        value
    );
    for (pointer, replacement) in [
        ("/result/content/sha256", json!("f".repeat(64))),
        ("/result/content/utf8Bytes", json!(8)),
        ("/result/result/kind", json!("observed")),
        ("/result/result/contents/0/markdown", json!("yes")),
        ("/result/result/contents/0/text", json!("x".repeat(65_537))),
        (
            "/result/result/contents",
            json!(vec![value["result"]["result"]["contents"][0].clone(); 33]),
        ),
    ] {
        let mut wrong = value.clone();
        *wrong.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            parse(&wrong, request.clone()).is_err(),
            "accepted {pointer}"
        );
    }
    for pointer in [
        "/result",
        "/result/content",
        "/result/result",
        "/result/result/contents/0",
    ] {
        let mut wrong = value.clone();
        wrong
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(true));
        assert!(parse(&wrong, request.clone()).is_err());
    }
    let mut mismatch = value.clone();
    mismatch["result"]["result"] = json!({"kind":"mismatch"});
    assert!(parse(&mismatch, request.clone()).is_ok());
    mismatch["result"]["result"]["contents"] = json!([]);
    assert!(parse(&mismatch, request.clone()).is_err());
    for (field, value) in [
        ("sha256", json!("A".repeat(64))),
        ("sha256", json!("0".repeat(63))),
        ("utf8Bytes", json!(4_194_305)),
    ] {
        let mut wrong = serde_json::to_value(&request).unwrap();
        wrong["content"][field] = value;
        assert!(
            serde_json::from_value::<Request>(wrong)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    for pointer in ["", "/content", "/query", "/query/position"] {
        let mut wrong = serde_json::to_value(&request).unwrap();
        wrong
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("path".into(), json!("replacement"));
        assert!(serde_json::from_value::<Request>(wrong).is_err());
    }
}

#[test]
fn content_observations_reuse_exact_diagnostic_and_symbol_validation() {
    let (request, mut response) = fixture();
    for kind in ["language", "symbols"] {
        let mut wire = serde_json::to_value(&request).unwrap();
        wire["query"] = json!({"kind":kind});
        let request: Request = serde_json::from_value(wire).unwrap();
        let observation = if kind == "language" {
            language()["result"].clone()
        } else {
            json!({"kind":"symbols","symbols":[symbol()]})
        };
        response["result"]["result"] = json!({"kind":"observed","observation":observation});
        assert!(parse(&response, request.clone()).is_ok());
        response["result"]["result"]["observation"]["extra"] = json!(true);
        assert!(parse(&response, request.clone()).is_err());
        response["result"]["result"]["observation"] = json!({"kind":"content","content":response["result"]["content"],"result":{"kind":"mismatch"}});
        assert!(parse(&response, request).is_err());
    }
}
