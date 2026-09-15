use super::*;
use serde_json::json;

fn language() -> Value {
    json!({"type":"bufferLeaseRead", "api_version":1, "lease":"original",
        "opened_version":[{"replicaId":0,"timestamp":1}],
        "result":{"kind":"language", "diagnosticsState":"observed", "diagnostics":[{
            "start":{"row":0,"column":0}, "end":{"row":0,"column":3},
            "severity":1,"source":"fixture","message":"a diagnostic"}],
            "inlayHints":[{"offset":3,"label":"type","kind":null,"paddingLeft":false,"paddingRight":true}],
            "semanticTokens":[0,0,3,0,0]}})
}

fn symbol() -> Value {
    json!({"name":"function", "kind":12, "start":{"row":0,"column":0},
        "end":{"row":2,"column":1}, "selectionStart":{"row":0,"column":3},
        "selectionEnd":{"row":0,"column":11}, "children":[]})
}

fn parse(value: &Value, request: Request) -> Result<Reply<String>> {
    Reply::parse(value, &"original".to_owned(), request)
}

#[test]
fn observation_codec_is_closed_and_does_not_accept_positions_or_authority() {
    for kind in ["language", "symbols"] {
        assert!(serde_json::from_value::<Request>(json!({"kind":kind})).is_ok());
        for field in ["lease", "path", "version", "resourceId", "machineId"] {
            assert!(
                serde_json::from_value::<Request>(json!({"kind":kind, field:"other"})).is_err()
            );
        }
    }
    for request in [
        json!({"kind":"hover","offset":0}),
        json!({"kind":"navigate"}),
        json!({}),
    ] {
        assert!(serde_json::from_value::<Request>(request).is_err());
    }
    let value = language();
    let parsed = parse(&value, Request::Language {}).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    assert!(parse(&value, Request::Symbols {}).is_err());
    for (pointer, replacement) in [
        ("/type", json!("bufferLanguage")),
        ("/api_version", json!(2)),
        ("/result/diagnosticsState", json!("unobserved")),
        ("/result/diagnosticsState", json!("invented")),
        ("/lease", json!("replacement")),
        ("/result/diagnostics/0/end/column", json!(-1)),
        ("/result/inlayHints/0/offset", json!(u64::MAX)),
        ("/result/semanticTokens", json!([1])),
        (
            "/opened_version",
            json!([{"replicaId":0,"timestamp":1},{"replicaId":0,"timestamp":2}]),
        ),
    ] {
        let mut wrong = value.clone();
        *wrong.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            parse(&wrong, Request::Language {}).is_err(),
            "accepted {pointer}"
        );
    }
    for pointer in [
        "",
        "/result",
        "/opened_version/0",
        "/result/diagnostics/0",
        "/result/diagnostics/0/start",
        "/result/inlayHints/0",
    ] {
        let mut wrong = value.clone();
        wrong
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(true));
        assert!(
            parse(&wrong, Request::Language {}).is_err(),
            "accepted extra at {pointer}"
        );
    }
}

#[test]
fn reply_limits_bound_text_arrays_and_nested_symbols() {
    let mut value = language();
    value["result"]["diagnostics"][0]["message"] = json!("x".repeat(64 * 1024 + 1));
    assert!(parse(&value, Request::Language {}).is_err());
    value["result"]["diagnostics"] =
        json!(vec![language()["result"]["diagnostics"][0].clone(); 1_001]);
    assert!(parse(&value, Request::Language {}).is_err());
    value["result"] = json!({"kind":"symbols","symbols":[symbol()]});
    assert!(parse(&value, Request::Symbols {}).is_ok());
    value["result"]["symbols"][0]["extra"] = json!(true);
    assert!(parse(&value, Request::Symbols {}).is_err());
    let mut deep = symbol();
    for _ in 0..16 {
        let mut parent = symbol();
        parent["children"] = json!([deep]);
        deep = parent;
    }
    value["result"]["symbols"] = json!([deep]);
    assert!(parse(&value, Request::Symbols {}).is_err());
    value["result"]["symbols"] = json!(vec![symbol(); 2_001]);
    assert!(parse(&value, Request::Symbols {}).is_err());
    value = language();
    value["result"]["diagnostics"][0]["message"] = json!("x".repeat(MAX_REPLY_BYTES));
    assert!(parse(&value, Request::Language {}).is_err());
}
