//! Actual immutable Machine readers of bounded per-attempt codecs. These
//! intentionally partial historical snapshots test readers, not installation
//! execution or independently verified restoration of their active links.
use super::*;
use crate::machine_protocol::installation_revision::InstallationRevision;
use crate::machine_protocol::plugin_install::{
    InstallOutcome, InstallPhase, InstallReceipt, InstallRejection, InstallStep, InstallUncertainty,
};
use crate::machine_protocol::plugin_step::digest;
use fixtures::{MACHINE, SERVICE};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InstallCase {
    Absent,
    Prepared,
    Staging,
    Activating,
    ProjectingAuthentication,
    Applied,
    Rejected,
    Unknown,
    ChecksumCorrupt,
    FutureSchema,
    MissingAuthority,
    MissingSlot,
}

impl InstallCase {
    const ALL: [Self; 12] = [
        Self::Absent,
        Self::Prepared,
        Self::Staging,
        Self::Activating,
        Self::ProjectingAuthentication,
        Self::Applied,
        Self::Rejected,
        Self::Unknown,
        Self::ChecksumCorrupt,
        Self::FutureSchema,
        Self::MissingAuthority,
        Self::MissingSlot,
    ];

    pub(super) fn marker(self) -> Option<&'static str> {
        match self {
            Self::ChecksumCorrupt | Self::FutureSchema => Some("install attempt integrity failure"),
            Self::MissingAuthority => Some("install attempts lost installation authority"),
            Self::MissingSlot => Some("install attempt lost installation slot"),
            _ => None,
        }
    }

    pub(super) fn receipt(self) -> Option<InstallReceipt> {
        let phase = match self {
            Self::Absent => return None,
            Self::Staging | Self::Unknown => InstallPhase::Staging,
            Self::Activating => InstallPhase::Activating,
            Self::ProjectingAuthentication => InstallPhase::ProjectingAuthentication,
            _ => InstallPhase::Prepared,
        };
        let step = self.step();
        let outcome = match self {
            Self::Applied | Self::MissingSlot => InstallOutcome::Applied {
                revision: revision(),
            },
            Self::Rejected => InstallOutcome::Rejected {
                reason: InstallRejection::Expired,
            },
            Self::Unknown => InstallOutcome::Unknown {
                phase,
                reason: InstallUncertainty::Interrupted,
            },
            _ => InstallOutcome::Pending { phase },
        };
        Some(InstallReceipt {
            request_digest: step.request_digest().unwrap(),
            step,
            outcome,
        })
    }

    pub(super) fn step(self) -> InstallStep {
        let mut step = crate::machine_protocol::plugin_install::fixture();
        step.service_id = SERVICE.into();
        step.machine_id = MACHINE.into();
        // Historical deadlines must not be regenerated between child opens.
        step.expires_at_ms = 1_700_000_000_000;
        if self == Self::ProjectingAuthentication {
            step.plugin_kind = cowboy_plugin_sdk::PluginKind::AgentProvider;
        }
        step.envelope_digest = digest(&serde_json::to_vec(&self.desired()).unwrap());
        step
    }

    pub(super) fn desired(self) -> crate::machine_protocol::DesiredPlugin {
        let step = crate::machine_protocol::plugin_install::fixture();
        let kind = if self == Self::ProjectingAuthentication {
            cowboy_plugin_sdk::PluginKind::AgentProvider
        } else {
            step.plugin_kind
        };
        // An expired duplicate must remain observable even when the package
        // is no longer verifiable. This is intentionally NOT installable and
        // cannot start a process or make a network request if replay regresses.
        serde_json::from_value(serde_json::json!({
            "release": {
                "release_schema":1, "plugin_id":step.plugin_id, "plugin_version":step.plugin_version,
                "plugin_kind":kind, "package_digest":digest(b"unavailable historical package"),
                "artifact_digest":step.generation_digest, "artifact_url":"https://example.invalid/historical-plugin",
                "publisher":"fixture", "contract_fingerprint":step.contract_fingerprint,
                "component_release":"2.8.0", "host_bundle_digest":null, "signature":"fixture",
                "supported_platforms":[], "runtime_artifacts":[]
            },
            "package_base64":"e30=", "publisher_public_key":"fixture", "host_bundle_base64":null
        })).unwrap()
    }
}

fn revision() -> InstallationRevision {
    format!("installation-{}", "b".repeat(64))
        .try_into()
        .unwrap()
}

// Exact version-one slot codec order; a canonical receipt checksum is not the
// checksum of serde_json::Value's sorted object projection.
#[derive(Serialize)]
struct IncarnationFixture {
    schema: u16,
    plugin_id: String,
    revision: InstallationRevision,
    previous_revision: Option<InstallationRevision>,
    generation_digest: Option<String>,
    effect: &'static str,
    operation_digest: Option<String>,
    outcome: serde_json::Value,
}

pub(super) fn evidence_path(root: &Path, case: InstallCase) -> PathBuf {
    root.join("machine/plugin-operations/install-attempts-v1")
        .join(format!("{}.json", case.step().key().unwrap()))
}

async fn seed(root: &Path, empty: &Fixture, helper: &Path, case: InstallCase) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    probe::seed(root, empty, helper).await?;
    let Some(receipt) = case.receipt() else {
        return Ok(());
    };
    for directory in ["install-attempts-v1", "installations-v1"] {
        if directory == "installations-v1" && case == InstallCase::MissingAuthority {
            continue;
        }
        let directory = root.join("machine/plugin-operations").join(directory);
        std::fs::create_dir(&directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    if matches!(
        case,
        InstallCase::Applied | InstallCase::ProjectingAuthentication
    ) {
        // A retained historical result does not assert the CURRENT generation
        // is stable. A later interrupted activation keeps this slot pending.
        let transition = IncarnationFixture {
            schema: 1,
            plugin_id: receipt.step.plugin_id.clone(),
            revision: format!("installation-{}", "c".repeat(64))
                .try_into()
                .unwrap(),
            previous_revision: (case == InstallCase::Applied).then(revision),
            generation_digest: Some(receipt.step.generation_digest.clone()),
            effect: "install",
            operation_digest: None,
            outcome: serde_json::json!({"state":"unknown"}),
        };
        probe::private_write(
            &root.join("machine/plugin-operations/installations-v1/victoria.json"),
            &serde_json::to_vec(
                &serde_json::json!({"transition":transition, "evidence_digest":digest(&serde_json::to_vec(&transition)?)}),
            )?,
        )?;
    }
    let mut record = serde_json::json!({"schema":1, "evidence_digest":digest(&serde_json::to_vec(&receipt)?), "receipt":receipt});
    if case == InstallCase::ChecksumCorrupt {
        record["evidence_digest"] = digest(b"wrong").into();
    }
    if case == InstallCase::FutureSchema {
        record["schema"] = 99.into();
    }
    probe::private_write(&evidence_path(root, case), &serde_json::to_vec(&record)?)?;
    Ok(())
}

#[derive(Serialize)]
struct Check {
    role: Role,
    case: InstallCase,
    cold_read: u8,
    accepted: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    ssh_keygen: manifest::Executable,
    checks: Vec<Check>,
    accepted: bool,
    not_checked: [&'static str; 5],
}

#[tokio::test]
#[ignore = "just plugin-machine-install-reader-conformance; immutable readers and isolated network required"]
async fn immutable_machine_installation_readers() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_MACHINE_INSTALL_READER_MATRIX")?);
    let output = PathBuf::from(std::env::var("COWBOY_TEST_MACHINE_INSTALL_READER_RECEIPT")?);
    ensure!(
        output.is_absolute() && output.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let mut receipt = Receipt {
        schema: "dravengarden.cowboy.machine-install-reader-conformance/v1",
        source_revision: manifest::clean_revision()?,
        artifacts: serde_json::from_slice::<manifest::MachineMatrix>(&std::fs::read(matrix)?)?
            .resolve()?,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_active_recovery_and_cold_configuration",
            "installation_effects_and_independent_restoration",
            "production_operator_provider_credentials_and_sessions",
            "controller_machine_coordinator_and_writer_admission",
            "production_activation_and_full_plugin_refactor",
        ],
    };
    let empty = Fixture::build(Case::Absent).await?;
    for artifact in &receipt.artifacts {
        for case in InstallCase::ALL {
            let root = tempfile::tempdir()?;
            let setup = seed(root.path(), &empty, &receipt.ssh_keygen.path, case).await;
            for cold_read in 1..=2 {
                let result = if setup.is_ok() {
                    probe::machine_installation_reader(artifact, root.path(), case).await
                } else {
                    Err(Failure::Setup)
                };
                eprintln!(
                    "machine-install/{:?}/{case:?}/{cold_read}: {result:?}",
                    artifact.role
                );
                receipt.checks.push(Check {
                    role: artifact.role,
                    case,
                    cold_read,
                    accepted: result.is_ok(),
                    failure: result.err(),
                });
            }
        }
    }
    receipt.accepted = receipt.checks.len() == 3 * InstallCase::ALL.len() * 2
        && receipt.checks.iter().all(|check| check.accepted);
    write_receipt(&output, &receipt)?;
    ensure!(
        receipt.accepted,
        "Machine installation reader matrix rejected; inspect bounded receipt"
    );
    Ok(())
}

#[tokio::test]
async fn machine_install_reader_fixtures_are_closed_and_retain_historical_evidence() -> Result<()> {
    let empty = Fixture::build(Case::Absent).await?;
    let helper = manifest::ssh_keygen()?;
    for case in InstallCase::ALL {
        let root = tempfile::tempdir()?;
        seed(root.path(), &empty, &helper.path, case).await?;
        let store = crate::machine_plugins::MachinePluginStore::new(
            &root.path().join("machine"),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        );
        if let Some(marker) = case.marker() {
            let Err(error) = store else {
                anyhow::bail!("invalid fixture accepted: {case:?}");
            };
            ensure!(
                format!("{error:#}").contains(marker),
                "wrong fixture rejection: {case:?}"
            );
        } else {
            let store = store?;
            let step = case.step();
            ensure!(
                step.matches_envelope(&case.desired()),
                "fixture envelope mismatch"
            );
            let observed = store
                .query_installation_step(&step, Some(SERVICE), MACHINE, false)
                .await;
            ensure!(!observed.admission_enabled, "fixture admitted writes");
            if let Some(expected) = case.receipt() {
                let crate::machine_protocol::plugin_install::InstallLookup::Found { receipt } =
                    observed.result
                else {
                    anyhow::bail!("fixture receipt unavailable: {case:?}");
                };
                ensure!(*receipt == expected, "fixture receipt changed");
            }
        }
    }
    Ok(())
}
