//! Actual immutable readers, not a new production inspection or mutation API.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod fixtures;
mod manifest;
mod probe;

use fixtures::{Case, Fixture};
use manifest::{Artifact, Lane, Matrix, Role};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Failure {
    Setup,
    Spawn,
    Timeout,
    ExitedBeforeReady,
    UnexpectedReadiness,
    MissingReaderFence,
    WrongProtocol,
    WrongObservation,
    EvidenceChanged,
    Cleanup,
}

#[derive(Serialize)]
struct Check {
    lane: Lane,
    role: Role,
    case: Case,
    cold_read: u8,
    service_fixture_sha256: Option<String>,
    machine_fixture_sha256: Option<String>,
    accepted: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct Receipt {
    schema: u16,
    purpose: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    ssh_keygen: manifest::Executable,
    checks: Vec<Check>,
    accepted: bool,
    not_checked: [&'static str; 6],
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn write_receipt(path: &Path, receipt: &impl Serialize) -> Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("receipt parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, receipt)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[tokio::test]
#[ignore = "just telemetry-reader-conformance; immutable inputs and isolated loopback required"]
async fn immutable_telemetry_readers() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_READER_MATRIX")?);
    let path = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_READER_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let revision = manifest::clean_revision()?;
    let matrix: Matrix = serde_json::from_slice(&std::fs::read(matrix)?)?;
    let artifacts = matrix.resolve()?;
    // Fixture writers exist only in this test binary. Child release processes
    // receive no write/installation admission or mutation commands.
    let fixtures = Fixture::all().await?;
    let mut receipt = Receipt {
        schema: 1,
        purpose: "supplied_immutable_telemetry_reader_matrix",
        source_revision: revision,
        artifacts,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_active_rollback_and_cold_configuration",
            "production_data_and_postgres_startup",
            "plugin_installation_policy_and_external_otlp_delivery",
            "writer_admission_and_operator_confirmation_surfaces",
            "native_clients_provider_auth_and_existing_sessions",
            "host_activation_and_full_plugin_refactor",
        ],
    };
    for artifact in &receipt.artifacts {
        for fixture in &fixtures {
            // Reopen the SAME disposable state twice, including corrupt cases.
            let root = tempfile::tempdir()?;
            let setup = probe::seed(root.path(), fixture, &receipt.ssh_keygen.path).await;
            for cold_read in 1..=2 {
                let result = if setup.is_ok() {
                    probe::run(artifact, fixture, root.path()).await
                } else {
                    Err(Failure::Setup)
                };
                eprintln!(
                    "{:?}/{:?}/{:?}/{cold_read}: {result:?}",
                    artifact.lane, artifact.role, fixture.case
                );
                receipt.checks.push(Check {
                    lane: artifact.lane,
                    role: artifact.role,
                    case: fixture.case,
                    cold_read,
                    service_fixture_sha256: fixture.document.as_ref().map(|s| sha256(s.as_bytes())),
                    machine_fixture_sha256: fixture.machine.as_deref().map(sha256),
                    accepted: result.is_ok(),
                    failure: result.err(),
                });
            }
        }
    }
    receipt.accepted = receipt.checks.len() == 6 * Case::ALL.len() * 2
        && receipt.checks.iter().all(|check| check.accepted);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "immutable telemetry reader matrix failed; inspect bounded receipt"
    );
    Ok(())
}

#[test]
fn receipt_is_private_create_only_and_has_no_diagnostic_payloads() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("receipt.json");
    write_receipt(&path, &Failure::WrongObservation).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(write_receipt(&path, &Failure::Cleanup).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let link = root.path().join("linked.json");
    symlink(&path, &link).unwrap();
    assert!(write_receipt(&link, &Failure::Cleanup).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
