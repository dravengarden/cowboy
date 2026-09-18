//! Actual private server, isolated by the native conformance parent. Fault loss
//! occurs only after receiving a real Closed response; it grants no recovery.
use super::*;
use crate::buffer_leases::{LeaseRef, LeaseState};

async fn open(
    zed: &Zed,
    worktrees: &Worktrees,
    buffers: &Buffers,
    root: &Path,
    name: &str,
) -> LeaseRef {
    let Response::BufferLease { lease, .. } = respond(
        Request::PrepareBuffer {
            worktree: root.into(),
            path: name.into(),
        },
        worktrees,
        buffers,
        Some(zed),
    )
    .await
    .unwrap() else {
        panic!("not prepared")
    };
    assert!(matches!(
        respond(
            Request::OpenBufferLease {
                lease: lease.clone()
            },
            worktrees,
            buffers,
            Some(zed)
        )
        .await
        .unwrap(),
        Response::BufferLease {
            state: LeaseState::Open,
            ..
        }
    ));
    lease
}

async fn native_owner(zed: &Zed, id: u64, retained: bool) {
    let position = zed.diagnostics.lock().unwrap().position(id, 0, 0).unwrap();
    let result = crate::navigation_native::query(
        zed,
        crate::navigation_request(
            id,
            &position.version,
            position.anchor,
            NavigationKind::Definition,
        ),
    )
    .await;
    if retained {
        assert!(result.unwrap().is_empty());
    } else {
        assert_eq!(
            result
                .unwrap_err()
                .downcast_ref::<crate::navigation_native::NativeRefusal>()
                .unwrap()
                .0,
            wire::cowboy_navigation_response::Refusal::Source
        );
    }
}

async fn atomic_refusal(zed: &Zed, ids: &[u64]) {
    let instance = exchange(zed, request(vec![], vec![])).await.unwrap();
    let mut missing = ids.to_vec();
    missing.push(u64::MAX);
    let Payload::CloseResponse(refused) = zed
        .sync
        .exchange(
            zed,
            Payload::CloseRequest(request(instance.0.to_vec(), missing.clone())),
        )
        .await
        .unwrap()
    else {
        panic!("not native close response")
    };
    assert_eq!(refused.outcome, Outcome::Refused as i32);
    assert_eq!(refused.instance, instance.0);
    assert_eq!(refused.buffer_ids, missing);
    let mut foreign = instance.0;
    foreign[0] ^= 1;
    assert!(
        exchange(zed, request(foreign.to_vec(), ids.to_vec()))
            .await
            .is_err()
    );
    for id in ids {
        native_owner(zed, *id, true).await;
    }
}

async fn retained_uncertainty(
    zed: &Zed,
    worktrees: &Worktrees,
    buffers: &Buffers,
    lease: &LeaseRef,
) {
    for request in [
        Request::QueryBufferLease {
            lease: lease.clone(),
        },
        Request::ReleaseBufferLease {
            lease: lease.clone(),
        },
        Request::OpenBufferLease {
            lease: lease.clone(),
        },
    ] {
        assert!(matches!(
            respond(request, worktrees, buffers, Some(zed))
                .await
                .unwrap(),
            Response::BufferLease {
                state: LeaseState::Unknown,
                ..
            }
        ));
    }
    let active = buffers.active.read().await;
    assert!(active.values().any(|buffer| buffer.closing));
    assert!(crate::sync_owners::ensure_admission(buffers, &active).is_err());
}

pub(crate) async fn exercise(zed: &Zed, root: &Path) {
    let workspace = root.join("close-worktree");
    std::fs::create_dir(&workspace).unwrap();
    for name in ["first.txt", "second.txt"] {
        std::fs::write(workspace.join(name), "keep🙂\n").unwrap();
    }
    let worktrees: Worktrees = Arc::default();
    let buffers: Buffers = Arc::default();
    respond(
        Request::OpenWorktree {
            path: workspace.clone(),
            trusted: true,
        },
        &worktrees,
        &buffers,
        Some(zed),
    )
    .await
    .unwrap();
    let first = open(zed, &worktrees, &buffers, &workspace, "first.txt").await;
    let second = open(zed, &worktrees, &buffers, &workspace, "second.txt").await;
    let (first_id, second_id) = {
        let active = buffers.active.read().await;
        (
            active[&(workspace.clone(), "first.txt".into())].remote_id,
            active[&(workspace.clone(), "second.txt".into())].remote_id,
        )
    };
    let mut ids = vec![first_id, second_id];
    ids.sort_unstable();
    atomic_refusal(zed, &ids).await;
    zed.sync.discard_next_close_response();
    assert!(
        respond(
            Request::ReleaseBufferLease {
                lease: first.clone()
            },
            &worktrees,
            &buffers,
            Some(zed)
        )
        .await
        .is_err()
    );
    assert!(zed.sync.discarded_close_response());
    // Native proves its peer ownership is gone. That observation cannot repair
    // the adapter's lost original acknowledgement or retire retained authority.
    native_owner(zed, first_id, false).await;
    native_owner(zed, second_id, true).await;
    retained_uncertainty(zed, &worktrees, &buffers, &first).await;
    assert!(matches!(
        respond(
            Request::ReleaseBufferLease { lease: second },
            &worktrees,
            &buffers,
            Some(zed)
        )
        .await
        .unwrap(),
        Response::BufferLease {
            state: LeaseState::Released,
            ..
        }
    ));
    assert_eq!(buffers.active.read().await.len(), 1);
    retained_uncertainty(zed, &worktrees, &buffers, &first).await;
    assert!(zed.diagnostics.lock().unwrap().revision(first_id).is_ok());
    assert!(zed.diagnostics.lock().unwrap().revision(second_id).is_err());
    for name in ["first.txt", "second.txt"] {
        assert_eq!(
            std::fs::read_to_string(workspace.join(name)).unwrap(),
            "keep🙂\n"
        );
    }
    println!(
        "native close: atomic missing-member/foreign-instance refusal, actual lost Closed response, original Unknown/no replay and independent confirmed release passed (no restoration or background-drain claim)"
    );
}
