//! Private transport fixtures, not evidence of a production language server.
use super::*;
use coordinates::tests::{peer, wire};

mod content;

async fn fixture() -> (Arc<ZedRuntime>, mpsc::UnboundedReceiver<proto::Envelope>) {
    let (outbound, receiver) = mpsc::unbounded_channel();
    let mut child = Command::new("true").spawn().unwrap();
    child.wait().await.unwrap();
    let zed = Arc::new(ZedRuntime {
        _child: Mutex::new(child),
        outbound,
        pending: Arc::default(),
        sync: sync_native::Transport::new().0,
        events: broadcast::channel(16).0,
        buffer_files: Arc::default(),
        worktree_paths: Arc::default(),
        diagnostics: Arc::default(),
        next_message_id: AtomicU32::new(1),
        next_lsp_request_id: AtomicU64::new(1),
    });
    {
        let mut cache = zed.diagnostics.lock().unwrap();
        for variant in [
            proto::create_buffer_for_peer::Variant::State(proto::BufferState {
                id: 7,
                base_text: "a🙂z\n".into(),
                ..Default::default()
            }),
            proto::create_buffer_for_peer::Variant::Chunk(proto::BufferChunk {
                buffer_id: 7,
                is_last: true,
                ..Default::default()
            }),
        ] {
            cache.observe(&proto::envelope::Payload::CreateBufferForPeer(
                proto::CreateBufferForPeer {
                    variant: Some(variant),
                    ..Default::default()
                },
            ));
        }
    }
    (zed, receiver)
}

fn edit(zed: &ZedRuntime) {
    let mut source = peer("a🙂z\n", 1);
    zed.diagnostics
        .lock()
        .unwrap()
        .observe(&proto::envelope::Payload::UpdateBuffer(
            proto::UpdateBuffer {
                buffer_id: 7,
                operations: vec![wire(&source.edit([(0..0, "汉\n")]))],
                ..Default::default()
            },
        ));
}

async fn reply(
    zed: &ZedRuntime,
    envelope: proto::Envelope,
    responses: Vec<proto::LspResponse>,
) -> proto::LspQuery {
    let Some(proto::envelope::Payload::LspQuery(query)) = envelope.payload else {
        panic!("wrong query")
    };
    zed.pending
        .lock()
        .await
        .remove(&envelope.id)
        .unwrap()
        .send(proto::Envelope {
            payload: Some(proto::envelope::Payload::Ack(proto::Ack {})),
            ..Default::default()
        })
        .unwrap();
    zed.events
        .send(proto::Envelope {
            payload: Some(proto::envelope::Payload::LspQueryResponse(
                proto::LspQueryResponse {
                    lsp_request_id: query.lsp_request_id,
                    responses,
                    ..Default::default()
                },
            )),
            ..Default::default()
        })
        .unwrap();
    query
}

#[tokio::test]
async fn edited_hover_sends_current_native_insertion_and_version() {
    let (zed, mut outbound) = fixture().await;
    edit(&zed);
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.hover(7, 0, 1).await })
    };
    let envelope = outbound.recv().await.unwrap();
    let query = reply(&zed, envelope, Vec::new()).await;
    let Some(proto::lsp_query::Request::GetHover(hover)) = query.request else {
        panic!("wrong kind")
    };
    let anchor = hover.position.unwrap();
    assert_eq!((anchor.replica_id, anchor.offset), (1, 3));
    assert!(hover.version.iter().any(|entry| entry.replica_id == 1));
    assert!(task.await.unwrap().unwrap().is_empty());
}

#[tokio::test]
async fn late_edits_discard_hover_navigation_and_symbols() {
    for kind in 0..3 {
        let (zed, mut outbound) = fixture().await;
        let task = {
            let zed = zed.clone();
            tokio::spawn(async move {
                match kind {
                    0 => zed.hover(7, 0, 1).await.map(|_| ()),
                    1 => zed
                        .navigate(7, 0, 1, NavigationKind::Definition)
                        .await
                        .map(|_| ()),
                    _ => zed.document_symbols(7).await.map(|_| ()),
                }
            })
        };
        let envelope = outbound.recv().await.unwrap();
        edit(&zed);
        reply(&zed, envelope, Vec::new()).await;
        let error = task.await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("changed during read"),
            "{error:#}"
        );
        assert!(
            zed.diagnostics.lock().unwrap().revision(7).is_ok(),
            "read failure discarded owner state"
        );
    }
}

#[tokio::test]
async fn navigation_resolves_original_native_anchors_without_reading_disk() {
    let (zed, mut outbound) = fixture().await;
    edit(&zed);
    zed.worktree_paths
        .write()
        .await
        .insert(1, PathBuf::from("/nonexistent-native-coordinate-fixture"));
    zed.buffer_files.write().await.insert(
        7,
        proto::File {
            worktree_id: 1,
            path: "does-not-exist.txt".into(),
            ..Default::default()
        },
    );
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.navigate(7, 1, 1, NavigationKind::Definition).await })
    };
    let envelope = outbound.recv().await.unwrap();
    let responses = vec![proto::LspResponse {
        response: Some(proto::lsp_response::Response::GetDefinitionResponse(
            proto::GetDefinitionResponse {
                links: vec![proto::LocationLink {
                    target: Some(proto::Location {
                        buffer_id: 7,
                        start: Some(position_anchor(7, 1)),
                        end: Some(position_anchor(7, 5)),
                    }),
                    ..Default::default()
                }],
            },
        )),
        ..Default::default()
    }];
    reply(&zed, envelope, responses).await;
    let locations = task.await.unwrap().unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(
        (
            locations[0].start.row,
            locations[0].start.column,
            locations[0].end.column
        ),
        (1, 1, 3)
    );
}

#[tokio::test]
async fn language_observation_discards_late_edits_across_all_subqueries() {
    let (zed, mut outbound) = fixture().await;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.language(7).await })
    };
    let requests = [
        outbound.recv().await.unwrap(),
        outbound.recv().await.unwrap(),
        outbound.recv().await.unwrap(),
    ];
    edit(&zed);
    for request in requests {
        reply(&zed, request, Vec::new()).await;
    }
    let error = task
        .await
        .unwrap()
        .expect_err("stale language observation succeeded");
    assert!(
        error.to_string().contains("changed during read"),
        "{error:#}"
    );
}
