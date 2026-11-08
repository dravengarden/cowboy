use super::*;
use crate::code_observer::{Coverage, PageState};
use cowboy::machine_protocol::MachineEvent;
use cowboy::managed_calls::{
    host::{CallHost, Uplink},
    protocol::Grant,
};

struct Fixture {
    _state: tempfile::TempDir,
    root: tempfile::TempDir,
    host: Arc<CallHost>,
    grant: Grant,
    connection: Uplink,
    _receiver: tokio::sync::mpsc::UnboundedReceiver<MachineEvent>,
}

impl Fixture {
    fn new() -> Self {
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), "before").unwrap();
        let host = CallHost::new(state.path()).unwrap();
        let grant = Grant {
            grant_id: "observer-grant".into(),
            parent_session_id: "parent".into(),
            agent_api: true,
            agent_actions: false,
            mcp: false,
        };
        host.install(&grant).unwrap();
        let (tx, receiver) = tokio::sync::mpsc::unbounded_channel();
        let connection = host.attach(tx);
        Self {
            _state: state,
            root,
            host,
            grant,
            connection,
            _receiver: receiver,
        }
    }

    fn worker(&self, capacity: &Capacity) -> Result<Worker> {
        self.with_fence(capacity, Arc::new(|| true))
    }

    fn with_fence(
        &self,
        capacity: &Capacity,
        current: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<Worker> {
        Worker::start(
            File::open(self.root.path()).unwrap(),
            self.root.path().into(),
            Arc::new(self.host.live_grant("parent", "observer-grant").unwrap()),
            current,
            capacity,
        )
    }
}

#[tokio::test]
async fn observes_kernel_changes_and_close_confirms_capacity_release() {
    let fixture = Fixture::new();
    let capacity = Capacity::default();
    let worker = fixture.worker(&capacity).unwrap();
    let initial = worker.read(None).await.unwrap();
    assert_eq!(initial.state, PageState::Reset);
    assert!(initial.scan_at.is_some());
    std::fs::write(fixture.root.path().join("file"), "after").unwrap();
    let changed = worker.read(Some(initial.head)).await.unwrap();
    assert_eq!(changed.coverage, Coverage::EventStreamOnly);
    assert_eq!(changed.state, PageState::Changes);
    assert!(
        changed
            .events
            .iter()
            .any(|event| event.paths.iter().any(|path| path == "file"))
    );
    worker.close().await.unwrap();
    assert_eq!(capacity.0.available_permits(), 2);
}

#[tokio::test]
async fn revocation_and_same_id_reinstall_never_restore_an_old_worker() {
    let fixture = Fixture::new();
    let capacity = Capacity::default();
    let worker = fixture.worker(&capacity).unwrap();
    worker.read(None).await.unwrap();
    fixture.host.revoke(&fixture.grant);
    fixture.host.install(&fixture.grant).unwrap();
    assert!(worker.read(None).await.is_err());
    tokio::time::timeout(Duration::from_secs(2), worker.close())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capacity.0.available_permits(), 2);
}

#[tokio::test]
async fn disconnect_and_local_scope_loss_refuse_reads() {
    let fixture = Fixture::new();
    let capacity = Capacity::default();
    let allowed = Arc::new(AtomicBool::new(true));
    let check = Arc::clone(&allowed);
    let worker = fixture
        .with_fence(&capacity, Arc::new(move || check.load(Ordering::Acquire)))
        .unwrap();
    worker.read(None).await.unwrap();
    allowed.store(false, Ordering::Release);
    assert!(worker.read(None).await.is_err());
    worker.close().await.unwrap();
    let worker = fixture.worker(&capacity).unwrap();
    worker.read(None).await.unwrap();
    drop(fixture.connection);
    assert!(worker.read(None).await.is_err());
    worker.close().await.unwrap();
}

#[tokio::test]
async fn capacity_is_reserved_before_launch_and_released_after_drop() {
    let fixture = Fixture::new();
    let capacity = Capacity::default();
    let first = fixture.worker(&capacity).unwrap();
    let second = fixture.worker(&capacity).unwrap();
    assert!(fixture.worker(&capacity).is_err());
    drop(first);
    tokio::time::timeout(Duration::from_secs(2), async {
        let permit = capacity.0.acquire().await.unwrap();
        drop(permit);
    })
    .await
    .unwrap();
    let replacement = fixture.worker(&capacity).unwrap();
    replacement.close().await.unwrap();
    second.close().await.unwrap();
    assert_eq!(capacity.0.available_permits(), 2);
}

#[tokio::test]
async fn timed_out_close_cannot_release_a_still_running_native_thread() {
    let fixture = Fixture::new();
    let capacity = Capacity::default();
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    struct Unblock(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl Drop for Unblock {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    let unblock = Unblock(Arc::clone(&gate));
    let (entered, waiting) = oneshot::channel();
    let entered = parking_lot::Mutex::new(Some(entered));
    let first = AtomicBool::new(true);
    let worker = fixture
        .with_fence(
            &capacity,
            Arc::new(move || {
                if first.swap(false, Ordering::AcqRel) {
                    return true; // Admission is immediate; block only the native thread.
                }
                if let Some(entered) = entered.lock().take() {
                    let _ = entered.send(());
                }
                let (lock, changed) = &*gate;
                let _guard = changed
                    .wait_while(lock.lock().unwrap(), |open| !*open)
                    .unwrap();
                true
            }),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), worker.close())
            .await
            .is_err()
    );
    assert_eq!(capacity.0.available_permits(), 1);
    drop(unblock);
    let permits = tokio::time::timeout(Duration::from_secs(2), capacity.0.acquire_many(2))
        .await
        .unwrap()
        .unwrap();
    drop(permits);
    assert_eq!(capacity.0.available_permits(), 2);
}

#[test]
fn runtime_shutdown_cancels_even_an_unpolled_monitor() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let capacity = Capacity::default();
    // No await/yield after start: the single-thread runtime never polls the
    // cancellation monitor before shutdown. Keep the grant/channel alive.
    let (fixture, worker) = runtime.block_on(async {
        let fixture = Fixture::new();
        let worker = fixture.worker(&capacity).unwrap();
        (fixture, worker)
    });
    drop(runtime);
    assert!(!worker.is_current());
    let deadline = Instant::now() + Duration::from_secs(2);
    while capacity.0.available_permits() != 2 {
        assert!(
            Instant::now() < deadline,
            "native worker survived runtime shutdown"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(worker);
    drop(fixture);
}
