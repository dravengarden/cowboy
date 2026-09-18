//! Exercise the original native reload route only in the isolated test process.
//! This is not a fallback from any owned read, sync or navigation operation.
use super::*;

const ORIGINAL: &str = "retained🙂\n";

async fn reload(zed: &ZedRuntime, buffer: u64) -> Result<proto::Envelope> {
    zed.request(proto::envelope::Payload::ReloadBuffers(
        proto::ReloadBuffers {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            buffer_ids: vec![buffer],
        },
    ))
    .await
}

fn vector(version: &[BufferVersionEntry]) -> Vec<(u32, u32)> {
    version
        .iter()
        .map(|entry| (entry.replica_id, entry.timestamp))
        .collect()
}

pub(super) async fn exercise(zed: &ZedRuntime, instance: &[u8], workspace: &Path, worktree: u64) {
    let mut expanded = vec![0xff, 0xfe];
    for _ in 0..1_500_000 {
        expanded.extend_from_slice(&[0x00, 0x08]);
    }
    for (name, bytes) in [
        ("reload-growth.txt", vec![b'a'; 4 * 1024 * 1024 + 1]),
        ("reload-decoded.txt", expanded),
        ("reload-binary.txt", vec![0; 1024]),
    ] {
        let path = workspace.join(name);
        tokio::fs::write(&path, ORIGINAL).await.unwrap();
        let (buffer, _) = zed.open_buffer(worktree, Path::new(name)).await.unwrap();
        let version = zed.diagnostics.lock().unwrap().version(buffer).unwrap();
        tokio::fs::write(&path, &bytes).await.unwrap();
        let error = reload(zed, buffer).await.expect_err(
            "native reload must refuse growth/decoded expansion without applying a prefix",
        );
        assert!(error.to_string().starts_with("Zed request failed:"));
        assert_native_mirror(zed, buffer, ORIGINAL).await;
        assert_eq!(
            vector(&zed.diagnostics.lock().unwrap().version(buffer).unwrap()),
            vector(&version)
        );
        assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
        // The actual native conditional admission rejects a lodged reload_task.
        // Prepare is effect-free and is retired explicitly without any Apply.
        let ticket = prepare(zed, instance, buffer, &version, ORIGINAL.as_bytes())
            .await
            .expect("failed reload must clear only its completed task");
        action(zed, instance, ticket, Action::Retire).await.unwrap();
        tokio::fs::write(&path, "later\n").await.unwrap();
        assert!(matches!(
            reload(zed, buffer).await.unwrap().payload,
            Some(proto::envelope::Payload::ReloadBuffersResponse(_))
        ));
        assert_native_mirror(zed, buffer, "later\n").await;
        zed.close_buffer(buffer).unwrap();
    }
    // Initial open was regular; replacing its path may not weaken actual reads.
    let path = workspace.join("reload-link.txt");
    let retained = workspace.join("reload-original.txt");
    tokio::fs::write(&path, ORIGINAL).await.unwrap();
    let (buffer, _) = zed
        .open_buffer(worktree, Path::new("reload-link.txt"))
        .await
        .unwrap();
    let version = zed.diagnostics.lock().unwrap().version(buffer).unwrap();
    tokio::fs::rename(&path, &retained).await.unwrap();
    tokio::fs::symlink(&retained, &path).await.unwrap();
    assert!(reload(zed, buffer).await.is_err());
    assert_native_mirror(zed, buffer, ORIGINAL).await;
    assert_eq!(
        vector(&zed.diagnostics.lock().unwrap().version(buffer).unwrap()),
        vector(&version)
    );
    assert_eq!(
        tokio::fs::read_to_string(&retained).await.unwrap(),
        ORIGINAL
    );
    zed.close_buffer(buffer).unwrap();
    zed.sync.probe(zed).await.unwrap();
    println!(
        "native reload: raw/decoded/binary/symlink refusal, unchanged original vector/text, completed-task cleanup and separate later invocation passed"
    );
}
