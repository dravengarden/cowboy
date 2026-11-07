use super::*;
use sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn language_snapshot_uses_current_mirror_not_open_version_or_disk() {
    let (transport, mut frontier, _) = native_close::tests::fixture();
    let (zed, mut outbound) = fixture_with(transport).await;
    let opened = zed.diagnostics.lock().unwrap().version(7).unwrap();
    edit(&zed);
    let expected = zed
        .diagnostics
        .lock()
        .unwrap()
        .snapshot(7)
        .unwrap()
        .unwrap();
    let expected = serde_json::to_value(expected).unwrap();
    assert_ne!(expected["version"], serde_json::to_value(&opened).unwrap());
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!(
        "cw-language-snapshot-{:032x}",
        u128::from_be_bytes(nonce)
    ));
    std::fs::create_dir(&root).unwrap();
    let root_guard = Root(root.canonicalize().unwrap());
    let root = root_guard.0.clone();
    std::fs::write(root.join("file.txt"), "different disk text").unwrap();
    let buffers = Arc::new(BufferState::default());
    buffers.active.write().await.insert(
        (root.clone(), "file.txt".into()),
        BufferLease {
            lease_ids: HashSet::from([BufferOwner::Legacy("fixture".into())]),
            remote_id: 7,
            version: opened,
            sync: None,
            closing: false,
        },
    );
    std::os::unix::fs::symlink("file.txt", root.join("alias.txt")).unwrap();
    let regular = if cfg!(target_os = "linux") {
        "changed"
    } else {
        "unavailable"
    };
    for (path, source_state) in [("file.txt", regular), ("alias.txt", "unavailable")] {
        let task = {
            let zed = zed.clone();
            let root = root.clone();
            let buffers = buffers.clone();
            tokio::spawn(
                async move { buffer_language(root, path.into(), &buffers, Some(&zed)).await },
            )
        };
        for _ in 0..3 {
            let request = outbound.recv().await.unwrap();
            reply(&zed, request, Vec::new()).await;
        }
        let request = frontier.recv().await.unwrap();
        assert!(matches!(request.payload, Some(Payload::AnalysisRequest(_))));
        let response = wire::CowboyBufferSyncEnvelope {
            responding_to: Some(request.id),
            payload: Some(Payload::AnalysisResponse(
                wire::CowboyAnalysisFrontierResponse {
                    protocol: 1,
                    servers: Vec::new(),
                },
            )),
            ..Default::default()
        };
        assert!(zed.sync.response(request.id, &response.encode_to_vec()));
        let response = serde_json::to_value(task.await.unwrap().unwrap()).unwrap();
        assert_eq!(response["bufferSnapshot"], expected);
        assert_eq!(response["sourceState"], source_state);
        assert_eq!(response["version"], expected["version"]);
        assert_eq!(response["diagnostics"], serde_json::json!([]));
        let mirrored = coordinates::Mirror::new(1, "汉\na🙂z\n").unwrap();
        assert_eq!(
            response["bufferSnapshot"]["content"],
            serde_json::to_value(mirrored.content()).unwrap()
        );
    }
}
