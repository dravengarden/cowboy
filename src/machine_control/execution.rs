use super::{MachineControl, Reply, ReplyKind, RequestBinding};
use crate::machine_protocol::{
    MachineCommand,
    execution::{Action, Request, Response},
};

impl MachineControl {
    pub(crate) async fn execution_request(
        &self,
        machine_id: &str,
        request: Request,
    ) -> Result<Response, String> {
        let timeout = if matches!(request.action, Action::Prepare { .. }) {
            super::WORKSPACE_ADAPTER_TIMEOUT + std::time::Duration::from_secs(30)
        } else {
            std::time::Duration::from_secs(30)
        };
        let id = self.request_id("execution")?;
        let connection = self.operation_connection(machine_id)?;
        let (response, _pending) = self.begin_request(
            machine_id,
            &id,
            MachineCommand::Execution {
                request_id: id.clone(),
                request: Box::new(request),
            },
            ReplyKind::Execution,
            Some(RequestBinding::Connection(&connection)),
        )?;
        let response = tokio::time::timeout(timeout, response).await;
        if !self.is_current(&connection) {
            return Err("Execution Machine connection changed; observe the original operation before proceeding".to_owned());
        }
        match response {
            Ok(Ok(Reply::Execution(response))) => Ok(*response),
            _ => Err(
                "Execution response unavailable; observe the original operation before proceeding"
                    .to_owned(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::MachineEvent;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn request(service: &str, machine: &str) -> Request {
        Request {
            service_id: service.into(),
            machine_id: machine.into(),
            action: Action::Inventory,
        }
    }

    #[tokio::test]
    async fn execution_requires_exact_site_and_capable_peer_before_dispatch() {
        for (protocol, service, machine) in [
            (22, "service-test", "target"),
            (23, "foreign", "target"),
            (23, "service-test", "other"),
        ] {
            let control = MachineControl::default();
            let (tx, mut commands) = mpsc::unbounded_channel();
            control.install("target".into(), "epoch".into(), false, protocol, tx);
            assert!(
                control
                    .execution_request("target", request(service, machine))
                    .await
                    .is_err()
            );
            assert!(commands.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn execution_replies_are_typed_private_and_not_inventory_history() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut commands) = mpsc::unbounded_channel();
        let connection = control.install("target".into(), "epoch".into(), false, 23, tx);
        let caller = Arc::clone(&control);
        let pending = tokio::spawn(async move {
            caller
                .execution_request("target", request("service-test", "target"))
                .await
        });
        let MachineCommand::Execution { request_id, .. } = commands.recv().await.unwrap() else {
            panic!("execution command")
        };
        control.record_remote(
            &connection,
            MachineEvent::AdapterResponse {
                request_id: request_id.clone(),
                accepted: true,
                payload: Some(serde_json::json!({"private": "wrong-kind"})),
                detail: None,
                refusal: None,
            },
        );
        assert!(!pending.is_finished());
        let response = Response::Call {
            response: crate::execution_protocol::Response::Operation {
                outcome: crate::execution_protocol::Outcome::Completed {
                    reply: serde_json::json!({"result": "source-sentinel"}),
                },
            },
        };
        control.record_remote(
            &connection,
            MachineEvent::ExecutionResponse {
                request_id,
                response: Box::new(response.clone()),
            },
        );
        assert_eq!(pending.await.unwrap().unwrap(), response);
        assert!(control.live.read().events.is_empty());
        assert!(!format!("{response:?}").contains("source-sentinel"));
    }

    #[tokio::test]
    async fn execution_receipt_cannot_cross_machine_connection_replacement() {
        let control = Arc::new(MachineControl::default());
        let (tx, mut commands) = mpsc::unbounded_channel();
        let old = control.install("target".into(), "epoch".into(), false, 23, tx);
        let caller = Arc::clone(&control);
        let pending = tokio::spawn(async move {
            caller
                .execution_request("target", request("service-test", "target"))
                .await
        });
        let MachineCommand::Execution { request_id, .. } = commands.recv().await.unwrap() else {
            panic!("execution command")
        };
        let (tx, mut new_commands) = mpsc::unbounded_channel();
        control.install("target".into(), "epoch".into(), false, 23, tx);
        control.record_remote(
            &old,
            MachineEvent::ExecutionResponse {
                request_id,
                response: Box::new(Response::Inventory { executor: None }),
            },
        );
        assert!(pending.await.unwrap().is_err());
        assert!(new_commands.try_recv().is_err());
    }
}
