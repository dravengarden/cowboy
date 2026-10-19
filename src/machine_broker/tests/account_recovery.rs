use super::*;
use crate::cgroup::Containment;
use tokio::io::AsyncWriteExt as _;

struct Fixture {
    broker: Arc<Broker>,
    launch: StartSession,
    launch_id: uuid::Uuid,
    worker_rx: mpsc::UnboundedReceiver<Frame>,
    guard: Containment,
    child: tokio::process::Child,
}

fn fixture(root: &Path, command: &mut Command) -> Fixture {
    let (mut broker, mut launch, worker_rx) = reconnecting_worker_fixture();
    broker.args.provider_store = Arc::new(
        crate::machine_plugins::MachinePluginStore::new(
            root,
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap(),
    );
    launch.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    launch.provider = "claude-code".into();
    launch.provider_version = "fixture".into();
    launch.provider_generation_digest = "digest".into();
    launch.provider_auth_generation = Some(1);
    broker
        .sessions
        .lock()
        .insert(launch.session_id.clone(), launch.clone());
    let guard = Containment::prepare(command).unwrap();
    let launch_id = broker
        .args
        .provider_store
        .begin_contained_account_worker(&launch, Some(guard.identity()))
        .unwrap();
    let child = command.kill_on_drop(true).spawn().unwrap();
    Fixture {
        broker: Arc::new(broker),
        launch,
        launch_id,
        worker_rx,
        guard,
        child,
    }
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn named_direct_stop_preserves_graceful_worker_exit() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("flushed");
    let mut command = Command::new("sh");
    command
        .args([
            "-ec",
            "read line; test \"$line\" = stop; printf done > \"$1\"",
            "worker",
        ])
        .arg(&marker)
        .stdin(std::process::Stdio::piped())
        .process_group(0);
    let Fixture {
        broker,
        launch,
        launch_id,
        mut worker_rx,
        guard,
        mut child,
    } = fixture(temp.path(), &mut command);
    broker
        .direct_worker_pids
        .lock()
        .insert(launch.session_id.clone(), child.id().unwrap());
    let owner = Arc::clone(&broker);
    let reaper = tokio::spawn(async move {
        loop {
            if matches!(
                worker_rx.recv().await,
                Some(Frame::WorkerCommand {
                    command: WorkerCommand::Stop { .. },
                    ..
                })
            ) {
                break;
            }
        }
        // A queued Stop must be processed before containment sends SIGKILL.
        tokio::time::sleep(Duration::from_millis(50)).await;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"stop\n")
            .await
            .unwrap();
        assert!(child.wait().await.unwrap().success());
        guard.finish().await.unwrap();
        owner
            .args
            .provider_store
            .record_account_worker_exit(&launch.session_id, &launch.provider_account_id, launch_id)
            .unwrap();
        owner.direct_worker_pids.lock().remove(&launch.session_id);
    });
    assert!(broker.force_recycle_failed_start("sess-1").await);
    reaper.await.unwrap();
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "done");
}

#[tokio::test]
#[ignore = "requires delegated writable cgroup v2"]
async fn named_direct_stop_recovers_without_a_session_or_pid_map() {
    for startup in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut command = Command::new("sleep");
        command.arg("60");
        let Fixture {
            broker: original,
            launch,
            guard,
            mut child,
            ..
        } = fixture(temp.path(), &mut command);
        let broker = Arc::new(Broker::new(original.args.clone()));
        drop(original);
        assert!(broker.sessions.lock().is_empty());
        assert!(broker.direct_worker_pids.lock().is_empty());
        let (tx, mut rx) = mpsc::unbounded_channel();
        broker.install_controller(tx);
        while rx.try_recv().is_ok() {}
        let wrong = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        broker
            .quiesce_account_session("sess-1".into(), "wrong".into(), wrong)
            .await;
        assert!(matches!(
            rx.recv().await,
            Some(Frame::CommandAck {
                accepted: false,
                ..
            })
        ));
        assert!(child.try_wait().unwrap().is_none());
        assert!(
            broker
                .args
                .provider_store
                .account_stops()
                .unwrap()
                .is_empty()
        );
        if startup {
            broker
                .args
                .provider_store
                .stop_account_session("sess-1", &launch.provider_account_id)
                .unwrap();
            recover_account_stops(
                Arc::clone(&broker),
                broker.args.provider_store.account_stops().unwrap(),
            )
            .await;
        } else {
            broker
                .quiesce_account_session(
                    "sess-1".into(),
                    "stop".into(),
                    launch.provider_account_id.clone(),
                )
                .await;
            assert!(matches!(
                rx.recv().await,
                Some(Frame::CommandAck { accepted: true, .. })
            ));
        }
        assert!(
            broker
                .args
                .provider_store
                .account_worker_exit("sess-1")
                .unwrap()
                .unwrap()
                .tree_exited
        );
        assert!(broker.account_session_stopped("sess-1"));
        assert!(!child.wait().await.unwrap().success());
        guard.finish().await.unwrap();
        recover_account_stops(
            Arc::clone(&broker),
            broker.args.provider_store.account_stops().unwrap(),
        )
        .await;
        assert!(broker.cancelled_sessions.lock().contains("sess-1"));
    }
}
