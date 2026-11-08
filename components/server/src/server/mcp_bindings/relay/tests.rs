use super::super::tests::{binding, fixture, update};
use super::*;
use tokio::sync::mpsc;

fn connect(
    control: &MachineControl,
    machine: &str,
) -> (ConnectionToken, mpsc::UnboundedReceiver<MachineCommand>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        control.install(machine.into(), "same-epoch".into(), false, 42, tx),
        rx,
    )
}

#[tokio::test]
async fn relay_scope_rechecks_epoch_exact_grant_and_cancellation() {
    let (hub, meta, policies, store, _root) = fixture().await;
    let saved = policies
        .replace(&hub, &store, &meta, update(None, vec![binding()]))
        .await
        .unwrap();
    let control = Arc::new(MachineControl::default());
    let (runtime, _runtime_rx) = connect(&control, "runtime");
    let (execution, mut commands) = connect(&control, "execution");
    let grant = Grant {
        grant_id: "grant-one".into(),
        parent_session_id: meta.id.clone(),
        agent_api: true,
        agent_actions: true,
        mcp: true,
    };
    let session = Arc::new(Session {
        admission: Mutex::new(
            policies
                .admit(&hub, &meta.id, "runtime", &saved.epoch, "suger")
                .unwrap(),
        ),
        grant: grant.clone(),
        runtime,
        remote: Remote {
            control: Arc::clone(&control),
            connection: execution.clone(),
            handle: format!("mcp-{}", "a".repeat(32)),
            closed: AtomicBool::new(false),
        },
        cancelled: watch::channel(false).0,
        _permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
    });
    let request = RelayRequest {
        binding: "suger".into(),
        epoch: saved.epoch.clone(),
        action: RelayAction::Open,
    };
    assert!(session.owns(&execution, &grant, &request));
    let mut changed = grant.clone();
    changed.grant_id = "replacement".into();
    assert!(!session.owns(&execution, &changed, &request));
    session.scope_current(&policies, &hub, &control).unwrap();
    drop(InFlight {
        state: std::sync::Weak::new(),
        session: Arc::clone(&session),
        handle: "relay-test".into(),
        completed: true,
    });
    assert!(!*session.cancelled.borrow());
    assert!(commands.try_recv().is_err());
    let revoked = policies
        .replace(&hub, &store, &meta, update(Some(saved.epoch), vec![]))
        .await
        .unwrap();
    policies
        .replace(
            &hub,
            &store,
            &meta,
            update(Some(revoked.epoch), vec![binding()]),
        )
        .await
        .unwrap();
    assert!(session.scope_current(&policies, &hub, &control).is_err());
    let mut cancelled = session.cancelled.subscribe();
    let flight = InFlight {
        state: std::sync::Weak::new(),
        session: Arc::clone(&session),
        handle: "relay-test".into(),
        completed: false,
    };
    let mut pending = Box::pin(async {
        let _flight = flight;
        control
            .mcp_request_on_connection(
                &execution,
                TransportRequest::Request {
                    handle: session.remote.handle.clone(),
                    message: json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"suger_read","arguments":{"code":"await new Promise(()=>{});"}}}),
                },
            )
            .await
    });
    assert!(futures::poll!(&mut pending).is_pending());
    assert!(matches!(
        commands.try_recv().unwrap(),
        MachineCommand::McpTransport {
            request: TransportRequest::Request { .. },
            ..
        }
    ));
    drop(pending);
    cancelled.changed().await.unwrap();
    assert!(*cancelled.borrow());
    assert!(
        session.subscribe_cancellation().is_err(),
        "a late subscriber must not wait for a second cancellation"
    );
    assert!(matches!(
        commands.try_recv().unwrap(),
        MachineCommand::McpTransport {
            request: TransportRequest::Close { .. },
            ..
        }
    ));
    session.end();
    assert!(commands.try_recv().is_err(), "close must be sent only once");
}

#[tokio::test]
async fn remote_close_never_moves_to_replacement_connection() {
    let control = Arc::new(MachineControl::default());
    let (old, _old_rx) = connect(&control, "execution");
    let remote = Remote {
        control: Arc::clone(&control),
        connection: old,
        handle: format!("mcp-{}", "a".repeat(32)),
        closed: AtomicBool::new(false),
    };
    let (_replacement, mut rx) = connect(&control, "execution");
    drop(remote);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn replacing_either_connection_ends_the_scope() {
    for changed in ["runtime", "execution"] {
        let (hub, meta, policies, store, _root) = fixture().await;
        let saved = policies
            .replace(&hub, &store, &meta, update(None, vec![binding()]))
            .await
            .unwrap();
        let control = Arc::new(MachineControl::default());
        let (runtime, _runtime_rx) = connect(&control, "runtime");
        let (execution, _execution_rx) = connect(&control, "execution");
        let session = Session {
            admission: Mutex::new(
                policies
                    .admit(&hub, &meta.id, "runtime", &saved.epoch, "suger")
                    .unwrap(),
            ),
            grant: Grant {
                grant_id: "grant-one".into(),
                parent_session_id: meta.id.clone(),
                agent_api: true,
                agent_actions: true,
                mcp: true,
            },
            runtime,
            remote: Remote {
                control: Arc::clone(&control),
                connection: execution,
                handle: format!("mcp-{}", "a".repeat(32)),
                closed: AtomicBool::new(false),
            },
            cancelled: watch::channel(false).0,
            _permit: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
        };
        session.scope_current(&policies, &hub, &control).unwrap();
        let _replacement = connect(&control, changed);
        assert!(
            session.scope_current(&policies, &hub, &control).is_err(),
            "{changed}"
        );
    }
}

#[test]
fn relay_replies_satisfy_the_existing_gateway_envelope() {
    let success = envelope(json!({"handle":"relay-test"}));
    assert_eq!(success["schema"], 1);
    assert_eq!(success["state"], "ok");
    assert_eq!(success["result"]["handle"], "relay-test");
    let error = envelope(failure("mcp_not_admitted"));
    assert_eq!(error["schema"], 1);
    assert_eq!(error["state"], "error");
    assert_eq!(error["error"]["code"], "mcp_not_admitted");
}

#[tokio::test]
async fn renewal_ack_cannot_restore_expired_or_replaced_admission() {
    for cause in ["success", "expiry", "revoke"] {
        let (hub, meta, policies, store, _root) = fixture().await;
        let saved = policies
            .replace(&hub, &store, &meta, update(None, vec![binding()]))
            .await
            .unwrap();
        let mut admission = policies
            .admit(&hub, &meta.id, "runtime", &saved.epoch, "suger")
            .unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(20)).await;
        let candidate = admission.renewal(&policies, &hub).unwrap();
        match cause {
            "expiry" => {
                tokio::time::advance(Duration::from_secs(11)).await;
                assert!(candidate.current(&policies, &hub).is_ok());
                assert!(
                    admission
                        .commit_renewal(candidate, &policies, &hub)
                        .is_err()
                );
                assert!(admission.renewal(&policies, &hub).is_err());
            }
            "revoke" => {
                tokio::time::resume();
                let revoked = policies
                    .replace(&hub, &store, &meta, update(Some(saved.epoch), vec![]))
                    .await
                    .unwrap();
                policies
                    .replace(
                        &hub,
                        &store,
                        &meta,
                        update(Some(revoked.epoch), vec![binding()]),
                    )
                    .await
                    .unwrap();
                assert!(
                    admission
                        .commit_renewal(candidate, &policies, &hub)
                        .is_err()
                );
                continue;
            }
            _ => {
                admission
                    .commit_renewal(candidate, &policies, &hub)
                    .unwrap();
                tokio::time::advance(Duration::from_secs(11)).await;
                admission.current(&policies, &hub).unwrap();
            }
        }
        tokio::time::resume();
    }
}
