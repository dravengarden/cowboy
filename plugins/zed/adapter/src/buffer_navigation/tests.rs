//! Deterministic private-transport tests, not a production LSP acceptance.
use super::*;
use crate::sync_native::wire;
use crate::{Request, Worktrees, coordinate_queries, respond};
use prost::Message as _;
use std::sync::Arc;
use tokio::sync::mpsc;

mod faults;
mod handoff;

#[tokio::test]
async fn navigation_support_is_closed_and_requires_the_actual_native_pair() {
    assert!(
        serde_json::from_value::<Request>(serde_json::json!({
            "type":"bufferNavigationSupport","authorized":true
        }))
        .is_err()
    );
    assert!(
        respond(
            Request::BufferNavigationSupport {},
            &Arc::default(),
            &Arc::default(),
            None
        )
        .await
        .is_err()
    );
}

struct Fixture {
    root: PathBuf,
    worktrees: Worktrees,
    buffers: Buffers,
    zed: Zed,
    outbound: mpsc::UnboundedReceiver<proto::Envelope>,
    navigation: mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
    closed: mpsc::UnboundedReceiver<Vec<u64>>,
    lease: buffer_leases::LeaseRef,
}

impl Fixture {
    async fn new() -> Self {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).unwrap();
        let root =
            std::env::temp_dir().join(format!("cw-nav-{:032x}", u128::from_be_bytes(random)));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("source"), "a🙂z\n").unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        crate::ensure_worktree(root.clone(), true, false, &worktrees, None)
            .await
            .unwrap();
        worktrees.write().await.get_mut(&root).unwrap().state = crate::WorktreeState::Ready;
        let Response::BufferLease { lease, .. } = respond(
            Request::PrepareBuffer {
                worktree: root.clone(),
                path: "source".into(),
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap() else {
            panic!("wrong reply")
        };
        respond(
            Request::OpenBufferLease {
                lease: lease.clone(),
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        buffers
            .active
            .write()
            .await
            .get_mut(&(root.clone(), "source".into()))
            .unwrap()
            .remote_id = 7;
        let (mut zed, outbound) = coordinate_queries::fixture().await;
        let (transport, navigation, closed) = crate::native_close::tests::fixture();
        Arc::get_mut(&mut zed).unwrap().sync = transport;
        zed.worktree_paths.write().await.insert(1, root.clone());
        zed.buffer_files.write().await.insert(
            7,
            proto::File {
                worktree_id: 1,
                path: "source".into(),
                ..Default::default()
            },
        );
        Self {
            root,
            worktrees,
            buffers,
            zed,
            outbound,
            navigation,
            closed,
            lease,
        }
    }

    async fn request(&self, request: Request) -> Result<Response> {
        respond(request, &self.worktrees, &self.buffers, Some(&self.zed)).await
    }

    fn spawn(&self, request: Request) -> tokio::task::JoinHandle<Result<Response>> {
        let (worktrees, buffers, zed) = (
            self.worktrees.clone(),
            self.buffers.clone(),
            self.zed.clone(),
        );
        tokio::spawn(async move { respond(request, &worktrees, &buffers, Some(&zed)).await })
    }

    async fn prepare(&self) -> NavigationRef {
        let Response::OwnedBufferNavigation {
            navigation,
            state: State::Prepared,
            ..
        } = self
            .request(Request::PrepareBufferNavigation {
                lease: self.lease.clone(),
                content: self.content(7),
                position: Point { row: 0, column: 1 },
                kind: NavigationKind::Definition,
            })
            .await
            .unwrap()
        else {
            panic!("not prepared")
        };
        navigation
    }

    fn content(&self, id: u64) -> Content {
        self.zed.diagnostics.lock().unwrap().content(id).unwrap()
    }

    async fn target(&self, id: u64, path: &str) {
        self.zed.buffer_files.write().await.insert(
            id,
            proto::File {
                worktree_id: 1,
                path: path.into(),
                ..Default::default()
            },
        );
        let mut cache = self.zed.diagnostics.lock().unwrap();
        for variant in [
            proto::create_buffer_for_peer::Variant::State(proto::BufferState {
                id,
                base_text: "a🙂z\n".into(),
                ..Default::default()
            }),
            proto::create_buffer_for_peer::Variant::Chunk(proto::BufferChunk {
                buffer_id: id,
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

    async fn execute(&mut self, navigation: &NavigationRef, ids: &[u64]) -> Result<Response> {
        let mut task = self.spawn(action(navigation, Action::Execute));
        let request = self.navigation.recv().await.unwrap();
        self.reply_navigation(request, definitions(ids));
        loop {
            tokio::select! {
                result = &mut task => return result.unwrap(),
                message = self.outbound.recv() => {
                    ack_registration(&self.zed, message.unwrap()).await;
                }
            }
        }
    }

    fn reply_navigation(
        &self,
        envelope: wire::CowboyBufferSyncEnvelope,
        responses: Vec<proto::LspResponse>,
    ) -> proto::LspQuery {
        use wire::cowboy_buffer_sync_envelope::Payload;
        let Some(Payload::NavigationRequest(request)) = envelope.payload else {
            panic!("not a bounded navigation query")
        };
        let query = proto::LspQuery::decode(request.query.as_slice()).unwrap();
        let reply = wire::CowboyBufferSyncEnvelope {
            responding_to: Some(envelope.id),
            payload: Some(Payload::NavigationResponse(
                wire::CowboyNavigationResponse {
                    protocol: 1,
                    outcome: wire::cowboy_navigation_response::Outcome::Complete as i32,
                    result: proto::LspQueryResponse {
                        project_id: proto::REMOTE_SERVER_PROJECT_ID,
                        responses,
                        ..Default::default()
                    }
                    .encode_to_vec(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert!(self.zed.sync.response(envelope.id, &reply.encode_to_vec()));
        query
    }

    fn aba(&self, id: u64) {
        use crate::coordinates::tests::{peer, wire};
        let mut source = peer("a🙂z\n", 1);
        let edit = source.edit([(0..0, "changed")]);
        source.finalize_last_transaction();
        let undo = source.undo().unwrap().1;
        self.zed
            .diagnostics
            .lock()
            .unwrap()
            .observe(&proto::envelope::Payload::UpdateBuffer(
                proto::UpdateBuffer {
                    buffer_id: id,
                    operations: vec![wire(&edit), wire(&undo)],
                    ..Default::default()
                },
            ));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn definitions(ids: &[u64]) -> Vec<proto::LspResponse> {
    vec![proto::LspResponse {
        response: Some(proto::lsp_response::Response::GetDefinitionResponse(
            proto::GetDefinitionResponse {
                links: ids
                    .iter()
                    .map(|id| proto::LocationLink {
                        target: Some(proto::Location {
                            buffer_id: *id,
                            start: Some(crate::position_anchor(*id, 1)),
                            end: Some(crate::position_anchor(*id, 5)),
                        }),
                        ..Default::default()
                    })
                    .collect(),
            },
        )),
        ..Default::default()
    }]
}

fn action(navigation: &NavigationRef, action: Action) -> Request {
    Request::BufferNavigation {
        navigation: navigation.clone(),
        action,
    }
}

fn state(response: Response) -> State {
    let Response::OwnedBufferNavigation { state, .. } = response else {
        panic!("wrong reply")
    };
    state
}

async fn ack_registration(zed: &Zed, message: proto::Envelope) {
    assert!(matches!(
        message.payload,
        Some(proto::envelope::Payload::RegisterBufferWithLanguageServers(
            _
        ))
    ));
    zed.pending
        .lock()
        .await
        .remove(&message.id)
        .unwrap()
        .send(proto::Envelope {
            responding_to: Some(message.id),
            payload: Some(proto::envelope::Payload::Ack(proto::Ack {})),
            ..Default::default()
        })
        .unwrap();
}

#[tokio::test]
async fn preparation_has_no_native_effect_and_source_release_cannot_be_retargeted() {
    let mut f = Fixture::new().await;
    let nav = f.prepare().await;
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(
        f.buffers
            .active
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .lease_ids
            .len(),
        1
    );
    f.request(Request::ReleaseBufferLease {
        lease: f.lease.clone(),
    })
    .await
    .unwrap();
    assert_eq!(f.closed.recv().await.unwrap(), [7]);
    assert!(f.outbound.try_recv().is_err());
    assert!(f.request(action(&nav, Action::Execute)).await.is_err());
    assert!(f.outbound.try_recv().is_err());
    assert!(matches!(
        state(f.request(action(&nav, Action::Release)).await.unwrap()),
        State::Released
    ));
}

#[tokio::test]
async fn original_targets_survive_source_close_and_deleted_paths_without_reopen() {
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    f.target(9, "second-target").await;
    let nav = f.prepare().await;
    let State::Retained { locations } = state(f.execute(&nav, &[8, 8, 9]).await.unwrap()) else {
        panic!("not retained")
    };
    assert_eq!(locations.len(), 3);
    assert_eq!((locations[0].start.column, locations[0].end.column), (1, 3));
    assert_eq!(locations[0].content, f.content(8));
    std::fs::remove_file(f.root.join("source")).unwrap();
    f.request(Request::ReleaseBufferLease {
        lease: f.lease.clone(),
    })
    .await
    .unwrap();
    assert!(f.outbound.try_recv().is_err());
    for action_kind in [Action::Query, Action::Execute] {
        assert!(matches!(
            state(f.request(action(&nav, action_kind)).await.unwrap()),
            State::Retained { .. }
        ));
    }
    assert!(f.outbound.try_recv().is_err());
    let task = f.spawn(Request::ReadBufferNavigation {
        navigation: nav.clone(),
        destination: 1,
        content: f.content(8),
        query: Query::Hover {
            position: Point { row: 0, column: 1 },
        },
    });
    let request = f.outbound.recv().await.unwrap();
    let query = coordinate_queries::reply(&f.zed, request, Vec::new()).await;
    let Some(proto::lsp_query::Request::GetHover(value)) = query.request else {
        panic!("wrong query")
    };
    assert_eq!(value.buffer_id, 8);
    task.await.unwrap().unwrap();
    assert!(matches!(
        state(f.request(action(&nav, Action::Release)).await.unwrap()),
        State::Released
    ));
    assert_eq!(f.closed.recv().await.unwrap(), [7, 8, 9]);
    assert!(f.closed.try_recv().is_err());
    assert!(f.outbound.try_recv().is_err());
    assert!(f.buffers.active.read().await.is_empty());
    assert!(matches!(
        state(f.request(action(&nav, Action::Release)).await.unwrap()),
        State::Released
    ));
    assert!(f.outbound.try_recv().is_err());
}

#[tokio::test]
async fn peer_and_alias_owners_are_not_closed_by_navigation_release() {
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    let alias = (f.root.join("alias"), PathBuf::from("peer"));
    f.buffers.active.write().await.insert(
        alias.clone(),
        crate::BufferLease {
            lease_ids: [BufferOwner::Owned(2)].into(),
            remote_id: 8,
            version: vec![],
            sync: None,
            closing: false,
        },
    );
    let nav = f.prepare().await;
    f.execute(&nav, &[8]).await.unwrap();
    f.request(action(&nav, Action::Release)).await.unwrap();
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(f.buffers.active.read().await.len(), 2);
    crate::close_buffer_at(
        alias.0,
        alias.1,
        BufferOwner::Owned(2),
        &f.buffers,
        Some(&f.zed),
        || {},
    )
    .await
    .unwrap();
    assert_eq!(f.closed.recv().await.unwrap(), [8]);
    assert!(f.outbound.try_recv().is_err());
    assert!(f.zed.diagnostics.lock().unwrap().revision(7).is_ok());
}

#[tokio::test]
async fn edit_undo_before_execute_or_destination_read_is_not_equal_content_authority() {
    let mut f = Fixture::new().await;
    let nav = f.prepare().await;
    let content = f.content(7);
    f.aba(7);
    assert_eq!(content, f.content(7));
    assert!(f.request(action(&nav, Action::Execute)).await.is_err());
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&nav, Action::Release)).await.unwrap();
    f.target(8, "target").await;
    let nav = f.prepare().await;
    f.execute(&nav, &[8]).await.unwrap();
    let content = f.content(8);
    f.aba(8);
    assert_eq!(content, f.content(8));
    assert!(
        f.request(Request::ReadBufferNavigation {
            navigation: nav.clone(),
            destination: 0,
            content,
            query: Query::Symbols {}
        })
        .await
        .is_err()
    );
    assert!(f.outbound.try_recv().is_err());
    // Releasing an obsolete observation still uses its original resource.
    f.request(action(&nav, Action::Release)).await.unwrap();
}

#[tokio::test]
async fn disconnected_observer_keeps_one_saved_navigation_result() {
    use tokio::io::AsyncWriteExt as _;
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    let nav = f.prepare().await;
    let (mut client, peer) = tokio::net::UnixStream::pair().unwrap();
    let task = tokio::spawn(crate::handle(
        peer,
        f.worktrees.clone(),
        f.buffers.clone(),
        Some(f.zed.clone()),
    ));
    let mut request = serde_json::to_vec(&action(&nav, Action::Execute)).unwrap();
    request.push(b'\n');
    client.write_all(&request).await.unwrap();
    let native = f.navigation.recv().await.unwrap();
    drop(client);
    f.reply_navigation(native, definitions(&[8]));
    ack_registration(&f.zed, f.outbound.recv().await.unwrap()).await;
    assert!(task.await.unwrap().is_err());
    for action_kind in [Action::Query, Action::Execute] {
        assert!(matches!(
            state(f.request(action(&nav, action_kind)).await.unwrap()),
            State::Retained { .. }
        ));
    }
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&nav, Action::Release)).await.unwrap();
}
