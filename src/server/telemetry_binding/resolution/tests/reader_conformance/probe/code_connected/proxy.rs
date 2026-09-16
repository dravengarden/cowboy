//! Byte-preserving relay: hold an actual correlated reply, never synthesize it.
use super::*;
use std::collections::BTreeMap;
use tokio::sync::{Notify, watch};

#[derive(Clone, Default, Serialize)]
pub(super) struct Counts {
    pub connections: u32,
    pub configurations: u32,
    pub commands: BTreeMap<String, u32>,
    pub replies: u32,
    pub held_replies: u32,
    pub cuts: u32,
}

#[derive(Clone)]
pub(super) struct Gate {
    reached: Arc<Notify>,
    resume: Arc<Notify>,
}
impl Gate {
    pub async fn held(&self) -> Result<(), Failure> {
        tokio::time::timeout(DEADLINE, self.reached.notified())
            .await
            .map_err(|_| Failure::Timeout)
    }
    pub fn release(&self) {
        self.resume.notify_one();
    }
}

#[derive(Default)]
struct Record {
    counts: Counts,
    pending: BTreeMap<String, String>,
    hold: Option<(&'static str, Gate)>,
    failure: Option<Failure>,
    generation: Option<String>,
    configured: bool,
}

impl Record {
    fn command(&mut self, id: String, kind: &str) -> Result<(), Failure> {
        check(self.pending.len() < 32 && !self.pending.contains_key(&id))?;
        self.pending.insert(id, kind.into());
        *self.counts.commands.entry(kind.into()).or_default() += 1;
        Ok(())
    }
    fn reply(&mut self, id: &str) -> Result<Option<Gate>, Failure> {
        let kind = self.pending.remove(id).ok_or(Failure::WrongObservation)?;
        self.counts.replies += 1;
        if self
            .hold
            .as_ref()
            .is_some_and(|(expected, _)| *expected == kind)
        {
            self.counts.held_replies += 1;
            return Ok(Some(self.hold.take().unwrap().1));
        }
        Ok(None)
    }
}

pub(super) struct Proxy {
    pub address: std::net::SocketAddr,
    record: Arc<parking_lot::Mutex<Record>>,
    cut: watch::Sender<u32>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    pub async fn start(upstream: std::net::SocketAddr) -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Failure::Setup)?;
        let address = listener.local_addr().map_err(|_| Failure::Setup)?;
        let record = Arc::new(parking_lot::Mutex::new(Record::default()));
        let observed = record.clone();
        let (cut, mut signal) = watch::channel(0);
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
                signal.borrow_and_update();
                loop {
                    let (from_machine, message) = tokio::select! {
                        _ = signal.changed() => break,
                        frame = machine.next() => (true, frame),
                        frame = controller.next() => (false, frame),
                    };
                    let Some(Ok(message)) = message else { break };
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                    let decision = inspect(&message, from_machine, &mut observed.lock());
                    match decision {
                        Err(error) => {
                            observed.lock().failure = Some(error);
                            break;
                        }
                        Ok(Some(gate)) => {
                            gate.reached.notify_one();
                            if tokio::time::timeout(DEADLINE, gate.resume.notified())
                                .await
                                .is_err()
                            {
                                observed.lock().failure = Some(Failure::Timeout);
                                break;
                            }
                        }
                        Ok(None) => {}
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
            cut,
            task,
        })
    }

    pub fn counts(&self) -> Result<Counts, Failure> {
        let record = self.record.lock();
        if let Some(failure) = record.failure {
            return Err(failure);
        }
        Ok(record.counts.clone())
    }
    pub fn snapshot(&self) -> Counts {
        self.record.lock().counts.clone()
    }
    pub fn hold(&self, kind: &'static str) -> Result<Gate, Failure> {
        let mut record = self.record.lock();
        check(
            record.hold.is_none()
                && ["openBufferLease", "readBufferLease", "releaseBufferLease"].contains(&kind),
        )?;
        let gate = Gate {
            reached: Arc::default(),
            resume: Arc::default(),
        };
        record.hold = Some((kind, gate.clone()));
        Ok(gate)
    }
    pub fn cut(&self) -> Result<(), Failure> {
        let mut record = self.record.lock();
        check(record.pending.is_empty() && record.hold.is_none())?;
        record.counts.cuts += 1;
        self.cut.send_modify(|value| *value += 1);
        Ok(())
    }
    pub async fn finish(&mut self) -> Result<(), Failure> {
        self.task.abort();
        match (&mut self.task).await {
            Err(error) if error.is_cancelled() => Ok(()),
            _ => Err(Failure::Cleanup),
        }
    }
}

fn inspect(
    message: &Message,
    from_machine: bool,
    record: &mut Record,
) -> Result<Option<Gate>, Failure> {
    let Message::Text(text) = message else {
        check(matches!(message, Message::Ping(_) | Message::Pong(_)))?;
        return Ok(None);
    };
    check(text.len() <= 256 * 1024)?;
    let frame: MachineFrame = serde_json::from_str(text).map_err(|_| Failure::FrameDecode)?;
    match frame {
        MachineFrame::Command { command } if !from_machine => command_frame(command, record)?,
        MachineFrame::Event { event } if from_machine => match event {
            MachineEvent::AdapterResponse { request_id, .. }
            | MachineEvent::PluginInstallationTarget { request_id, .. }
            | MachineEvent::PluginInstallationStep { request_id, .. }
            | MachineEvent::PluginUninstallRecovery { request_id, .. }
            | MachineEvent::PluginUninstallStep { request_id, .. } => {
                return record.reply(&request_id);
            }
            MachineEvent::Inventory { .. } | MachineEvent::PluginInventory { .. } => {}
            _ => return Err(Failure::WrongObservation),
        },
        other => handshake(other, from_machine, record)?,
    }
    Ok(None)
}

fn command_frame(command: MachineCommand, record: &mut Record) -> Result<(), Failure> {
    match command {
        MachineCommand::RefreshInventory { .. } => Ok(()),
        MachineCommand::ObservePluginInstallation { request_id, query } => {
            check(
                query.service_id == SERVICE
                    && query.machine_id == MACHINE
                    && query.plugin_id == "zed",
            )?;
            record.command(request_id, "installationObservation")
        }
        MachineCommand::InstallPluginStep {
            request_id,
            step,
            plugin,
        } => {
            check(
                step.service_id == SERVICE
                    && step.machine_id == MACHINE
                    && step.plugin_id == "zed"
                    && step.operation_id == "connected-code-install"
                    && step.plugin_kind == cowboy_plugin_sdk::PluginKind::CodeIntelligence
                    && step.matches_envelope(&plugin),
            )?;
            step.validate().map_err(|_| Failure::WrongObservation)?;
            record.command(request_id, "installationStep")
        }
        MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
        } => {
            let kind = if adapter == "zed" {
                let kind = payload["type"].as_str().ok_or(Failure::WrongObservation)?;
                check(
                    [
                        "ensureWorktree",
                        "bufferLeaseSupport",
                        "prepareBuffer",
                        "openBufferLease",
                        "queryBufferLease",
                        "releaseBufferLease",
                        "bufferLeaseContentSupport",
                        "readBufferLease",
                    ]
                    .contains(&kind),
                )?;
                kind
            } else {
                // Core file/manifest reader only. Never Agent/runtime commands.
                check(adapter == "code")?;
                let request: crate::code_adapter::CodeAdapterRequest =
                    serde_json::from_value(payload.clone())
                        .map_err(|_| Failure::WrongObservation)?;
                check(matches!(
                    request.operation,
                    crate::code_adapter::CodeOperation::Manifest
                ))?;
                "coreManifest"
            };
            record.command(request_id, kind)
        }
        MachineCommand::QueryPluginUninstallStep { request_id, step }
        | MachineCommand::QueryPluginUninstallRecovery { request_id, step } => {
            check(
                step.service_id == SERVICE && step.machine_id == MACHINE && step.plugin_id == "zed",
            )?;
            record.command(request_id, "uninstallObservation")
        }
        MachineCommand::UninstallPluginStep { request_id, step } => {
            check(
                step.service_id == SERVICE && step.machine_id == MACHINE && step.plugin_id == "zed",
            )?;
            record.command(request_id, "uninstallStep")
        }
        _ => Err(Failure::WrongObservation),
    }
}

fn handshake(frame: MachineFrame, from_machine: bool, record: &mut Record) -> Result<(), Failure> {
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
            record.generation = Some(
                hello
                    .components
                    .iter()
                    .find(|component| {
                        component.id.kind == crate::machine_protocol::ComponentKind::AcpRuntime
                            && component.state == crate::machine_protocol::ComponentState::Active
                    })
                    .map_or(hello.host_build, |component| component.generation.clone()),
            );
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
                record.counts.configurations += 1;
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
    Ok(())
}

#[test]
fn relay_accepts_only_scoped_uninstall_preflight_and_bounds_correlation() {
    let mut step = crate::machine_protocol::plugin_step::fixture();
    step.service_id = SERVICE.into();
    step.machine_id = MACHINE.into();
    step.plugin_id = "zed".into();
    let mut record = Record::default();
    command_frame(
        MachineCommand::QueryPluginUninstallStep {
            request_id: "preflight".into(),
            step: Box::new(step.clone()),
        },
        &mut record,
    )
    .unwrap();
    assert!(record.command("preflight".into(), "uninstallStep").is_err());
    assert!(record.reply("preflight").unwrap().is_none());
    step.machine_id = "another-machine".into();
    assert!(
        command_frame(
            MachineCommand::UninstallPluginStep {
                request_id: "wrong-target".into(),
                step: Box::new(step),
            },
            &mut record,
        )
        .is_err()
    );
    for index in 0..32 {
        record
            .command(index.to_string(), "readBufferLease")
            .unwrap();
    }
    assert!(
        record
            .command("overflow".into(), "readBufferLease")
            .is_err()
    );
}

#[test]
fn relay_refuses_path_reads_reload_and_unsolicited_replies() {
    for kind in [
        "openBuffer",
        "closeBuffer",
        "bufferHover",
        "reloadBuffers",
        "bufferLanguage",
    ] {
        let command = MachineCommand::AdapterRequest {
            request_id: "id".into(),
            adapter: "zed".into(),
            payload: json!({"type":kind}),
        };
        assert!(command_frame(command, &mut Record::default()).is_err());
    }
    assert!(Record::default().reply("unrequested").is_err());
    for protocol in [18, 20] {
        assert!(
            handshake(
                MachineFrame::Welcome {
                    protocol,
                    controller_epoch: 1,
                    heartbeat_interval_ms: 1000,
                    desired_components: vec![]
                },
                false,
                &mut Record::default()
            )
            .is_err()
        );
    }
}
