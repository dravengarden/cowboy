//! Real native outcome eviction must retain uncertainty, never prove no Open.
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use wire::cowboy_buffer_sync_envelope::Payload;
use wire::cowboy_open_attempt_response::Outcome;

pub(super) async fn exercise(server: &Path, root: &Path) {
    let zed = ZedRuntime::start_with_disconnect(server, &root.join("journal-state"), || {})
        .await
        .unwrap();
    let workspace = root.join("journal-worktree");
    tokio::fs::create_dir(&workspace).await.unwrap();
    tokio::fs::write(workspace.join("file.txt"), "original🙂\n")
        .await
        .unwrap();
    let (worktree, _) = zed.open_worktree(&workspace, true).await.unwrap();
    let fence = crate::native_open::Fence::default();
    let attempt = fence.begin().unwrap();
    let original = AtomicU32::new(0);
    let (buffer, _) = zed
        .open_buffer_tracked(worktree, Path::new("file.txt"), |id| {
            original.store(id, Ordering::Relaxed);
            attempt.dispatching(id);
        })
        .await
        .unwrap();
    drop(attempt);
    // Direct fixture traffic deliberately bypasses adapter admission. Every
    // request must alias the original peer; it must not create 1,024 buffers.
    for _ in 0..1024 {
        let response = zed
            .request(proto::envelope::Payload::OpenBufferByPath(
                proto::OpenBufferByPath {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    worktree_id: worktree,
                    path: "file.txt".into(),
                },
            ))
            .await
            .unwrap();
        let Some(proto::envelope::Payload::OpenBufferResponse(response)) = response.payload else {
            panic!("wrong native Open reply");
        };
        assert_eq!(response.buffer_id, buffer);
    }
    let Payload::OpenAttemptResponse(reply) = zed
        .sync
        .exchange(
            &zed,
            Payload::OpenAttemptRequest(wire::CowboyOpenAttempt {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                protocol: 1,
                message_id: original.load(Ordering::Relaxed),
            }),
        )
        .await
        .unwrap()
    else {
        panic!("wrong native journal reply");
    };
    assert_eq!(reply.outcome, Outcome::Forgotten as i32);
    assert!(
        crate::native_open::recover(&fence, &HashMap::new(), &zed)
            .await
            .is_err()
    );
    assert!(fence.check().is_err());
    assert_native_mirror(&zed, buffer, "original🙂\n").await;
    stop(&zed).await; // Disposable fixture teardown, not recovery evidence.
    println!(
        "native Open journal: actual settled-result eviction reports Forgotten, adapter keeps original fence and native buffer without Close/replay"
    );
}
