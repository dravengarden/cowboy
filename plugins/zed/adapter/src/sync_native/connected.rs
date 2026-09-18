//! Opt-in real private-server acceptance, never a production connection.
use super::*;
use sha2::{Digest as _, Sha256};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires immutable private server; run just zed-native-sync-conformance"]
fn immutable_native_sync() {
    let server =
        std::env::var_os("COWBOY_TEST_NATIVE_SYNC_SERVER").expect("immutable server is required");
    let server = std::fs::canonicalize(server).unwrap();
    assert!(server.starts_with("/nix/store") && server.is_file());
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let root = std::env::temp_dir().join(format!("cw-native-{:032x}", u128::from_be_bytes(random)));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(root);
    for dir in [
        "home", "config", "data", "cache", "state", "runtime", "worktree", "tmp",
    ] {
        std::fs::create_dir(scratch.0.join(dir)).unwrap();
    }
    // Child-only environment: neither native startup nor tests can discover
    // ordinary Zed state, Git credentials, language tools or updater settings.
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "sync_native::connected::native_process_child",
            "--exact",
            "--ignored",
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", scratch.0.join("home"))
        .env("XDG_CONFIG_HOME", scratch.0.join("config"))
        .env("XDG_DATA_HOME", scratch.0.join("data"))
        .env("XDG_CACHE_HOME", scratch.0.join("cache"))
        .env("XDG_STATE_HOME", scratch.0.join("state"))
        .env("XDG_RUNTIME_DIR", scratch.0.join("runtime"))
        .env("TMPDIR", scratch.0.join("tmp"))
        .env("PATH", "/nonexistent-cowboy-test-tools")
        .env("COWBOY_TEST_NATIVE_SYNC_SERVER", server)
        .env("COWBOY_TEST_NATIVE_SYNC_ROOT", &scratch.0)
        .current_dir(&scratch.0);
    if let Some(adapter) = std::env::var_os("COWBOY_TEST_NATIVE_NAVIGATION_ADAPTER") {
        let adapter = std::fs::canonicalize(adapter).unwrap();
        assert!(adapter.starts_with("/nix/store") && adapter.is_file());
        command.env("COWBOY_TEST_NATIVE_NAVIGATION_ADAPTER", adapter);
    }
    let status = command.status().unwrap();
    assert!(status.success(), "isolated native sync child failed");
}

async fn action(
    zed: &ZedRuntime,
    instance: &[u8],
    id: u64,
    action: Action,
) -> Result<wire::CowboyBufferSyncResponse> {
    zed.sync
        .request(
            zed,
            wire::CowboyBufferSync {
                protocol: 1,
                instance: instance.to_vec(),
                operation_id: id,
                action: action as i32,
                ..Default::default()
            },
        )
        .await
}

async fn prepare(
    zed: &ZedRuntime,
    instance: &[u8],
    buffer: u64,
    version: &[BufferVersionEntry],
    content: &[u8],
) -> Result<u64> {
    let reply = zed
        .sync
        .request(
            zed,
            wire::CowboyBufferSync {
                protocol: 1,
                action: Action::Prepare as i32,
                instance: instance.to_vec(),
                buffer_id: buffer,
                version: version
                    .iter()
                    .map(|value| wire::CowboyBufferSyncVersion {
                        replica_id: value.replica_id,
                        timestamp: value.timestamp,
                    })
                    .collect(),
                content_sha256: Sha256::digest(content).to_vec(),
                content_bytes: u32::try_from(content.len()).unwrap(),
                ..Default::default()
            },
        )
        .await?;
    Ok(reply.operation_id)
}

async fn finish(zed: &ZedRuntime, instance: &[u8], id: u64) -> wire::CowboyBufferSyncResponse {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = action(zed, instance, id, Action::Query).await.unwrap();
            if state.phase != Phase::Pending as i32 && state.phase != Phase::Prepared as i32 {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native effect did not settle")
}

async fn native_edits_and_close_refuse(
    zed: &ZedRuntime,
    instance: &[u8],
    workspace: &Path,
    worktree: u64,
) {
    for (case, undo) in [("dirty.txt", false), ("undo.txt", true)] {
        let base = "keep🙂\n";
        tokio::fs::write(workspace.join(case), base).await.unwrap();
        let (buffer, version) = zed.open_buffer(worktree, Path::new(case)).await.unwrap();
        let ticket = prepare(zed, instance, buffer, &version, base.as_bytes())
            .await
            .unwrap();
        let mut peer = coordinates::tests::peer(base, 1);
        let mut operations = vec![coordinates::tests::wire(&peer.edit([(0..0, "unsaved")]))];
        peer.finalize_last_transaction();
        if undo {
            operations.push(coordinates::tests::wire(&peer.undo().unwrap().1));
        }
        zed.request(proto::envelope::Payload::UpdateBuffer(
            proto::UpdateBuffer {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id: buffer,
                operations,
            },
        ))
        .await
        .unwrap();
        action(zed, instance, ticket, Action::Apply).await.unwrap();
        let refused = finish(zed, instance, ticket).await;
        assert_eq!(
            refused.phase,
            Phase::Refused as i32,
            "edit/undo passed: {case}"
        );
        assert_eq!(refused.refusal, Refusal::Changed as i32);
        assert_eq!(
            tokio::fs::read_to_string(workspace.join(case))
                .await
                .unwrap(),
            base
        );
        action(zed, instance, ticket, Action::Retire).await.unwrap();
        zed.close_buffer(buffer).unwrap();
    }
    tokio::fs::write(workspace.join("closed.txt"), "original")
        .await
        .unwrap();
    let (buffer, version) = zed
        .open_buffer(worktree, Path::new("closed.txt"))
        .await
        .unwrap();
    let ticket = prepare(zed, instance, buffer, &version, b"original")
        .await
        .unwrap();
    zed.close_buffer(buffer).unwrap();
    // Query is foreground, like CloseBuffer, and fences that earlier message.
    action(zed, instance, ticket, Action::Query).await.unwrap();
    action(zed, instance, ticket, Action::Apply).await.unwrap();
    let refused = finish(zed, instance, ticket).await;
    assert_eq!(refused.phase, Phase::Refused as i32);
    assert_eq!(refused.refusal, Refusal::Shared as i32);
    action(zed, instance, ticket, Action::Retire).await.unwrap();
}

async fn native_source_bounds_and_lost_reply(
    zed: &ZedRuntime,
    instance: &[u8],
    workspace: &Path,
    worktree: u64,
) {
    for (case, content) in [
        ("utf8.txt", vec![0xff]),
        ("crlf.txt", b"line\r\n".to_vec()),
        ("bom.txt", b"\xef\xbb\xbfline".to_vec()),
    ] {
        let path = workspace.join(case);
        tokio::fs::write(&path, "old").await.unwrap();
        let (buffer, version) = zed.open_buffer(worktree, Path::new(case)).await.unwrap();
        // Prepare first; then replace only disk bytes. The original File Arc
        // may also change in real watcher order: either refusal is safe.
        let ticket = prepare(zed, instance, buffer, &version, &content)
            .await
            .unwrap();
        tokio::fs::write(&path, content).await.unwrap();
        action(zed, instance, ticket, Action::Apply).await.unwrap();
        let refused = finish(zed, instance, ticket).await;
        assert_eq!(
            refused.phase,
            Phase::Refused as i32,
            "invalid text applied: {case}"
        );
        action(zed, instance, ticket, Action::Retire).await.unwrap();
        zed.close_buffer(buffer).unwrap();
    }
    tokio::fs::write(workspace.join("unobserved.txt"), "retained")
        .await
        .unwrap();
    let (buffer, version) = zed
        .open_buffer(worktree, Path::new("unobserved.txt"))
        .await
        .unwrap();
    let ticket = prepare(zed, instance, buffer, &version, b"retained")
        .await
        .unwrap();
    // Send the real protobuf with no response waiter at all. It is an admitted
    // operation, not a local mocked timeout; query can only use its old ticket.
    zed.sync
        .outbound
        .send(wire::CowboyBufferSyncEnvelope {
            id: zed.next_message_id.fetch_add(1, Ordering::Relaxed),
            responding_to: None,
            payload: Some(Payload::Request(wire::CowboyBufferSync {
                protocol: 1,
                instance: instance.to_vec(),
                operation_id: ticket,
                action: Action::Apply as i32,
                ..Default::default()
            })),
        })
        .await
        .unwrap();
    assert_eq!(
        finish(zed, instance, ticket).await.phase,
        Phase::Applied as i32
    );
    action(zed, instance, ticket, Action::Retire).await.unwrap();
    zed.close_buffer(buffer).unwrap();
}

async fn native_input_bounds(zed: &ZedRuntime, workspace: &Path, worktree: u64) {
    const LIMIT: usize = 4 * 1024 * 1024;
    let exact = workspace.join("input-exact.txt");
    tokio::fs::write(&exact, vec![b'a'; LIMIT]).await.unwrap();
    let (buffer, _) = zed
        .open_buffer(worktree, Path::new("input-exact.txt"))
        .await
        .expect("the exact raw/decoded text limit must be inclusive");
    assert_eq!(
        zed.diagnostics
            .lock()
            .unwrap()
            .content(buffer)
            .unwrap()
            .utf8_bytes,
        u32::try_from(LIMIT).unwrap()
    );
    zed.close_buffer(buffer).unwrap();

    let oversized = workspace.join("input-oversized.txt");
    tokio::fs::write(&oversized, vec![b'a'; LIMIT + 1])
        .await
        .unwrap();
    let refusal = zed
        .open_buffer(worktree, Path::new("input-oversized.txt"))
        .await
        .expect_err("oversized source must not become a native buffer");
    assert!(
        refusal.to_string().starts_with("Zed request failed:")
            && refusal.to_string().contains("bounded regular file"),
        "require the actual native budget refusal, not a timeout: {refusal:#}"
    );

    // Raw bytes fit, but decoding to UTF-8 would exceed the native text limit.
    let mut expanded = vec![0xff, 0xfe];
    for _ in 0..1_500_000 {
        expanded.extend_from_slice(&[0x00, 0x08]);
    }
    tokio::fs::write(workspace.join("input-expanded.txt"), &expanded)
        .await
        .unwrap();
    let refusal = zed
        .open_buffer(worktree, Path::new("input-expanded.txt"))
        .await
        .expect_err("bounded source decoding must be checked before CRDT construction");
    assert!(
        refusal.to_string().starts_with("Zed request failed:")
            && refusal
                .to_string()
                .contains("private native text exceeds budget"),
        "require the actual native decode refusal, not a timeout: {refusal:#}"
    );
    // Failure is not a runtime-wide crash and never mutates source bytes.
    assert_eq!(
        tokio::fs::metadata(&oversized).await.unwrap().len(),
        (LIMIT + 1) as u64
    );
    assert_eq!(
        tokio::fs::read(workspace.join("input-expanded.txt"))
            .await
            .unwrap(),
        expanded
    );
    zed.sync.probe(zed).await.unwrap();
    println!(
        "native input bounds: inclusive 4 MiB, oversized raw/decoded refusal, unchanged files and live native probe passed"
    );
}

fn child_paths() -> (PathBuf, PathBuf) {
    let root = PathBuf::from(
        std::env::var_os("COWBOY_TEST_NATIVE_SYNC_ROOT").expect("private child root"),
    );
    assert_eq!(
        PathBuf::from(std::env::var_os("HOME").unwrap()),
        root.join("home")
    );
    let server = PathBuf::from(std::env::var_os("COWBOY_TEST_NATIVE_SYNC_SERVER").unwrap());
    (root, server)
}

async fn assert_native_mirror(zed: &ZedRuntime, buffer: u64, content: &str) {
    let mirror = coordinates::Mirror::new(buffer, content).unwrap();
    let expected = mirror.content();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if zed
                .diagnostics
                .lock()
                .unwrap()
                .match_content(buffer, expected)
                .ok()
                .flatten()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native operation stream did not reach the same content");
}

async fn stop(zed: &ZedRuntime) {
    #[expect(
        clippy::used_underscore_binding,
        reason = "test explicitly reaps the otherwise drop-owned private child"
    )]
    zed._child.lock().await.kill().await.unwrap();
}

async fn restart_does_not_adopt_old_ticket(
    zed: &ZedRuntime,
    server: &Path,
    root: &Path,
    instance: &[u8],
    id: u64,
) {
    let mut foreign = instance.to_vec();
    foreign[0] ^= 1;
    assert!(action(zed, &foreign, id, Action::Query).await.is_err());
    stop(zed).await;
    let replacement = ZedRuntime::start_with_disconnect(server, &root.join("state"), || {})
        .await
        .unwrap();
    assert_ne!(
        replacement
            .sync
            .probe(&replacement)
            .await
            .unwrap()
            .as_slice(),
        instance
    );
    assert!(
        action(&replacement, instance, id, Action::Query)
            .await
            .is_err()
    );
    stop(&replacement).await;
}

#[tokio::test]
#[ignore = "private child of immutable_native_sync, not directly invocable"]
async fn native_process_child() {
    let (root, server) = child_paths();
    crate::buffer_navigation::connected_lsp::configure(&root);
    let workspace = root.join("worktree");
    let path = workspace.join("sample.txt");
    let old = "before🙂\n";
    let new = "after汉字\n";
    tokio::fs::write(&path, old).await.unwrap();
    let zed = Arc::new(
        ZedRuntime::start_with_disconnect(&server, &root.join("state"), || {})
            .await
            .unwrap(),
    );
    let instance = zed.sync.probe(&zed).await.unwrap();
    let (worktree, _) = zed.open_worktree(&workspace, true).await.unwrap();
    let (buffer, version) = zed
        .open_buffer(worktree, Path::new("sample.txt"))
        .await
        .unwrap();
    let revision = zed.diagnostics.lock().unwrap().revision(buffer).unwrap();
    let id = prepare(&zed, &instance, buffer, &version, new.as_bytes())
        .await
        .unwrap();
    let before = action(&zed, &instance, id, Action::Query).await.unwrap();
    assert_eq!(before.phase, Phase::Prepared as i32);
    // With the same native buffer, a mismatching source must not mutate it.
    assert_eq!(
        action(&zed, &instance, id, Action::Apply)
            .await
            .unwrap()
            .phase,
        Phase::Pending as i32
    );
    let refused = finish(&zed, &instance, id).await;
    assert_eq!(refused.phase, Phase::Refused as i32);
    assert_eq!(refused.refusal, Refusal::Source as i32);
    assert_eq!(
        action(&zed, &instance, id, Action::Apply).await.unwrap(),
        refused
    );
    action(&zed, &instance, id, Action::Retire).await.unwrap();
    assert_eq!(
        action(&zed, &instance, id, Action::Apply)
            .await
            .unwrap()
            .phase,
        Phase::Retired as i32
    );

    tokio::fs::write(&path, new).await.unwrap();
    let id2 = prepare(&zed, &instance, buffer, &version, new.as_bytes())
        .await
        .unwrap();
    assert!(id2 > id);
    action(&zed, &instance, id2, Action::Apply).await.unwrap();
    let applied = finish(&zed, &instance, id2).await;
    assert_eq!(
        applied.phase,
        Phase::Applied as i32,
        "native refused: {applied:?}"
    );
    assert_eq!(
        applied.content_sha256,
        Sha256::digest(new.as_bytes()).to_vec()
    );
    assert_native_mirror(&zed, buffer, new).await;
    assert!(
        zed.diagnostics
            .lock()
            .unwrap()
            .check(buffer, revision)
            .is_err()
    );
    assert_eq!(
        action(&zed, &instance, id2, Action::Apply).await.unwrap(),
        applied
    );
    assert_eq!(
        tokio::fs::read_to_string(&path).await.unwrap(),
        new,
        "sync must not write disk"
    );
    assert!(
        prepare(&zed, &instance, buffer, &version, old.as_bytes())
            .await
            .is_err(),
        "stale native version accepted"
    );
    action(&zed, &instance, id2, Action::Retire).await.unwrap();
    assert!(
        action(&zed, &instance, id2 + 1, Action::Query)
            .await
            .is_err()
    );
    native_edits_and_close_refuse(&zed, &instance, &workspace, worktree).await;
    native_source_bounds_and_lost_reply(&zed, &instance, &workspace, worktree).await;
    native_input_bounds(&zed, &workspace, worktree).await;
    owned_resources(&zed, &root).await;
    restart_does_not_adopt_old_ticket(&zed, &server, &root, &instance, id2).await;
    crate::buffer_navigation::connected_lsp::immutable_pair(&root, &server).await;
    println!(
        "native sync identity, edit/undo/close refusal, source validation and lost-reply checks passed"
    );
}

async fn owned_resources(zed: &Zed, root: &Path) {
    let workspace = root.join("worktree");
    crate::sync_owners::connected::exercise(zed, &workspace).await;
    crate::buffer_navigation::connected::exercise(zed, &workspace).await;
    crate::buffer_navigation::connected_lsp::exercise(zed, root).await;
}
