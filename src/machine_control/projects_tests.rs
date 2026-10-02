use super::*;
use crate::machine_protocol::projects::Request;

#[tokio::test]
async fn projects_require_capable_peer_and_exact_site_before_dispatch() {
    let control = MachineControl::default();
    let (tx, mut commands) = mpsc::unbounded_channel();
    control.install("target".into(), "epoch".into(), false, 23, tx);
    assert!(
        control
            .project_request("target", Request::List)
            .await
            .is_err()
    );
    assert!(commands.try_recv().is_err());
    let (tx, mut commands) = mpsc::unbounded_channel();
    control.install("target".into(), "epoch".into(), false, 24, tx);
    for (service, machine) in [("foreign", "target"), ("service-test", "foreign")] {
        assert!(
            control
                .send(
                    "target",
                    MachineCommand::Projects {
                        request_id: "request".into(),
                        service_id: service.into(),
                        machine_id: machine.into(),
                        request: Request::List,
                    }
                )
                .is_err()
        );
    }
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn project_reply_from_replaced_connection_cannot_complete_or_replay() {
    let control = Arc::new(MachineControl::default());
    let (tx, mut commands) = mpsc::unbounded_channel();
    let old = control.install("target".into(), "epoch".into(), false, 24, tx);
    let caller = Arc::clone(&control);
    let request =
        tokio::spawn(async move { caller.project_request("target", Request::List).await });
    let MachineCommand::Projects { request_id, .. } = commands.recv().await.unwrap() else {
        panic!("project command")
    };
    let (tx, mut replacement) = mpsc::unbounded_channel();
    control.install("target".into(), "epoch".into(), false, 24, tx);
    control.record_remote(
        &old,
        MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(serde_json::json!({"revision":"old"})),
            detail: None,
            refusal: None,
        },
    );
    assert!(request.await.unwrap().is_err());
    assert!(replacement.try_recv().is_err());
}
