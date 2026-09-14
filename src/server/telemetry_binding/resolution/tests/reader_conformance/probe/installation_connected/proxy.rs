//! A byte-preserving, purpose-closed relay; never fabricates an installation ACK.
use super::*;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallOutcome, InstallStep, InstallTarget, InstallTargetObservation,
};

pub(super) struct Proxy {
    pub address: std::net::SocketAddr,
    record: Arc<parking_lot::Mutex<Record>>,
    task: tokio::task::JoinHandle<()>,
}

struct Record {
    flow: Flow,
    readonly: bool,
    counts: WireCounts,
    failure: Option<Failure>,
    generation: Option<String>,
    configured: bool,
    target_query: Option<(String, String)>,
    target: Option<InstallTarget>,
    steps: Vec<InstallStep>,
}

impl Record {
    fn new(flow: Flow, readonly: bool) -> Self {
        Self {
            flow,
            readonly,
            counts: WireCounts::default(),
            failure: None,
            generation: None,
            configured: false,
            target_query: None,
            target: None,
            steps: Vec::new(),
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    pub async fn start(
        upstream: std::net::SocketAddr,
        flow: Flow,
        readonly: bool,
    ) -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Failure::Setup)?;
        let address = listener.local_addr().map_err(|_| Failure::Setup)?;
        let record = Arc::new(parking_lot::Mutex::new(Record::new(flow, readonly)));
        let observed = record.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let result = tokio::time::timeout(Duration::from_secs(4), async {
                    let config = WebSocketConfig::default()
                        .max_message_size(Some(256 * 1024))
                        .max_frame_size(Some(256 * 1024));
                    let machine = tokio_tungstenite::accept_async_with_config(stream, Some(config))
                        .await
                        .ok()?;
                    let (controller, _) = tokio_tungstenite::connect_async(format!(
                        "ws://{upstream}/api/machine/connect"
                    ))
                    .await
                    .ok()?;
                    Some((machine, controller))
                })
                .await;
                let Ok(Some((mut machine, mut controller))) = result else {
                    continue;
                };
                loop {
                    let (from_machine, message) = tokio::select! {
                        frame = machine.next() => (true, frame),
                        frame = controller.next() => (false, frame),
                    };
                    let Some(Ok(message)) = message else { break };
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                    let action = inspect(&message, from_machine, &mut observed.lock());
                    match action {
                        Err(failure) => {
                            observed.lock().failure = Some(failure);
                            break;
                        }
                        Ok(Action::Drop) => continue,
                        Ok(Action::Disconnect) => break,
                        Ok(Action::Forward) => {}
                    }
                    let sent = if from_machine {
                        tokio::time::timeout(Duration::from_secs(2), controller.send(message)).await
                    } else {
                        tokio::time::timeout(Duration::from_secs(2), machine.send(message)).await
                    };
                    if !matches!(sent, Ok(Ok(()))) {
                        break;
                    }
                }
            }
        });
        Ok(Self {
            address,
            record,
            task,
        })
    }

    pub fn counts(&self) -> Result<WireCounts, Failure> {
        let record = self.record.lock();
        if let Some(failure) = record.failure {
            return Err(failure);
        }
        Ok(record.counts.clone())
    }

    pub fn snapshot(&self) -> WireCounts {
        self.record.lock().counts.clone()
    }

    pub async fn applied(&self) -> Result<(), Failure> {
        tokio::time::timeout(DEADLINE, async {
            while self.counts()?.receipts_observed == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
        .await
        .map_err(|_| Failure::Timeout)?
    }

    pub async fn finish(&mut self) -> Result<(), Failure> {
        self.task.abort();
        match (&mut self.task).await {
            Err(e) if e.is_cancelled() => Ok(()),
            _ => Err(Failure::Cleanup),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Forward,
    Drop,
    Disconnect,
}

fn inspect(message: &Message, from_machine: bool, record: &mut Record) -> Result<Action, Failure> {
    let Message::Text(text) = message else {
        return if matches!(message, Message::Ping(_) | Message::Pong(_)) {
            Ok(Action::Forward)
        } else {
            Err(Failure::FrameDecode)
        };
    };
    if text.len() > 256 * 1024 {
        return Err(Failure::FrameDecode);
    }
    let frame: MachineFrame = serde_json::from_str(text).map_err(|_| Failure::FrameDecode)?;
    match frame {
        MachineFrame::Command { command } if !from_machine => command_frame(command, record),
        MachineFrame::Event { event } if from_machine => event_frame(event, record),
        other => handshake(other, from_machine, record),
    }
}

fn command_frame(command: MachineCommand, record: &mut Record) -> Result<Action, Failure> {
    match command {
        MachineCommand::RefreshInventory { .. } => {}
        MachineCommand::ObservePluginInstallation { request_id, query } if !record.readonly => {
            check(
                query.service_id == SERVICE
                    && query.machine_id == MACHINE
                    && query.plugin_id == "victoria"
                    && record.target_query.is_none()
                    && record.target.is_none()
                    && record.steps.len() < record.flow.attempts()
                    && record.counts.target_queries as usize == record.steps.len(),
            )?;
            let digest = query.digest().map_err(|_| Failure::WrongObservation)?;
            record.target_query = Some((request_id, digest));
            record.counts.target_queries += 1;
        }
        MachineCommand::InstallPluginStep {
            request_id,
            step,
            plugin,
        } if !record.readonly => {
            let operation = match record.steps.len() {
                0 => FIRST,
                1 if record.flow == Flow::InstallAndReinstall => SECOND,
                _ => return Err(Failure::WrongObservation),
            };
            check(
                step.service_id == SERVICE
                    && step.machine_id == MACHINE
                    && step.plugin_id == "victoria"
                    && step.plugin_kind == cowboy_plugin_sdk::PluginKind::TelemetryBackend
                    && step.plugin_version == "1.1.0"
                    && step.operation_id == operation
                    && request_id == format!("plugin-install-{operation}")
                    && step.matches_envelope(&plugin)
                    && record.target.as_ref() == Some(&step.expected)
                    && record.counts.receipts_observed as usize == record.steps.len(),
            )?;
            step.validate().map_err(|_| Failure::WrongObservation)?;
            record.target = None;
            record.steps.push(*step);
            record.counts.steps_observed += 1;
            if record.flow == Flow::DisconnectBeforeDelivery {
                record.counts.forced_disconnects += 1;
                return Ok(Action::Disconnect);
            }
            record.counts.steps_forwarded += 1;
        }
        // No legacy installer, auth command, receipt fallback, export or session.
        _ => return Err(Failure::WrongObservation),
    }
    Ok(Action::Forward)
}

fn event_frame(event: MachineEvent, record: &mut Record) -> Result<Action, Failure> {
    match event {
        MachineEvent::PluginInstallationTarget {
            request_id,
            observation,
        } if !record.readonly => {
            let (expected_id, expected_digest) = record
                .target_query
                .take()
                .ok_or(Failure::WrongObservation)?;
            let InstallTargetObservation::Observed {
                query_digest,
                target,
                admission_enabled: true,
            } = *observation
            else {
                return Err(Failure::WrongObservation);
            };
            check(request_id == expected_id && query_digest == expected_digest)?;
            target.validate().map_err(|_| Failure::WrongObservation)?;
            record.target = Some(target);
            record.counts.target_receipts += 1;
        }
        MachineEvent::PluginInstallationStep {
            request_id,
            observation,
        } if !record.readonly => {
            let step = record.steps.last().ok_or(Failure::WrongObservation)?;
            check(
                observation.admission_enabled
                    && record.counts.receipts_observed < record.counts.steps_forwarded
                    && request_id == format!("plugin-install-{}", step.operation_id),
            )?;
            let InstallLookup::Found { receipt } = observation.result else {
                return Err(Failure::WrongObservation);
            };
            check(
                receipt.matches(step) && matches!(receipt.outcome, InstallOutcome::Applied { .. }),
            )?;
            record.counts.receipts_observed += 1;
            match record.flow {
                Flow::LostReceipt | Flow::ControllerCrashAfterApplied => {
                    record.counts.dropped_receipts += 1;
                    return Ok(Action::Drop);
                }
                Flow::DisconnectAfterApplied => {
                    record.counts.dropped_receipts += 1;
                    record.counts.forced_disconnects += 1;
                    return Ok(Action::Disconnect);
                }
                _ => record.counts.receipts_forwarded += 1,
            }
        }
        MachineEvent::Inventory { .. } | MachineEvent::PluginInventory { .. } => {}
        _ => return Err(Failure::WrongObservation),
    }
    Ok(Action::Forward)
}

fn handshake(
    frame: MachineFrame,
    from_machine: bool,
    record: &mut Record,
) -> Result<Action, Failure> {
    use crate::runtime_wire::{CoreCommand, Frame};
    match frame {
        MachineFrame::Challenge {
            proof_version: 3, ..
        } if !from_machine => {}
        MachineFrame::Hello { hello }
            if from_machine
                && hello.machine_id == MACHINE
                && hello.challenge_signature.is_some()
                && hello.encryption_public_key.is_some()
                && hello.min_protocol <= 19
                && hello.max_protocol >= 19 =>
        {
            let generation = hello
                .components
                .iter()
                .find(|component| {
                    component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                        && component.state == crate::machine_protocol::ComponentState::Active
                })
                .map_or(hello.host_build, |component| component.generation.clone());
            check(!generation.is_empty() && generation.len() <= 128)?;
            record.generation = Some(generation);
            record.configured = false;
        }
        MachineFrame::Welcome {
            protocol: 19,
            desired_components,
            ..
        } if !from_machine && desired_components.is_empty() => record.counts.connections += 1,
        MachineFrame::Runtime { frame } => match frame {
            Frame::CoreCommand {
                command:
                    CoreCommand::SetDesiredGeneration {
                        generation,
                        worker_command: None,
                    },
            } if !from_machine
                && !record.configured
                && record.generation.as_ref() == Some(&generation) =>
            {
                record.configured = true;
                record.counts.runtime_configurations += 1;
            }
            Frame::Hello {
                session_id: None, ..
            }
            | Frame::Heartbeat => {}
            Frame::Welcome { workers, .. } if workers.is_empty() => {}
            _ => return Err(Failure::WrongObservation),
        },
        MachineFrame::Heartbeat { .. } => {}
        _ => return Err(Failure::UnexpectedHandshake),
    }
    Ok(Action::Forward)
}

#[test]
fn installation_relay_refuses_old_protocols_auth_and_read_only_target_queries() {
    for protocol in [18, 19, 20] {
        let frame = MachineFrame::Welcome {
            protocol,
            controller_epoch: 1,
            heartbeat_interval_ms: 1000,
            desired_components: Vec::new(),
        };
        let mut record = Record::new(Flow::LostReceipt, false);
        assert_eq!(handshake(frame, false, &mut record).is_ok(), protocol == 19);
    }
    let mut record = Record::new(Flow::InstallAndReinstall, true);
    let query = crate::machine_protocol::plugin_install::InstallTargetQuery {
        schema: 1,
        service_id: SERVICE.into(),
        machine_id: MACHINE.into(),
        plugin_id: "victoria".into(),
    };
    assert!(
        command_frame(
            MachineCommand::ObservePluginInstallation {
                request_id: "query".into(),
                query: Box::new(query)
            },
            &mut record
        )
        .is_err()
    );
    record.readonly = false;
    assert!(
        command_frame(
            MachineCommand::BeginLogin {
                request_id: "forbidden".into(),
                provider: "forbidden".into(),
                auth_method: None
            },
            &mut record
        )
        .is_err()
    );
    assert_eq!(record.counts.steps_forwarded, 0);
}

#[test]
fn installation_relay_drops_only_exact_applied_receipts_and_rejects_repeats() {
    use crate::machine_protocol::plugin_install::{InstallObservation, InstallReceipt};
    let step = crate::machine_protocol::plugin_install::fixture();
    let receipt = InstallReceipt {
        step: step.clone(),
        request_digest: step.request_digest().unwrap(),
        outcome: InstallOutcome::Applied {
            revision: format!("installation-{}", "a".repeat(64))
                .try_into()
                .unwrap(),
        },
    };
    let event = |receipt: InstallReceipt| MachineEvent::PluginInstallationStep {
        request_id: format!("plugin-install-{}", step.operation_id),
        observation: Box::new(InstallObservation {
            admission_enabled: true,
            result: InstallLookup::Found {
                receipt: Box::new(receipt),
            },
        }),
    };
    for (flow, expected) in [
        (Flow::InstallAndReinstall, Action::Forward),
        (Flow::LostReceipt, Action::Drop),
        (Flow::ControllerCrashAfterApplied, Action::Drop),
        (Flow::DisconnectAfterApplied, Action::Disconnect),
    ] {
        let mut record = Record::new(flow, false);
        record.steps.push(step.clone());
        record.counts.steps_forwarded = 1;
        let mut wrong = receipt.clone();
        wrong.request_digest = format!("sha256:{}", "f".repeat(64));
        assert!(event_frame(event(wrong), &mut record).is_err());
        assert_eq!(record.counts.receipts_observed, 0);
        assert_eq!(
            event_frame(event(receipt.clone()), &mut record).unwrap(),
            expected
        );
        assert_eq!(record.counts.receipts_observed, 1);
        assert_eq!(
            record.counts.receipts_forwarded,
            u32::from(flow == Flow::InstallAndReinstall)
        );
        assert!(event_frame(event(receipt.clone()), &mut record).is_err());
        assert_eq!(record.counts.receipts_observed, 1);
    }
}
