//! Byte-preserving relay. Only a selected ACK may be dropped; no forged reply.
use super::*;
use tokio::sync::watch;

#[derive(Default)]
struct Record {
    counts: WireCounts,
    failure: Option<Failure>,
    rejection: Option<RelayRejection>,
    generation: Option<String>,
    configured: bool,
    mutation_started: Option<std::time::Instant>,
    delivery: Option<delivery::Wire>,
}

pub(super) struct Proxy {
    pub address: std::net::SocketAddr,
    record: Arc<parking_lot::Mutex<Record>>,
    cut: watch::Sender<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    pub async fn start(upstream: std::net::SocketAddr, flow: Flow) -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Failure::Setup)?;
        let address = listener.local_addr().map_err(|_| Failure::Setup)?;
        let record = Arc::new(parking_lot::Mutex::new(Record {
            delivery: (flow == Flow::ManagedDelivery).then(delivery::Wire::default),
            ..Record::default()
        }));
        let (cut, mut signal) = watch::channel(0_u64);
        let observed = record.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                // Restart may race a new TCP accept. It is readiness failure if
                // no subsequent *authenticated* pair becomes available.
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
                signal.borrow_and_update();
                loop {
                    let (downstream, message) = tokio::select! {
                        _ = signal.changed() => { break; }
                        frame = machine.next() => (true,frame),
                        frame = controller.next() => (false,frame),
                    };
                    let Some(Ok(message)) = message else { break };
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                    let decision = inspect(&message, downstream, flow, &mut observed.lock());
                    match decision {
                        Err(failure) => {
                            let mut record = observed.lock();
                            record.failure = Some(failure);
                            record.rejection = Some(rejection(&message));
                            break;
                        }
                        Ok(Action::Drop) => continue,
                        Ok(Action::Disconnect) => break,
                        Ok(Action::Forward) => {}
                    }
                    let sent = if downstream {
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
            cut,
            task,
        })
    }

    pub fn counts(&self) -> Result<WireCounts, Failure> {
        let record = self.record.lock();
        match record.failure {
            Some(failure) => Err(failure),
            None => Ok(record.counts.clone()),
        }
    }

    pub fn snapshot(&self) -> WireCounts {
        self.record.lock().counts.clone()
    }

    pub fn export_target(
        &self,
        target: crate::machine_protocol::telemetry_binding::BindingInstallation,
    ) -> Result<(), Failure> {
        self.record
            .lock()
            .delivery
            .as_mut()
            .ok_or(Failure::Setup)?
            .target(target)
    }

    pub fn export_ack(&self, ack: Ack) -> Result<(), Failure> {
        self.counts()?;
        self.record
            .lock()
            .delivery
            .as_mut()
            .ok_or(Failure::Setup)?
            .arm(ack)
    }

    pub fn export_snapshot(&self) -> Vec<WireExport> {
        self.record
            .lock()
            .delivery
            .as_ref()
            .map(|d| d.entries.clone())
            .unwrap_or_default()
    }

    pub fn rejection(&self) -> Option<RelayRejection> {
        self.record.lock().rejection
    }

    pub fn disconnect(&self) {
        self.record.lock().counts.forced_disconnects += 1;
        self.cut.send_modify(|value| *value += 1);
    }

    pub async fn finish(&mut self) -> Result<(), Failure> {
        self.task.abort();
        match (&mut self.task).await {
            Err(error) if error.is_cancelled() => Ok(()),
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

fn rejection(message: &Message) -> RelayRejection {
    use crate::runtime_wire::{CoreCommand, Frame};
    let Message::Text(text) = message else {
        return RelayRejection::Decode;
    };
    match serde_json::from_str::<MachineFrame>(text) {
        Ok(MachineFrame::Command { .. }) => RelayRejection::MachineCommand,
        Ok(MachineFrame::Event { .. }) => RelayRejection::MachineEvent,
        Ok(MachineFrame::Runtime { frame }) => match frame {
            Frame::CoreCommand {
                command: CoreCommand::SetDesiredGeneration { .. },
            } => RelayRejection::RuntimeGeneration,
            Frame::CoreCommand { .. } => RelayRejection::RuntimeCoreCommand,
            _ => RelayRejection::RuntimeOther,
        },
        Ok(_) => RelayRejection::Handshake,
        Err(_) => RelayRejection::Decode,
    }
}

fn inspect(
    message: &Message,
    downstream: bool,
    flow: Flow,
    record: &mut Record,
) -> Result<Action, Failure> {
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
    let counts = &mut record.counts;
    match frame {
        MachineFrame::Challenge {
            proof_version: 3, ..
        } if !downstream => {}
        MachineFrame::Hello { hello }
            if downstream
                && hello.machine_id == MACHINE
                && hello.challenge_signature.is_some()
                && hello.encryption_public_key.is_some() =>
        {
            let generation = hello
                .components
                .iter()
                .find(|component| {
                    component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                        && component.state == crate::machine_protocol::ComponentState::Active
                })
                .map_or(hello.host_build, |component| component.generation.clone());
            if generation.is_empty() || generation.len() > 128 {
                return Err(Failure::UnexpectedHandshake);
            }
            record.generation = Some(generation);
            record.configured = false;
        }
        MachineFrame::Welcome {
            protocol,
            desired_components,
            ..
        } if !downstream && matches!(protocol, 18 | 19) && desired_components.is_empty() => {
            counts.protocol = Some(protocol);
            counts.connections += 1;
        }
        MachineFrame::Command { command } if !downstream => match command {
            MachineCommand::ExportBoundTelemetry {
                request_id,
                attempt,
            } if flow == Flow::ManagedDelivery => {
                record
                    .delivery
                    .as_mut()
                    .ok_or(Failure::Setup)?
                    .begin(request_id, *attempt)?;
                counts.export_commands += 1;
            }
            MachineCommand::RefreshInventory { .. } => {}
            MachineCommand::QueryTelemetryBinding { .. } => {
                counts.binding_queries += 1;
                if flow == Flow::BindingLostAck
                    && counts.dropped_binding_acks == 1
                    && counts.fallback_after_ms.is_none()
                {
                    counts.fallback_after_ms = record
                        .mutation_started
                        .map(|time| u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX));
                }
            }
            MachineCommand::CommitTelemetryBinding { .. } => {
                counts.binding_commands += 1;
                record.mutation_started = Some(std::time::Instant::now());
            }
            MachineCommand::QueryTelemetryRecovery { .. } => {
                counts.recovery_queries += 1;
                if flow == Flow::PreparedRecovery
                    && counts.dropped_recovery_acks == 1
                    && counts.fallback_after_ms.is_none()
                {
                    counts.fallback_after_ms = record
                        .mutation_started
                        .map(|time| u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX));
                }
            }
            MachineCommand::RecoverTelemetryBinding { .. } => {
                counts.recovery_commands += 1;
                record.mutation_started = Some(std::time::Instant::now());
            }
            MachineCommand::QueryTelemetryRecoveryAudit { .. } => counts.audit_queries += 1,
            _ => return Err(Failure::WrongObservation), // No install, Provider, session or export commands.
        },
        MachineFrame::Event { event } if downstream => match event {
            MachineEvent::TelemetryExported {
                request_id,
                receipt,
            } if flow == Flow::ManagedDelivery => {
                let ack = record
                    .delivery
                    .as_mut()
                    .ok_or(Failure::Setup)?
                    .finish(&request_id, receipt.map(|r| *r))?;
                counts.export_receipts += 1;
                match ack {
                    Ack::Forward => {}
                    Ack::Drop => {
                        counts.dropped_export_acks += 1;
                        return Ok(Action::Drop);
                    }
                    Ack::Disconnect => {
                        counts.dropped_export_acks += 1;
                        counts.forced_disconnects += 1;
                        return Ok(Action::Disconnect);
                    }
                }
            }
            MachineEvent::TelemetryBindingCommitted { .. }
                if flow == Flow::BindingLostAck && counts.dropped_binding_acks == 0 =>
            {
                counts.dropped_binding_acks += 1;
                return Ok(Action::Drop);
            }
            MachineEvent::TelemetryBindingCommitted { .. }
                if flow == Flow::BindingDisconnected && counts.dropped_binding_acks == 0 =>
            {
                counts.dropped_binding_acks += 1;
                counts.forced_disconnects += 1;
                return Ok(Action::Disconnect);
            }
            MachineEvent::TelemetryBindingRecovered { .. }
                if flow == Flow::PreparedRecovery && counts.dropped_recovery_acks == 0 =>
            {
                counts.dropped_recovery_acks += 1;
                return Ok(Action::Drop);
            }
            MachineEvent::TelemetryBindingCommitted { .. }
            | MachineEvent::TelemetryBindingObservation { .. }
            | MachineEvent::TelemetryBindingRecovered { .. }
            | MachineEvent::TelemetryRecoveryObservation { .. }
            | MachineEvent::TelemetryRecoveryAuditObservation { .. }
            | MachineEvent::Inventory { .. }
            | MachineEvent::PluginInventory { .. } => {}
            _ => return Err(Failure::WrongObservation),
        },
        MachineFrame::Runtime { frame } => match frame {
            // Normal Core-to-empty-broker initialization. It must retain the
            // authenticated Machine's own generation and cannot supply a worker
            // executable, change a generation, or dispatch any session.
            crate::runtime_wire::Frame::CoreCommand {
                command:
                    crate::runtime_wire::CoreCommand::SetDesiredGeneration {
                        generation,
                        worker_command: None,
                    },
            } if !downstream
                && !record.configured
                && record.generation.as_ref() == Some(&generation) =>
            {
                record.configured = true;
                counts.runtime_configurations += 1;
            }
            crate::runtime_wire::Frame::Hello {
                session_id: None, ..
            }
            | crate::runtime_wire::Frame::Heartbeat => {}
            crate::runtime_wire::Frame::Welcome { workers, .. } if workers.is_empty() => {}
            _ => return Err(Failure::WrongObservation),
        },
        MachineFrame::Heartbeat { .. } => {}
        _ => return Err(Failure::UnexpectedHandshake),
    }
    Ok(Action::Forward)
}

#[test]
fn telemetry_relay_keeps_known_recovery_protocols_and_records_actual_negotiation() {
    for protocol in [17, 18, 19, 20] {
        let message = Message::Text(
            serde_json::to_string(&MachineFrame::Welcome {
                protocol,
                controller_epoch: 1,
                heartbeat_interval_ms: 1000,
                desired_components: Vec::new(),
            })
            .unwrap()
            .into(),
        );
        let mut record = Record::default();
        let expected = matches!(protocol, 18 | 19);
        assert_eq!(
            inspect(&message, false, Flow::BindingRoundTrip, &mut record).is_ok(),
            expected
        );
        assert_eq!(record.counts.protocol, expected.then_some(protocol));
        assert_eq!(record.counts.connections, u32::from(expected));
        assert!(
            inspect(
                &message,
                true,
                Flow::BindingRoundTrip,
                &mut Record::default()
            )
            .is_err()
        );
    }
}

#[test]
fn relay_drops_only_one_selected_ack_and_never_admits_worker_commands() {
    use crate::machine_protocol::telemetry_binding::{BindingCommitFailure, BindingCommitResult};
    let message = Message::Text(
        serde_json::to_string(&MachineFrame::Event {
            event: MachineEvent::TelemetryBindingCommitted {
                request_id: "fixture".into(),
                result: Box::new(BindingCommitResult::Unavailable {
                    failure: BindingCommitFailure::ReaderOnly,
                }),
            },
        })
        .unwrap()
        .into(),
    );
    let mut record = Record::default();
    assert_eq!(
        inspect(&message, true, Flow::BindingLostAck, &mut record).unwrap(),
        Action::Drop
    );
    assert_eq!(
        inspect(&message, true, Flow::BindingLostAck, &mut record).unwrap(),
        Action::Forward
    );
    assert_eq!(record.counts.dropped_binding_acks, 1);
    let command = Message::Text(
        serde_json::to_string(&MachineFrame::Command {
            command: MachineCommand::BeginLogin {
                request_id: "fixture".into(),
                provider: "forbidden".into(),
                auth_method: None,
            },
        })
        .unwrap()
        .into(),
    );
    assert!(inspect(&command, false, Flow::BindingRoundTrip, &mut record).is_err());
}

#[test]
fn relay_admits_only_one_exact_empty_broker_generation_without_an_executable() {
    use crate::runtime_wire::{CoreCommand, Frame};
    let message = |generation: &str, worker_command: Option<&str>| {
        Message::Text(
            serde_json::to_string(&MachineFrame::Runtime {
                frame: Frame::CoreCommand {
                    command: CoreCommand::SetDesiredGeneration {
                        generation: generation.into(),
                        worker_command: worker_command.map(str::to_owned),
                    },
                },
            })
            .unwrap()
            .into(),
        )
    };
    let ready = || Record {
        generation: Some("fixture-generation".into()),
        ..Record::default()
    };
    let valid = message("fixture-generation", None);
    let mut record = ready();
    assert_eq!(
        inspect(&valid, false, Flow::BindingLostAck, &mut record).unwrap(),
        Action::Forward
    );
    assert_eq!(record.counts.runtime_configurations, 1);
    assert!(inspect(&valid, false, Flow::BindingLostAck, &mut record).is_err());
    assert!(inspect(&valid, false, Flow::BindingLostAck, &mut Record::default()).is_err());
    assert!(inspect(&valid, true, Flow::BindingLostAck, &mut ready()).is_err());
    for rejected in [
        message("foreign-generation", None),
        message("fixture-generation", Some("no-worker")),
    ] {
        assert!(inspect(&rejected, false, Flow::BindingLostAck, &mut ready()).is_err());
    }
    for command in [
        CoreCommand::Cancel {
            session_id: "forbidden".into(),
            command_id: "forbidden".into(),
        },
        CoreCommand::RollProvider {
            provider: "forbidden".into(),
        },
    ] {
        let frame = Message::Text(
            serde_json::to_string(&MachineFrame::Runtime {
                frame: Frame::CoreCommand { command },
            })
            .unwrap()
            .into(),
        );
        assert!(inspect(&frame, false, Flow::BindingLostAck, &mut ready()).is_err());
    }
}
