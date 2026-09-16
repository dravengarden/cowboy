use super::*;
use serde_json::{Value, json};

fn request() -> Value {
    json!({"service_id":format!("svc-{}", "a".repeat(32)), "machine_id":"hawk", "action":{
        "kind":"prepare", "lease":{"instance":"b".repeat(32),"id":"0000000000000001"},
        "purpose":"refresh_from_disk", "content":{"sha256":"c".repeat(64),"utf8Bytes":4}
    }})
}

#[test]
fn closed_declarations_are_not_generic_adapter_payloads_or_caller_grants() {
    let valid = request();
    let decoded: Request = serde_json::from_value(valid.clone()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), valid);
    for (field, value) in [
        ("worktree", json!("/replacement")),
        ("path", json!("file")),
        ("authorized", json!(true)),
        ("version", json!([])),
        ("timeout", json!(999_999)),
    ] {
        let mut changed = valid.clone();
        changed["action"][field] = value;
        assert!(serde_json::from_value::<Request>(changed).is_err());
    }
    for kind in ["reload", "restore", "write", "refresh_from_disk"] {
        let mut changed = valid.clone();
        changed["action"]["kind"] = json!(kind);
        assert!(serde_json::from_value::<Request>(changed).is_err());
    }
    let command = super::super::MachineCommand::CodeBufferSync {
        request_id: "fixture".into(),
        request: Box::new(decoded),
    };
    assert_eq!(command.minimum_protocol(), 20);
}

#[test]
fn content_site_and_references_are_bounded_before_execution() {
    for (pointer, value) in [
        ("/service_id", json!("other")),
        ("/machine_id", json!("")),
        ("/machine_id", json!("x".repeat(257))),
        ("/action/lease/id", json!("0000000000000000")),
        ("/action/lease/instance", json!("F".repeat(32))),
        ("/action/content/sha256", json!("F".repeat(64))),
        ("/action/content/utf8Bytes", json!(4_194_305)),
    ] {
        let mut changed = request();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<Request>(changed)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}

#[test]
fn applied_evidence_must_repeat_exact_content_and_a_canonical_native_clock() {
    let request: Request = serde_json::from_value(request()).unwrap();
    let Action::Prepare { content, .. } = request.action else {
        unreachable!()
    };
    let mut state =
        json!({"kind":"applied","content":content,"version":[{"replicaId":0,"timestamp":1}]});
    serde_json::from_value::<State>(state.clone())
        .unwrap()
        .validate(&content)
        .unwrap();
    for version in [
        json!([{"replicaId":0,"timestamp":0}]),
        json!([{"replicaId":0,"timestamp":1},{"replicaId":0,"timestamp":2}]),
        json!([{"replicaId":2,"timestamp":1},{"replicaId":1,"timestamp":2}]),
    ] {
        state["version"] = version;
        assert!(
            serde_json::from_value::<State>(state.clone())
                .unwrap()
                .validate(&content)
                .is_err()
        );
    }
    state["version"] = json!([]);
    state["content"]["sha256"] = json!("d".repeat(64));
    assert!(
        serde_json::from_value::<State>(state)
            .unwrap()
            .validate(&content)
            .is_err()
    );
}

#[test]
fn effect_free_and_unknown_states_cannot_smuggle_extra_evidence() {
    for kind in ["prepared", "pending", "unknown", "retired"] {
        assert!(serde_json::from_value::<State>(json!({"kind":kind})).is_ok());
        for field in ["applied", "content", "version", "authorized"] {
            assert!(serde_json::from_value::<State>(json!({"kind":kind,field:true})).is_err());
        }
    }
}
