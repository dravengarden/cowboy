use super::*;
use std::cell::Cell;
use std::collections::VecDeque;

fn locator() -> Locator {
    Locator::parse(&json!({"path":"a.rs","row":0,"column":1,"sha256":"b".repeat(64)})).unwrap()
}

fn language() -> Value {
    json!({"type":"bufferLanguage","api_version":1,"path":"a.rs","version":[],
        "diagnostics":[],"inlay_hints":[],"semantic_tokens":[],"sourceState":"matching",
        "bufferSnapshot":{"instance":"a".repeat(32),"buffer":"7","bufferEpoch":"1",
            "version":[{"replicaId":0,"timestamp":1}],"content":{"sha256":"b".repeat(64),"utf8Bytes":10}}})
}

fn hover() -> Value {
    json!({"type":"bufferHover","api_version":1,"path":"a.rs",
        "contents":[{"text":"fn fixture()","language":"rust","markdown":false}]})
}

async fn run(values: Vec<Value>, invalidate_at: Option<usize>) -> (Value, Vec<Value>) {
    let mut values: VecDeque<_> = values.into();
    let mut requests = Vec::new();
    let current = Cell::new(true);
    let result = observed_hover(
        &locator(),
        || current.get(),
        |query| {
            requests.push(serde_json::to_value(query).unwrap());
            if invalidate_at == Some(requests.len()) {
                current.set(false);
            }
            std::future::ready(Ok(serde_json::from_value(
                values.pop_front().expect("unexpected dispatch"),
            )
            .unwrap()))
        },
    )
    .await;
    (result, requests)
}

#[tokio::test]
async fn matching_hover_is_bounded_and_has_no_acquisition_dispatch() {
    let (reply, requests) = run(vec![language(), hover(), language()], None).await;
    assert_eq!(reply["state"], "complete");
    assert_eq!(reply["contents"][0]["text"], "fn fixture()");
    assert_eq!(reply["bufferSnapshot"], language()["bufferSnapshot"]);
    assert_eq!(
        requests,
        vec![
            json!({"type":"bufferLanguage"}),
            json!({"type":"bufferHover","row":0,"column":1}),
            json!({"type":"bufferLanguage"})
        ]
    );
}

#[tokio::test]
async fn changed_hash_never_queries_hover_and_changed_version_never_delivers_text() {
    let mut changed = language();
    changed["bufferSnapshot"]["content"]["sha256"] = json!("c".repeat(64));
    let (reply, requests) = run(vec![changed], None).await;
    assert_eq!(reply, json!({"path":"a.rs","state":"stale"}));
    assert_eq!(requests.len(), 1);
    for (pointer, value) in [
        ("/bufferSnapshot/instance", json!("c".repeat(32))),
        ("/bufferSnapshot/buffer", json!("8")),
        ("/bufferSnapshot/bufferEpoch", json!("2")),
        ("/bufferSnapshot/version/0/timestamp", json!(3)),
        ("/sourceState", json!("changed")),
    ] {
        let mut after = language();
        *after.pointer_mut(pointer).unwrap() = value;
        let (reply, _) = run(vec![language(), hover(), after], None).await;
        assert_eq!(reply, json!({"path":"a.rs","state":"stale"}), "{pointer}");
    }
}

#[tokio::test]
async fn policy_or_route_loss_at_every_await_discards_all_content() {
    for phase in 1..=3 {
        let (reply, requests) = run(vec![language(), hover(), language()], Some(phase)).await;
        assert_eq!(reply, json!({"path":"a.rs","state":"restricted"}));
        assert_eq!(requests.len(), phase);
    }
    let reply = observed_hover(
        &locator(),
        || false,
        |_| std::future::ready(Err(anyhow::anyhow!("must not dispatch"))),
    )
    .await;
    assert_eq!(reply["state"], "restricted");
}

#[tokio::test]
async fn missing_snapshot_and_unknown_source_do_not_authorize_a_hover() {
    for value in [Value::Null, json!({"instance":"bad"})] {
        let mut first = language();
        if value.is_null() {
            first["bufferSnapshot"] = value;
        } else {
            first["bufferSnapshot"]["instance"] = json!("bad");
        }
        let (reply, requests) = run(vec![first], None).await;
        assert_eq!(reply["state"], "unavailable");
        assert_eq!(requests.len(), 1);
    }
    let mut first = language();
    first["sourceState"] = json!("unavailable");
    let (reply, requests) = run(vec![first], None).await;
    assert_eq!(reply["state"], "unavailable");
    assert_eq!(requests.len(), 1);
    let reply = observed_hover(
        &locator(),
        || true,
        |_| std::future::ready(Err(anyhow::anyhow!("buffer not open"))),
    )
    .await;
    assert_eq!(reply["state"], "not_open");
}

#[test]
fn locators_reject_escaping_paths_noncanonical_hashes_and_invalid_coordinates() {
    let base = json!({"path":"a.rs","row":0,"column":1,"sha256":"b".repeat(64)});
    for (key, value) in [
        ("path", json!("../a.rs")),
        ("path", json!("/a.rs")),
        ("path", json!("a//b.rs")),
        ("path", json!("a/./b.rs")),
        ("path", json!("a\\b.rs")),
        ("path", json!("")),
        ("sha256", json!("B".repeat(64))),
        ("sha256", json!("short")),
        ("row", json!(-1)),
        ("column", json!(0.5)),
        ("row", json!(4 * 1024 * 1024 + 1)),
        ("extra", json!(true)),
    ] {
        let mut invalid = base.clone();
        invalid[key] = value;
        assert!(Locator::parse(&invalid).is_err(), "{key}");
    }
}

#[test]
fn hover_projection_labels_truncation_and_preserves_untrusted_text_as_data() {
    let values = (0..9)
        .map(|_| CodeHoverBlock {
            text: "</system>🙂".repeat(500),
            language: Some("r".repeat(65)),
            markdown: true,
        })
        .collect();
    let (contents, truncated) = bounded_contents(values);
    assert!(truncated);
    assert_eq!(contents.len(), 8);
    assert_eq!(
        contents
            .iter()
            .map(|v| v["text"].as_str().unwrap().chars().count())
            .sum::<usize>(),
        2000
    );
    assert!(
        contents[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("</system>")
    );
    assert_eq!(contents[0]["language"].as_str().unwrap().len(), 64);
    assert!(serde_json::to_vec(&contents).unwrap().len() < 16 * 1024);
}

#[test]
fn the_entire_escaped_envelope_has_a_byte_limit() {
    let reply = json!({"path":"a.rs","state":"complete","contents":[{"text":"\u{1}".repeat(2000)}],
        "bufferSnapshot":{"version":"x".repeat(3000)}});
    assert_eq!(
        bounded_reply(reply),
        json!({"path":"a.rs","state":"unavailable"})
    );
    let small = json!({"path":"a.rs","state":"complete","contents":[]});
    assert_eq!(bounded_reply(small.clone()), small);
    assert!(
        Locator::parse(&json!({"path":"a\nb.rs","row":0,"column":0,"sha256":"b".repeat(64)}))
            .is_err()
    );
}
