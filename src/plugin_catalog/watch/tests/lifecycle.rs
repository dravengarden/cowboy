use super::super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::oneshot;

fn observe<P, A>(
    probe: impl FnMut() -> P + Send + 'static,
    refresh: impl FnMut() -> A + Send + 'static,
) -> (CatalogWatcher, watch::Sender<bool>, mpsc::Sender<()>)
where
    P: Future<Output = Result<sources::Hint>> + Send + 'static,
    A: Future<Output = Result<usize>> + Send + 'static,
{
    let (stop, owner) = watch::channel(false);
    let (service, shutdown) = watch::channel(false);
    let (hints, receiver) = mpsc::channel(1);
    let task = tokio::spawn(run(
        receiver,
        Stop {
            owner,
            service: shutdown,
        },
        probe,
        refresh,
    ));
    (
        CatalogWatcher {
            watchers: vec![],
            stop,
            task: Some(task),
        },
        service,
        hints,
    )
}

fn hints() -> [sources::Hint; 2] {
    let root = tempfile::tempdir().unwrap();
    [
        sources::sample(&[]).unwrap(),
        sources::sample(&[root.path().join("absent")]).unwrap(),
    ]
}

fn counter(
    result: bool,
) -> (
    Arc<AtomicUsize>,
    impl FnMut() -> std::future::Ready<Result<usize>>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    (calls, move || {
        count.fetch_add(1, Ordering::SeqCst);
        std::future::ready(if result {
            Ok(0)
        } else {
            Err(anyhow::anyhow!("synthetic failure"))
        })
    })
}

async fn turn() {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn stop_before_start_blocks_both_probe_and_refresh() {
    let (_catalog, service, _hints) = observe(
        || async { panic!("must not probe") },
        || async { panic!("must not refresh") },
    );
    service.send_replace(true);
    _catalog.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn owner_drop_cancels_queued_settling_before_first_attempt() {
    let (mut catalog, _service, events) = observe(
        || async { panic!("must not probe") },
        || async { panic!("must not refresh") },
    );
    events.try_send(()).unwrap();
    turn().await;
    let join = catalog.task.take().unwrap();
    drop(catalog);
    tokio::time::timeout(SETTLE, join).await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn service_stop_or_sender_loss_ends_observation_without_dropping_owner() {
    for disconnect in [false, true] {
        let hint = hints()[0];
        let (calls, refresh) = counter(true);
        let (mut catalog, service, events) = observe(move || async move { Ok(hint) }, refresh);
        turn().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        if !disconnect {
            service.send_replace(true);
        }
        drop(service);
        events.try_send(()).unwrap();
        catalog.task.take().unwrap().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        catalog.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn owner_drop_during_probe_prevents_following_refresh() {
    let hint = hints()[0];
    let (started, wait_started) = oneshot::channel();
    let (finish, wait_finish) = oneshot::channel();
    let mut probe = Some((started, wait_finish));
    let (mut catalog, _service, _hints) = observe(
        move || {
            let (started, wait_finish) = probe.take().expect("one read-only probe");
            async move {
                started.send(()).unwrap();
                wait_finish.await.unwrap();
                Ok(hint)
            }
        },
        || async { panic!("stopped probe cannot authorize refresh") },
    );
    wait_started.await.unwrap();
    let join = catalog.task.take().unwrap();
    drop(catalog);
    finish.send(()).unwrap();
    join.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_during_retry_does_not_wait_for_or_run_the_next_attempt() {
    let hint = hints()[0];
    let (calls, refresh) = counter(false);
    let (catalog, _service, _hints) = observe(move || async move { Ok(hint) }, refresh);
    turn().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_millis(100), catalog.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_drains_an_active_attempt_without_aborting_or_repeating_it() {
    let hint = hints()[0];
    let (started, wait_started) = oneshot::channel();
    let (finish, wait_finish) = oneshot::channel();
    let mut attempt = Some((started, wait_finish));
    let completed = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&completed);
    let (catalog, _service, hints) = observe(
        move || async move { Ok(hint) },
        move || {
            let (started, wait_finish) = attempt.take().expect("no second attempt after stop");
            let completed = Arc::clone(&completed);
            async move {
                started.send(()).unwrap();
                wait_finish.await.unwrap();
                completed.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            }
        },
    );
    wait_started.await.unwrap();
    hints.try_send(()).unwrap();
    let draining = tokio::spawn(catalog.shutdown());
    turn().await;
    assert!(
        !draining.is_finished(),
        "shutdown must wait for the admitted attempt"
    );
    assert_eq!(observed.load(Ordering::SeqCst), 0);
    finish.send(()).unwrap();
    draining.await.unwrap().unwrap();
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn unchanged_hints_skip_rebuilds_and_closed_notifications_keep_polling() {
    let values = hints();
    let selected = Arc::new(AtomicUsize::new(0));
    let current = Arc::clone(&selected);
    let probes = Arc::new(AtomicUsize::new(0));
    let scans = Arc::clone(&probes);
    let (calls, refresh) = counter(true);
    let (catalog, _service, events) = observe(
        move || {
            scans.fetch_add(1, Ordering::SeqCst);
            let hint = values[current.load(Ordering::SeqCst)];
            async move { Ok(hint) }
        },
        refresh,
    );
    turn().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    events.try_send(()).unwrap();
    turn().await;
    tokio::time::advance(SETTLE).await;
    turn().await;
    tokio::time::advance(RECHECK).await;
    turn().await;
    assert!(probes.load(Ordering::SeqCst) >= 3);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(events);
    turn().await;
    let idle = probes.load(Ordering::SeqCst);
    turn().await;
    assert_eq!(
        probes.load(Ordering::SeqCst),
        idle,
        "no closed-channel spin"
    );
    selected.store(1, Ordering::SeqCst);
    tokio::time::advance(RECHECK).await;
    turn().await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "poll survives lost native watcher"
    );
    catalog.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn exhausted_retries_and_scan_failures_recover_without_a_new_event() {
    let hint = hints()[0];
    let mut probe_failed = false;
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let (catalog, _service, events) = observe(
        move || {
            let failed = std::mem::replace(&mut probe_failed, true);
            async move {
                if failed {
                    Ok(hint)
                } else {
                    anyhow::bail!("synthetic source failure")
                }
            }
        },
        move || {
            let count = counted.fetch_add(1, Ordering::SeqCst);
            async move {
                if count < 4 {
                    anyhow::bail!("synthetic refresh failure")
                } else {
                    Ok(0)
                }
            }
        },
    );
    drop(events);
    turn().await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    tokio::time::advance(RECHECK).await;
    turn().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    for delay in RETRY_BACKOFF {
        tokio::time::advance(delay).await;
        turn().await;
    }
    // The interval may already be due after the 60-second final backoff.
    tokio::time::advance(RECHECK).await;
    turn().await;
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    catalog.shutdown().await.unwrap();
}
