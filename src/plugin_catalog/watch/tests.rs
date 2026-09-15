use super::*;
use std::fs;

mod lifecycle;

#[test]
fn ambiguous_events_and_backend_errors_wake_but_reads_do_not() {
    assert!(wakes_reader(&Err(notify::Error::generic(
        "synthetic overflow"
    ))));
    for kind in [EventKind::Any, EventKind::Other] {
        assert!(wakes_reader(&Ok(notify::Event::new(kind))));
    }
    assert!(!wakes_reader(&Ok(notify::Event::new(EventKind::Access(
        notify::event::AccessKind::Read,
    )))));
    assert!(wakes_reader(&Ok(notify::Event::new(EventKind::Access(
        notify::event::AccessKind::Read,
    ))
    .set_flag(notify::event::Flag::Rescan))));
}

#[tokio::test(start_paused = true)]
async fn regression_sustained_events_cannot_starve_refresh() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let writes = tokio::spawn(async move {
        for _ in 0..64 {
            if sender.send(()).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    let settled = tokio::time::timeout(Duration::from_secs(4), settle(&mut receiver)).await;
    writes.abort();
    let _ = writes.await;
    assert!(
        settled.is_ok(),
        "a continuous write stream must still permit a refresh"
    );
}

#[tokio::test]
async fn regression_existing_publisher_key_directory_is_observed() {
    let root = tempfile::tempdir().unwrap();
    let publishers = root.path().join("trusted-publishers");
    fs::create_dir(&publishers).unwrap();
    let (_watchers, mut receiver) = watch_roots(&[root.path().to_owned()]).unwrap();
    fs::write(publishers.join("fixture.pub"), b"synthetic public key").unwrap();
    tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("public trust changes must wake the Catalog reader")
        .expect("watch remains connected");
}

/// Publication writes several files per release. The loop must observe the
/// burst, not one reload per byte.
#[tokio::test(start_paused = true)]
async fn a_publication_burst_settles_into_one_reload() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(1);
    for _ in 0..5 {
        let _ = sender.try_send(());
    }
    assert!(receiver.recv().await.is_some(), "the burst wakes the loop");
    settle(&mut receiver).await;
    // Everything the burst queued was absorbed by the settle window, so a
    // second reload has nothing left to consume.
    assert!(
        tokio::time::timeout(SETTLE, receiver.recv()).await.is_err(),
        "the settled burst must not schedule a second reload"
    );
    drop(sender);
}

/// A closed channel ends the watch instead of spinning.
#[tokio::test(start_paused = true)]
async fn settling_returns_when_every_watcher_is_gone() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(1);
    drop(sender);
    settle(&mut receiver).await;
    assert!(receiver.recv().await.is_none());
}

#[test]
fn an_absent_root_is_skipped_without_failing_the_watch() {
    let root = tempfile::tempdir().unwrap();
    let present = root.path().join("catalog");
    fs::create_dir_all(&present).unwrap();
    let (watchers, _receiver) =
        watch_roots(&[present, root.path().join("legacy-that-never-existed")]).unwrap();
    assert_eq!(watchers.len(), 1, "only the existing root is watched");
}

/// The real notify backend must wake the loop when a release lands. This
/// exercises the actual filesystem path, not a simulated event.
#[tokio::test]
async fn a_published_file_wakes_the_loop() {
    let root = tempfile::tempdir().unwrap();
    let catalog = root.path().join("catalog");
    fs::create_dir_all(&catalog).unwrap();
    let (_watchers, mut receiver) = watch_roots(std::slice::from_ref(&catalog)).unwrap();

    // The envelope is the commit marker publication links last.
    fs::write(catalog.join("fixture-1.0.0.release.json"), b"{}").unwrap();

    tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("a Catalog write must wake the loop")
        .expect("the watcher keeps the channel open");
}
