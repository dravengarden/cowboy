//! Original native Open completed, but the adapter never committed its owner.
use super::*;
use crate::legacy_release::{Action, State};

pub(super) async fn exercise(server: &Path, root: &Path) {
    let zed = Arc::new(
        ZedRuntime::start_with_disconnect(server, &root.join("legacy-recovery-state"), || {})
            .await
            .unwrap(),
    );
    let workspace = root.join("legacy-recovery-worktree");
    tokio::fs::create_dir(&workspace).await.unwrap();
    tokio::fs::write(workspace.join("file.txt"), "original🙂\n")
        .await
        .unwrap();
    let (worktree, _) = zed.open_worktree(&workspace, true).await.unwrap();
    let buffers: crate::Buffers = Arc::default();
    let request = (
        workspace.clone(),
        PathBuf::from("file.txt"),
        "unfinished".to_owned(),
    );
    let capture = {
        let mut registry = buffers.legacy.lock().await;
        registry
            .pin(request.clone(), (workspace.clone(), "file.txt".into()))
            .unwrap();
        registry.capture(request.clone()).unwrap()
    };
    let attempt = buffers.native_open.begin().unwrap();
    attempt.track(&capture.recovery);
    let (orphan, _) = zed
        .open_buffer_tracked(worktree, Path::new("file.txt"), |id| {
            attempt.dispatching(id);
        })
        .await
        .unwrap();
    assert!(orphan > 0);
    drop(attempt); // Simulates interruption after native completion, before owner commit.
    let prepared = crate::legacy_release::respond(
        crate::Request::PrepareLegacyRelease {
            worktree: request.0,
            path: request.1,
            lease_id: request.2,
        },
        &buffers,
        Some(&zed),
    )
    .await
    .unwrap();
    let crate::Response::LegacyRelease {
        release,
        state: State::Prepared,
        ..
    } = prepared
    else {
        panic!("not prepared")
    };
    assert_native_mirror(&zed, orphan, "original🙂\n").await;
    for action in [Action::Execute, Action::Query, Action::Execute] {
        let result = crate::legacy_release::respond(
            crate::Request::LegacyRelease {
                release: release.clone(),
                action,
            },
            &buffers,
            Some(&zed),
        )
        .await
        .unwrap();
        assert!(matches!(
            result,
            crate::Response::LegacyRelease {
                state: State::Released,
                ..
            }
        ));
    }
    assert!(buffers.native_open.check().is_ok());
    assert!(zed.diagnostics.lock().unwrap().revision(orphan).is_err());
    // A new initial share proves the original peer holding really ended.
    zed.open_buffer(worktree, Path::new("file.txt"))
        .await
        .expect("original orphan was not released");
    stop(&zed).await;
    println!(
        "legacy unfinished Open: original native acquisition released through prepared reference, no Open replay, fresh share after release"
    );
}
