//! Saturate an independent real native process across two worktrees.
//! No product retry, eviction, implicit recovery or production installation.
use super::*;
use wire::cowboy_buffer_sync_envelope::Payload;
use wire::cowboy_close_buffers_response::Outcome;

pub(super) async fn exercise(server: &Path, root: &Path) {
    let zed = ZedRuntime::start_with_disconnect(server, &root.join("acquisition-state"), || {})
        .await
        .unwrap();
    let mut worktrees = Vec::new();
    for name in ["acquisition-a", "acquisition-b"] {
        let path = root.join(name);
        tokio::fs::create_dir(&path).await.unwrap();
        for index in 0..=64 {
            tokio::fs::write(path.join(format!("file-{index}.txt")), "retained🙂\n")
                .await
                .unwrap();
        }
        worktrees.push(zed.open_worktree(&path, true).await.unwrap().0);
    }
    let mut ids = Vec::new();
    for index in 0..64 {
        let (buffer, _) = zed
            .open_buffer(
                worktrees[index % 2],
                Path::new(&format!("file-{index}.txt")),
            )
            .await
            .unwrap();
        ids.push(buffer);
    }
    for worktree in &worktrees {
        let error = zed
            .open_buffer(*worktree, Path::new("file-64.txt"))
            .await
            .expect_err("native process must share its acquisition capacity across worktrees");
        let detail = error.to_string();
        assert!(detail.starts_with("Zed request failed:"), "{detail}");
        assert!(detail.contains("native buffer acquisition capacity exceeded"));
    }
    for id in &ids {
        assert_native_mirror(&zed, *id, "retained🙂\n").await;
    }
    let Payload::CloseResponse(probe) = zed
        .sync
        .exchange(
            &zed,
            Payload::CloseRequest(wire::CowboyCloseBuffers {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                protocol: 1,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
    else {
        panic!("not a native close probe");
    };
    assert_eq!(probe.outcome, Outcome::Supported as i32);
    ids.sort_unstable();
    for chunk in ids.chunks(32) {
        let Payload::CloseResponse(response) = zed
            .sync
            .exchange(
                &zed,
                Payload::CloseRequest(wire::CowboyCloseBuffers {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    protocol: 1,
                    instance: probe.instance.clone(),
                    buffer_ids: chunk.to_vec(),
                }),
            )
            .await
            .unwrap()
        else {
            panic!("not a native close reply");
        };
        assert_eq!(response.outcome, Outcome::Closed as i32);
        assert_eq!(response.buffer_ids, chunk);
        assert_eq!(response.instance, probe.instance);
    }
    // This specific plain-text fixture really relinquishes its native handles.
    // The ACK alone is not general quiescence (the native GPUI gate checks that).
    let (buffer, _) = zed
        .open_buffer(worktrees[0], Path::new("file-63.txt"))
        .await
        .expect("a separate acquisition after actual buffer release must be possible");
    assert!(!ids.contains(&buffer));
    assert_native_mirror(&zed, buffer, "retained🙂\n").await;
    stop(&zed).await;
    println!(
        "native acquisition: shared 64-buffer capacity across two worktrees, actual overflow refusal, preserved originals and separate post-release acquisition passed"
    );
}
