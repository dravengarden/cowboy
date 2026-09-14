use super::super::machine_installation::{InstallCase, evidence_path};
use super::*;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallUnavailable,
};

pub(in super::super) async fn run(
    artifact: &Artifact,
    root: &Path,
    case: InstallCase,
) -> Result<(), Failure> {
    let before = snapshot(root, case)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    // No --plugin-operation-admission: reader negotiation cannot authorize
    // namespace writes, including when testing a later writer-capable ELF.
    let mut running = Running::spawn(&mut configured_command(artifact, root, address))?;
    let result = tokio::time::timeout(DEADLINE, async {
        if let Some(marker) = case.marker() {
            rejected_startup(&mut running, marker).await?;
        } else {
            let (mut socket, protocol) =
                connect_machine_at(&mut running, listener, root, 19).await?;
            if protocol != 19 {
                return Err(Failure::WrongProtocol);
            }
            observe(&mut socket, case).await?;
        }
        Ok(())
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    let cleanup = running.finish().await;
    if snapshot(root, case)? != before {
        return Err(Failure::EvidenceChanged);
    }
    cleanup.and(result)
}

fn snapshot(root: &Path, case: InstallCase) -> Result<serde_json::Value, Failure> {
    let paths = [
        evidence_path(root, case),
        root.join("machine/plugin-operations/installations-v1/victoria.json"),
    ];
    let mut files = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(Failure::EvidenceChanged),
        };
        files.push(bytes);
    }
    Ok(serde_json::json!({
        "files":files,
        "attempts_present":root.join("machine/plugin-operations/install-attempts-v1").exists(),
        "slots_present":root.join("machine/plugin-operations/installations-v1").exists(),
    }))
}

async fn query(
    socket: &mut Socket,
    step: crate::machine_protocol::plugin_install::InstallStep,
) -> Result<InstallObservation, Failure> {
    send(
        socket,
        MachineFrame::Command {
            command: MachineCommand::QueryPluginInstallStep {
                request_id: "install-read-only".into(),
                step: Box::new(step),
            },
        },
    )
    .await?;
    response(socket).await
}

async fn response(socket: &mut Socket) -> Result<InstallObservation, Failure> {
    loop {
        match receive(socket).await? {
            MachineFrame::Event {
                event:
                    MachineEvent::PluginInstallationStep {
                        request_id,
                        observation,
                    },
            } if request_id == "install-read-only" => return Ok(*observation),
            MachineFrame::Heartbeat { .. }
            | MachineFrame::Event {
                event: MachineEvent::Inventory { .. },
            } => {}
            MachineFrame::Event {
                event: MachineEvent::PluginInventory { plugins, .. },
            } if plugins.is_empty() => {}
            _ => return Err(Failure::WrongObservation),
        }
    }
}

async fn observe(socket: &mut Socket, case: InstallCase) -> Result<(), Failure> {
    let step = case.step();
    let expected = case.receipt().map_or(
        InstallLookup::Unavailable {
            reason: InstallUnavailable::TargetChanged,
        },
        |receipt| InstallLookup::Found {
            receipt: Box::new(receipt),
        },
    );
    for _ in 0..2 {
        let result = query(socket, step.clone()).await?;
        if result.admission_enabled || result.result != expected {
            return Err(Failure::WrongObservation);
        }
    }
    let mut foreign = step.clone();
    foreign.service_id = "foreign-service".into();
    if query(socket, foreign).await?.result
        != (InstallLookup::Unavailable {
            reason: InstallUnavailable::WrongOwner,
        })
    {
        return Err(Failure::WrongObservation);
    }
    if case != InstallCase::Absent {
        send(
            socket,
            MachineFrame::Command {
                command: MachineCommand::InstallPluginStep {
                    request_id: "install-read-only".into(),
                    step: Box::new(step.clone()),
                    plugin: Box::new(case.desired()),
                },
            },
        )
        .await?;
        let duplicate = response(socket).await?;
        if duplicate.admission_enabled || duplicate.result != expected {
            return Err(Failure::WrongObservation);
        }
        let mut changed = step.clone();
        changed.expires_at_ms += 1;
        if query(socket, changed).await?.result
            != (InstallLookup::Unavailable {
                reason: InstallUnavailable::IdentityConflict,
            })
        {
            return Err(Failure::WrongObservation);
        }
        let mut other = step;
        other.operation_id = "new-operation-is-not-replay".into();
        let expected = if case == InstallCase::Rejected {
            InstallLookup::NotFound {}
        } else {
            InstallLookup::Unavailable {
                reason: InstallUnavailable::SlotFenced,
            }
        };
        if query(socket, other).await?.result != expected {
            return Err(Failure::WrongObservation);
        }
    }
    Ok(())
}
