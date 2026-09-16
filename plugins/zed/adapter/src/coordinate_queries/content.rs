use super::*;
use crate::content_reads::{Content, Output, Query};

fn contract() -> serde_json::Value {
    serde_json::from_str(include_str!("../../fixtures/content.json")).unwrap()
}

fn input() -> (Content, Query) {
    let value = contract();
    (
        serde_json::from_value(value["request"]["content"].clone()).unwrap(),
        serde_json::from_value(value["request"]["query"].clone()).unwrap(),
    )
}

#[tokio::test]
async fn oversized_native_hover_is_refused_not_truncated_into_success() {
    for contents in [
        vec![proto::HoverBlock::default(); 33],
        vec![proto::HoverBlock {
            text: "x".repeat(65_537),
            ..Default::default()
        }],
    ] {
        let (zed, mut outbound) = fixture().await;
        let task = {
            let zed = zed.clone();
            tokio::spawn(async move {
                let (content, query) = input();
                zed.content_read(7, &content, query).await
            })
        };
        let request = outbound.recv().await.unwrap();
        reply(
            &zed,
            request,
            vec![proto::LspResponse {
                response: Some(proto::lsp_response::Response::GetHoverResponse(
                    proto::GetHoverResponse {
                        contents,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }],
        )
        .await;
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("exceed limits")
        );
    }
}

#[tokio::test]
async fn content_hover_matches_the_shared_unicode_wire_without_path_access() {
    let (zed, mut outbound) = fixture().await;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move {
            let (content, query) = input();
            zed.content_read(7, &content, query).await
        })
    };
    let request = outbound.recv().await.unwrap();
    let query = reply(
        &zed,
        request,
        vec![proto::LspResponse {
            response: Some(proto::lsp_response::Response::GetHoverResponse(
                proto::GetHoverResponse {
                    contents: vec![proto::HoverBlock {
                        text: "Unicode fixture".into(),
                        is_markdown: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )),
            ..Default::default()
        }],
    )
    .await;
    let Some(proto::lsp_query::Request::GetHover(hover)) = query.request else {
        panic!("wrong query");
    };
    assert_eq!(hover.position.unwrap().offset, 5);
    assert_eq!(
        serde_json::to_value(task.await.unwrap().unwrap()).unwrap(),
        contract()["response"]["result"]["result"]
    );
}

#[tokio::test]
async fn mismatched_content_never_dispatches_and_original_content_stays_usable() {
    let (zed, mut outbound) = fixture().await;
    let (original, query) = input();
    for content in [
        Content {
            sha256: "f".repeat(64),
            ..original.clone()
        },
        Content {
            utf8_bytes: 6,
            ..original.clone()
        },
    ] {
        assert!(matches!(
            zed.content_read(7, &content, query).await.unwrap(),
            Output::Mismatch {}
        ));
        assert!(outbound.try_recv().is_err());
    }
    assert!(
        zed.diagnostics
            .lock()
            .unwrap()
            .match_content(7, &original)
            .unwrap()
            .is_some()
    );
    let mut invalid = original;
    invalid.sha256 = "F".repeat(64);
    assert!(zed.content_read(7, &invalid, query).await.is_err());
    assert!(outbound.try_recv().is_err());
}

#[tokio::test]
async fn content_reads_reject_edit_undo_aba_for_all_three_operations() {
    for query in [Query::Language {}, Query::Symbols {}, input().1] {
        let (zed, mut outbound) = fixture().await;
        let task = {
            let zed = zed.clone();
            tokio::spawn(async move { zed.content_read(7, &input().0, query).await })
        };
        let mut requests = vec![outbound.recv().await.unwrap()];
        if matches!(query, Query::Language {}) {
            requests.push(outbound.recv().await.unwrap());
            requests.push(outbound.recv().await.unwrap());
        }
        let mut source = peer("a🙂z\n", 1);
        let operation = source.edit([(0..0, "changed")]);
        source.finalize_last_transaction();
        let undo = source.undo().unwrap().1;
        zed.diagnostics
            .lock()
            .unwrap()
            .observe(&proto::envelope::Payload::UpdateBuffer(
                proto::UpdateBuffer {
                    buffer_id: 7,
                    operations: vec![wire(&operation), wire(&undo)],
                    ..Default::default()
                },
            ));
        assert!(
            zed.diagnostics
                .lock()
                .unwrap()
                .match_content(7, &input().0)
                .unwrap()
                .is_some()
        );
        for request in requests {
            reply(&zed, request, Vec::new()).await;
        }
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("changed during read")
        );
    }
}

#[tokio::test]
async fn content_positions_refuse_surrogate_splits_and_missing_native_history() {
    let (zed, mut outbound) = fixture().await;
    let (content, _) = input();
    for position in [
        serde_json::json!({"row":0,"column":2}),
        serde_json::json!({"row":2,"column":0}),
    ] {
        let query = serde_json::from_value(serde_json::json!({"kind":"hover","position":position}))
            .unwrap();
        assert!(zed.content_read(7, &content, query).await.is_err());
    }
    zed.diagnostics.lock().unwrap().remove(7);
    assert!(
        zed.content_read(7, &content, Query::Symbols {})
            .await
            .is_err()
    );
    assert!(outbound.try_recv().is_err());
}
