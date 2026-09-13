use super::super::admission::{PolicyCase, Scenario, State};
use super::*;
use std::os::unix::fs::MetadataExt as _;

mod machine;
mod service;

#[derive(PartialEq, Eq)]
pub(super) struct PolicySnapshot {
    identity: [u64; 5],
    changed: (i64, i64),
    link: Option<PathBuf>,
    bytes: Vec<u8>,
}

pub(super) fn policy_snapshot(path: &Path) -> Result<Option<PolicySnapshot>> {
    let meta = match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        value => value?,
    };
    Ok(Some(PolicySnapshot {
        identity: [
            meta.dev(),
            meta.ino(),
            meta.nlink(),
            u64::from(meta.mode()),
            u64::from(meta.uid()),
        ],
        changed: (meta.ctime(), meta.ctime_nsec()),
        link: meta
            .is_symlink()
            .then(|| std::fs::read_link(path))
            .transpose()?,
        bytes: std::fs::read(path)?,
    }))
}

pub(in super::super) async fn run(
    artifact: &Artifact,
    scenario: Scenario,
    policy: PolicyCase,
    state: &mut State,
    root: &Path,
    cold_read: u8,
) -> Result<Option<u16>, Failure> {
    let policy_path = match artifact.lane {
        Lane::Controller => root.join("controller-writer.json"),
        Lane::Machine => root.join("machine/telemetry-writer-policy.json"),
    };
    if cold_read == 1 {
        policy.write(&policy_path).map_err(|_| Failure::Setup)?;
    }
    let policy_before = policy_snapshot(&policy_path).map_err(|_| Failure::Setup)?;
    unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    let mut command = configured_command(artifact, root, address);
    if artifact.lane == Lane::Controller && policy != PolicyCase::Absent {
        command.arg("--telemetry-writer-policy").arg(&policy_path);
    }
    let listener = if artifact.lane == Lane::Controller {
        drop(listener);
        None
    } else {
        Some(listener)
    };
    let mut running = Running::spawn(&mut command)?;
    let result = tokio::time::timeout(DEADLINE, async {
        if let Some(marker) = policy.failure_marker(artifact.lane) {
            rejected_startup(&mut running, marker).await?;
            return Ok(None);
        }
        match listener {
            None => {
                controller_admission(
                    &mut running,
                    address,
                    &state.evidence,
                    policy.startup_binding(),
                )
                .await?;
                service::exercise(address, policy, state, root, cold_read).await?;
                // Writer policy and historical resolution do not create an exporter.
                if running.log_contains("managed telemetry background startup evaluated") {
                    return Err(Failure::WrongExportState);
                }
                startup::local_recording(address, root, cold_read, false).await?;
                Ok(None)
            }
            Some(listener) => {
                let (mut socket, protocol) = connect_machine(&mut running, listener, root).await?;
                if protocol != 18 {
                    return Err(Failure::WrongProtocol);
                }
                observe_machine(&mut socket, protocol, &state.evidence).await?;
                unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)?;
                machine::exercise(&mut socket, scenario, policy, state, root, cold_read).await?;
                observe_machine(&mut socket, protocol, &state.evidence).await?;
                Ok(Some(protocol))
            }
        }
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    let cleanup = running.finish().await;
    let retained = unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged);
    let policy_after = policy_snapshot(&policy_path).map_err(|_| Failure::EvidenceChanged)?;
    cleanup
        .and(retained)
        .and(if policy_before == policy_after {
            Ok(())
        } else {
            Err(Failure::EvidenceChanged)
        })
        .and(result)
}

#[test]
fn snapshot_observes_equal_byte_policy_replacement_and_link_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policy.json");
    PolicyCase::All.write(&path).unwrap();
    let before = policy_snapshot(&path).unwrap();
    let replacement = root.path().join("next.json");
    PolicyCase::All.write(&replacement).unwrap();
    std::fs::rename(replacement, &path).unwrap();
    assert!(before != policy_snapshot(&path).unwrap());
    let link = root.path().join("policy-link.json");
    let new_link = root.path().join("next-link.json");
    let equal = root.path().join("equal.json");
    PolicyCase::All.write(&equal).unwrap();
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let before_link = policy_snapshot(&link).unwrap();
    std::os::unix::fs::symlink(&equal, &new_link).unwrap();
    std::fs::rename(new_link, &link).unwrap();
    let after_link = policy_snapshot(&link).unwrap();
    assert_eq!(
        before_link.as_ref().unwrap().bytes,
        after_link.as_ref().unwrap().bytes
    );
    assert!(before_link != after_link);
    assert!(
        policy_snapshot(&root.path().join("absent"))
            .unwrap()
            .is_none()
    );
}
