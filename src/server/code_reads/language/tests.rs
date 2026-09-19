use super::*;
use crate::core::{Hub, SessionOrigin, SessionRegistration};
use crate::machine_control::MachineControl;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use serde_json::json;
use tokio::sync::mpsc;

fn queries() -> [(Query, serde_json::Value); 4] {
    [
        (Query::Language, json!({"type":"bufferLanguage"})),
        (
            Query::Hover { row: 2, column: 3 },
            json!({"type":"bufferHover","row":2,"column":3}),
        ),
        (
            Query::Navigation {
                row: 2,
                column: 3,
                kind: CodeNavigationKind::Definition,
            },
            json!({"type":"bufferNavigate","row":2,"column":3,"kind":"definition"}),
        ),
        (Query::Outline, json!({"type":"bufferSymbols"})),
    ]
}

#[test]
fn path_queries_encode_only_the_closed_original_contract() {
    for (query, mut expected) in queries() {
        expected["worktree"] = json!("/worktree");
        expected["path"] = json!("a.rs");
        assert_eq!(
            serde_json::to_value(Request {
                worktree: "/worktree",
                path: "a.rs",
                query,
            })
            .unwrap(),
            expected
        );
    }
}

#[tokio::test]
async fn response_projection_cannot_accept_a_different_query_or_resource_result() {
    let replies = [
        json!({"type":"bufferLanguage","api_version":1,"path":"a.rs","version":[],"diagnostics":[],"inlay_hints":[],"semantic_tokens":[]}),
        json!({"type":"bufferHover","api_version":1,"path":"a.rs","contents":[]}),
        json!({"type":"bufferNavigation","api_version":1,"path":"a.rs","locations":[]}),
        json!({"type":"bufferSymbols","api_version":1,"path":"a.rs","symbols":[]}),
        json!({"type":"buffer","api_version":1,"path":"a.rs","leases":1}),
    ];
    for (index, (query, _)) in queries().into_iter().enumerate() {
        for (reply_index, reply) in replies.iter().enumerate() {
            let response = project(query, serde_json::from_value(reply.clone()).unwrap());
            assert_eq!(
                response.status(),
                if index == reply_index {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_GATEWAY
                }
            );
            if index == reply_index {
                let bytes = axum::body::to_bytes(response.into_body(), 4096)
                    .await
                    .unwrap();
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["apiVersion"], 1);
                assert_eq!(value["path"], "a.rs");
            }
        }
    }
}

#[tokio::test]
async fn language_queries_never_adopt_replacement_routes_or_changed_sessions() {
    for (query, _) in queries() {
        for change in ["before", "parked", "session", "none"] {
            let hub = Hub::new();
            hub.create_session(SessionRegistration {
                id: "session".into(),
                provider: "codex".into(),
                provider_version: String::new(),
                provider_generation_digest: String::new(),
                provider_auth_generation: None,
                provider_behavior: None,
                machine_id: "machine".into(),
                workspace_id: None,
                workspace_name: None,
                workspace_source_path: None,
                cwd: "/worktree".into(),
                title: "fixture".into(),
                origin: SessionOrigin::default(),
                system: false,
                owner_user_id: Some("user".into()),
                owner_username: None,
            });
            let control = MachineControl::default();
            let (tx, mut commands) = mpsc::unbounded_channel();
            let connection = control.install("machine".into(), "same-epoch".into(), false, 21, tx);
            let CodeReadScope::Session(scope) =
                super::super::session::resolve(&hub, &control, "service-test", "session").unwrap()
            else {
                unreachable!()
            };
            let request = serde_json::to_value(Request {
                worktree: "/worktree",
                path: "a.rs",
                query,
            })
            .unwrap();
            let replacement = || {
                let (tx, rx) = mpsc::unbounded_channel();
                control.install("machine".into(), "same-epoch".into(), false, 21, tx);
                rx
            };
            if change == "before" {
                let mut new_commands = replacement();
                assert!(
                    super::super::session::adapter_request(&hub, &control, None, &scope, request)
                        .await
                        .is_err()
                );
                assert!(commands.try_recv().is_err());
                assert!(new_commands.try_recv().is_err());
                continue;
            }
            let mut read = Box::pin(super::super::session::adapter_request(
                &hub,
                &control,
                None,
                &scope,
                request.clone(),
            ));
            let command = tokio::select! {
                _ = &mut read => panic!("read must wait for original reply"),
                command = commands.recv() => command.unwrap(),
            };
            let MachineCommand::AdapterRequest {
                request_id,
                adapter,
                payload,
            } = command
            else {
                panic!("expected adapter request")
            };
            assert_eq!(adapter, "zed");
            assert_eq!(payload, request);
            control.record_remote(
                &connection,
                MachineEvent::AdapterResponse {
                    request_id,
                    accepted: true,
                    detail: None,
                    payload: Some(
                        json!({"type":"bufferHover","api_version":1,"path":"a.rs","contents":[]}),
                    ),
                },
            );
            let mut new_commands = (change == "parked").then(replacement);
            if change == "session" {
                hub.update_session_cwd("session", "/other".into()).unwrap();
                hub.update_session_cwd("session", "/worktree".into())
                    .unwrap();
            }
            assert_eq!(read.await.is_ok(), change == "none");
            if let Some(commands) = &mut new_commands {
                assert!(commands.try_recv().is_err());
            }
            assert!(commands.try_recv().is_err());
        }
    }
}
