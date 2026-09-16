//! Called by the installed, signed real-runtime gate; disposable files only.
use super::*;
use crate::machine_protocol::code_buffer_sync::{Action, OperationRef, Purpose, Request, State};
use serde_json::{Value, json};

fn invocation(scope: &PluginExecutionScope, action: Action) -> CodeBufferSyncInvocation {
    scope
        .code_buffer_sync(Request {
            service_id: format!("svc-{}", "a".repeat(32)),
            machine_id: "fixture".into(),
            action,
        })
        .unwrap()
}

pub(super) async fn prepare(
    store: &MachinePluginStore,
    worktree: &Path,
) -> (PluginExecutionScope, OperationRef, Value, Value) {
    fs::write(worktree.join("core-sync.txt"), "old native text\n").unwrap();
    let mut owners = Vec::new();
    for _ in 0..2 {
        let prepared = store
            .code_request(
                "zed",
                &json!({"type":"prepareBuffer","worktree":worktree,"path":"core-sync.txt"}),
                None,
            )
            .await
            .unwrap();
        let lease = prepared["lease"].clone();
        store
            .code_request(
                "zed",
                &json!({"type":"openBufferLease","lease":lease}),
                None,
            )
            .await
            .unwrap();
        owners.push(lease);
    }
    let text = "new exact native text\n汉🙂\n";
    fs::write(worktree.join("core-sync.txt"), text).unwrap();
    let content =
        json!({"sha256":format!("{:x}", Sha256::digest(text.as_bytes())),"utf8Bytes":text.len()});
    let scope = PluginExecutionScope::new(Some(&format!("svc-{}", "a".repeat(32))), "fixture");
    let prepare = Action::Prepare {
        lease: serde_json::from_value(owners[0].clone()).unwrap(),
        purpose: Purpose::RefreshFromDisk,
        content: serde_json::from_value(content.clone()).unwrap(),
    };
    assert!(
        store
            .synchronize_code_buffer(invocation(&scope, prepare.clone()))
            .await
            .is_err(),
        "shared native owner was admitted"
    );
    store
        .code_request(
            "zed",
            &json!({"type":"releaseBufferLease","lease":owners[1]}),
            None,
        )
        .await
        .unwrap();
    // The generic path remains closed even when callers inject a worktree.
    assert!(store.code_request("zed", &json!({"type":"prepareBufferSync","worktree":worktree,"lease":owners[0],"purpose":"refresh_from_disk","content":content}), None).await.is_err());
    let prepared = store
        .synchronize_code_buffer(invocation(&scope, prepare))
        .await
        .unwrap();
    assert_eq!(prepared.state, State::Prepared {});
    assert!(
        store
            .code_request(
                "zed",
                &json!({"type":"releaseBufferLease","lease":owners[0]}),
                None
            )
            .await
            .is_err()
    );
    (scope, prepared.operation, owners.remove(0), content)
}

pub(super) async fn finish(
    store: &MachinePluginStore,
    prepared: (PluginExecutionScope, OperationRef, Value, Value),
) {
    let (scope, operation, lease, content) = prepared;
    // Installation is now gone. Continuation must retain the original native
    // process rather than selecting a new package or a legacy socket.
    let mut observed = store
        .synchronize_code_buffer(invocation(
            &scope,
            Action::Apply {
                operation: operation.clone(),
            },
        ))
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while matches!(observed.state, State::Pending {} | State::Unknown {}) {
        assert!(
            Instant::now() < deadline,
            "real synchronization did not settle"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        observed = store
            .synchronize_code_buffer(invocation(
                &scope,
                Action::Query {
                    operation: operation.clone(),
                },
            ))
            .await
            .unwrap();
    }
    let State::Applied {
        content: applied, ..
    } = &observed.state
    else {
        panic!("real synchronization refused: {:?}", observed.state)
    };
    assert_eq!(serde_json::to_value(applied).unwrap(), content);
    assert_eq!(
        store
            .synchronize_code_buffer(invocation(
                &scope,
                Action::Apply {
                    operation: operation.clone()
                }
            ))
            .await
            .unwrap(),
        observed
    );
    // A receipt may precede native text events. The mirror floor must refuse
    // stale reads, then expose only the exact synchronized content.
    loop {
        let read = store.code_request("zed", &json!({"type":"readBufferLease","lease":lease,"request":{"kind":"content","content":content,"query":{"kind":"language"}}}), None).await;
        if let Ok(read) = read {
            assert_eq!(read["result"]["result"]["kind"], "observed");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native mirror did not observe synchronization"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        store
            .synchronize_code_buffer(invocation(&scope, Action::Retire { operation }))
            .await
            .unwrap()
            .state,
        State::Retired {}
    );
    store
        .code_request(
            "zed",
            &json!({"type":"releaseBufferLease","lease":lease}),
            None,
        )
        .await
        .unwrap();
}
