use super::*;
use crate::managed_calls::{host::CallHost, protocol::Grant};

#[tokio::test]
async fn duplicate_start_close_and_root_replacement_preserve_original_lifetimes() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), "before").unwrap();
    let config = WorkspaceConfig::new(
        state.path().join("absent.json"),
        vec![format!("test={}", root.display())],
        None,
    )
    .unwrap();
    let roots = crate::session_workspace::WorktreeRoots::single(state.path().join("worktrees"));
    let calls_root = state.path().join("calls");
    std::fs::create_dir(&calls_root).unwrap();
    let host = CallHost::new(&calls_root).unwrap();
    let grant = Grant {
        grant_id: "grant".into(),
        parent_session_id: "parent".into(),
        agent_api: true,
        agent_actions: false,
    };
    host.install(&grant).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let _connection = host.attach(tx);
    let live_grant = Arc::new(host.live_grant("parent", "grant").unwrap());
    let root_text = root.to_str().unwrap();
    let key = Continuation {
        id: "a".repeat(32),
        parent_session_id: "parent".into(),
        grant_id: "grant".into(),
        root: root_text.into(),
        root_incarnation: config.session_identities.lock().observe(root_text).unwrap(),
    };
    let invoke = |request| {
        handle(
            serde_json::to_value(request).unwrap(),
            &config,
            &roots,
            Some(&live_grant),
        )
    };
    invoke(Request::Start {
        continuation: key.clone(),
    })
    .await
    .unwrap();
    let first = invoke(Request::Read {
        continuation: key.clone(),
        cursor: None,
    })
    .await
    .unwrap();
    invoke(Request::Start {
        continuation: key.clone(),
    })
    .await
    .unwrap();
    let second = invoke(Request::Read {
        continuation: key.clone(),
        cursor: None,
    })
    .await
    .unwrap();
    assert_eq!(first["head"]["instance"], second["head"]["instance"]);
    assert!(first["head"]["instance"].as_str().is_some());
    let mut wrong = key.clone();
    wrong.parent_session_id = "other".into();
    assert!(
        invoke(Request::Close {
            continuation: wrong
        })
        .await
        .is_err()
    );
    std::fs::rename(&root, state.path().join("original")).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert!(
        invoke(Request::Read {
            continuation: key.clone(),
            cursor: None
        })
        .await
        .is_err()
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let close = invoke(Request::Close {
            continuation: key.clone(),
        })
        .await
        .unwrap();
        if close["state"] == "closed" {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        invoke(Request::Start {
            continuation: key.clone()
        })
        .await
        .is_err()
    );
    let mut delayed = key;
    delayed.id = "c".repeat(32);
    delayed.root_incarnation = config.session_identities.lock().observe(root_text).unwrap();
    let payload = serde_json::to_value(Request::Start {
        continuation: delayed,
    })
    .unwrap();
    let captured = capture(&payload, Some(&host)).unwrap();
    let (tx, _new_rx) = tokio::sync::mpsc::unbounded_channel();
    let _new_connection = host.attach(tx);
    assert!(
        handle(payload, &config, &roots, Some(&captured))
            .await
            .is_err()
    );
    assert_eq!(config.observers.entries.lock().len(), 1);

    // Missing pathname must retire the pin even if the same inode returns.
    let mut disappeared = config
        .observers
        .entries
        .lock()
        .values()
        .next()
        .unwrap()
        .continuation
        .clone();
    disappeared.root_incarnation = config.session_identities.lock().observe(root_text).unwrap();
    let original = pin(&disappeared, &config, &roots).unwrap();
    std::fs::rename(&root, state.path().join("temporarily-away")).unwrap();
    assert!(pin(&disappeared, &config, &roots).is_err());
    std::fs::rename(state.path().join("temporarily-away"), &root).unwrap();
    assert!(!(original.current)());
    assert!(pin(&disappeared, &config, &roots).is_err());
}

#[test]
fn receipt_conflicts_and_refusals_never_relaunch_or_evict() {
    let registry = Registry::default();
    let mut key = Continuation {
        id: "a".repeat(32),
        parent_session_id: "parent".into(),
        grant_id: "grant".into(),
        root: "/root".into(),
        root_incarnation: "b".repeat(32),
    };
    assert!(
        registry
            .access(&key, false, || anyhow::bail!("launch refused"))
            .is_err()
    );
    assert!(
        registry
            .access(&key, false, || panic!("must not retry"))
            .unwrap()
            .is_none()
    );
    key.grant_id = "different".into();
    assert!(registry.access(&key, true, || unreachable!()).is_err());
    for index in 1..MAX_RECEIPTS {
        key.id = format!("{index:032x}");
        assert!(
            registry
                .access(&key, true, || unreachable!())
                .unwrap()
                .is_none()
        );
    }
    key.id = "c".repeat(32);
    assert!(
        registry
            .access(&key, false, || panic!("over capacity"))
            .is_err()
    );
}
