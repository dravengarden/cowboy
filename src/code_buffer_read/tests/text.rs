use super::*;

fn fixture() -> (Value, Value) {
    let wire: Value = serde_json::from_str(include_str!(
        "../../../plugins/zed/adapter/fixtures/text.json"
    ))
    .unwrap();
    (
        wire["request"].clone(),
        json!({"type":"bufferLeaseRead","api_version":1,"lease":"original","opened_version":wire["response"]["openedVersion"],"result":wire["response"]["result"]}),
    )
}

#[test]
fn text_pages_are_closed_and_match_original_content_offset_and_snapshot() {
    let (request, reply) = fixture();
    let decode = |value: &Value| parse(value, serde_json::from_value(request.clone()).unwrap());
    assert_eq!(
        serde_json::to_value(decode(&reply).unwrap()).unwrap(),
        reply
    );
    let mut missing = reply.clone();
    missing["result"]["result"]
        .as_object_mut()
        .unwrap()
        .remove("nextOffset");
    assert!(decode(&missing).is_err());
    for (pointer, replacement) in [
        ("/result/content/sha256", json!("f".repeat(64))),
        ("/result/result/snapshot", json!("A".repeat(64))),
        ("/result/result/offset", json!(1)),
        ("/result/result/text", json!("b🙂z\n")),
        ("/result/result/text", json!("a🙂z\r")),
        ("/result/result/nextOffset", json!(7)),
        ("/result/result/kind", json!("stale")),
    ] {
        let mut wrong = reply.clone();
        *wrong.pointer_mut(pointer).unwrap() = replacement;
        assert!(decode(&wrong).is_err(), "accepted {pointer}");
    }
    for pointer in ["/result", "/result/result", "/result/content"] {
        let mut wrong = reply.clone();
        wrong
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("path".into(), json!("replacement"));
        assert!(decode(&wrong).is_err());
    }
    for pointer in ["", "/content", "/page"] {
        let mut wrong = request.clone();
        wrong
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("path".into(), json!("replacement"));
        assert!(serde_json::from_value::<Request>(wrong).is_err());
    }
    for offset in [0, 7, u32::MAX] {
        let mut wrong = request.clone();
        wrong["page"] = json!({"kind":"continue","offset":offset,"snapshot":"a".repeat(64)});
        assert!(
            serde_json::from_value::<Request>(wrong)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut value = reply;
    value["result"]["result"] = json!({"kind":"mismatch"});
    assert!(decode(&value).is_ok());
    value["result"]["result"] = json!({"kind":"stale"});
    assert!(decode(&value).is_err());
}

#[test]
fn fixed_page_bounds_refuse_truncation_stalls_and_snapshot_substitution() {
    let (mut request, mut value) = fixture();
    request["content"]["utf8Bytes"] = json!(131_072);
    value["result"]["content"] = request["content"].clone();
    value["result"]["result"]["text"] = json!("x".repeat(65_536));
    value["result"]["result"]["nextOffset"] = json!(65_536);
    let decode = |value: &Value, request: &Value| {
        parse(value, serde_json::from_value(request.clone()).unwrap())
    };
    assert!(decode(&value, &request).is_ok());
    for size in [0, 1, 65_532, 65_537] {
        let mut wrong = value.clone();
        wrong["result"]["result"]["text"] = json!("x".repeat(size));
        wrong["result"]["result"]["nextOffset"] = json!(size);
        assert!(decode(&wrong, &request).is_err());
    }
    value["result"]["result"]["nextOffset"] = Value::Null;
    assert!(decode(&value, &request).is_err());
    request["page"] = json!({"kind":"continue","offset":65_536,"snapshot":"a".repeat(64)});
    value["result"]["result"]["offset"] = json!(65_536);
    assert!(decode(&value, &request).is_ok());
    value["result"]["result"]["snapshot"] = json!("b".repeat(64));
    assert!(decode(&value, &request).is_err());
    value["result"]["result"] = json!({"kind":"stale"});
    assert!(decode(&value, &request).is_ok());
}
