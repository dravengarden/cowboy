//! Actual Controller startup on populated install evidence, including repeat
//! cold opens. This reuses only the existing isolated process/receipt harness.
use super::*;
use crate::plugin_operation::installation::{InstallIntent, InstallPhase, InstallProblem};
use crate::store::Store;
use fixtures::{MACHINE, SERVICE};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InstallCase {
    Absent,
    Phase(InstallPhase),
    ChecksumCorrupt,
    FutureSchema,
    ForeignOwner,
}

impl InstallCase {
    pub(super) fn marker(self) -> Option<&'static str> {
        match self {
            Self::ChecksumCorrupt => Some("invalid install intent integrity"),
            Self::FutureSchema => Some("invalid install identity"),
            Self::ForeignOwner => Some("unfinished install belongs to another Service"),
            _ => None,
        }
    }

    pub(super) fn phase(self) -> Option<InstallPhase> {
        match self {
            Self::Phase(phase) => Some(phase),
            _ => None,
        }
    }
}

fn cases() -> Vec<InstallCase> {
    let mut result = vec![InstallCase::Absent];
    result.extend(
        [
            InstallPhase::Prepared,
            InstallPhase::SyncingAuthentication,
            InstallPhase::Installing,
            InstallPhase::MachineAcknowledged,
            InstallPhase::Completed,
            InstallPhase::AuthenticationPending,
            InstallPhase::Aborted,
            InstallPhase::NeedsAttention,
        ]
        .map(InstallCase::Phase),
    );
    result.extend([
        InstallCase::ChecksumCorrupt,
        InstallCase::FutureSchema,
        InstallCase::ForeignOwner,
    ]);
    result
}

pub(super) fn intent() -> InstallIntent {
    let mut intent = crate::plugin_operation::installation::fixture("immutable-reader");
    intent.service_id = SERVICE.into();
    intent.machine_id = MACHINE.into();
    intent.actor = crate::plugin_operation::Actor::Product {
        user_id: crate::product_auth::local_product_principal().user_id,
    };
    intent
}

pub(super) fn database_path(root: &Path) -> PathBuf {
    root.join("controller/store.sqlite3")
}

async fn seed(root: &Path, empty: &Fixture, helper: &Path, case: InstallCase) -> Result<()> {
    probe::seed(root, empty, helper).await?;
    if matches!(case, InstallCase::Absent) {
        return Ok(());
    }
    let url = format!("sqlite://{}", database_path(root).display());
    let store = Store::connect(&url, root.join("controller/artifacts")).await?;
    let mut intent = intent();
    if matches!(case, InstallCase::ForeignOwner) {
        intent.service_id = "foreign-service".into();
    }
    store.begin_plugin_install(&intent).await?;
    let phase = case.phase().unwrap_or(InstallPhase::Prepared);
    if phase == InstallPhase::Aborted {
        store
            .advance_plugin_install(
                &intent,
                InstallPhase::Prepared,
                phase,
                Some(InstallProblem::PreconditionsChanged),
            )
            .await?;
    } else {
        let mut from = InstallPhase::Prepared;
        for next in [
            InstallPhase::SyncingAuthentication,
            InstallPhase::Installing,
            InstallPhase::MachineAcknowledged,
        ] {
            if from == phase || phase == InstallPhase::NeedsAttention {
                break;
            }
            store
                .advance_plugin_install(&intent, from, next, None)
                .await?;
            from = next;
        }
        if phase == InstallPhase::NeedsAttention {
            store
                .advance_plugin_install(&intent, from, InstallPhase::Installing, None)
                .await?;
            from = InstallPhase::Installing;
        }
        if phase != from {
            let problem = match phase {
                InstallPhase::AuthenticationPending => {
                    Some(InstallProblem::AuthenticationSyncFailed)
                }
                InstallPhase::NeedsAttention => Some(InstallProblem::UnknownMachineOutcome),
                _ => None,
            };
            store
                .advance_plugin_install(&intent, from, phase, problem)
                .await?;
        }
    }
    let db = rusqlite::Connection::open(database_path(root))?;
    match case {
        InstallCase::ChecksumCorrupt => {
            db.execute(
                "UPDATE plugin_install_operations SET intent_sha256 = 'invalid'",
                [],
            )?;
        }
        InstallCase::FutureSchema => {
            intent.schema = 99;
            let document = serde_json::to_string(&intent)?;
            db.execute(
                "UPDATE plugin_install_operations SET intent = ?1, intent_sha256 = ?2",
                [&document, &sha256(document.as_bytes())],
            )?;
        }
        _ => {}
    }
    Ok(())
}

#[derive(Serialize)]
struct InstallCheck {
    role: Role,
    case: InstallCase,
    cold_read: u8,
    accepted: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct InstallReceipt {
    schema: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    checks: Vec<InstallCheck>,
    accepted: bool,
    not_checked: [&'static str; 5],
}

#[tokio::test]
#[ignore = "just plugin-install-reader-conformance; immutable inputs and isolated loopback required"]
async fn immutable_installation_readers() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_INSTALL_READER_MATRIX")?);
    let path = PathBuf::from(std::env::var("COWBOY_TEST_INSTALL_READER_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let revision = manifest::clean_revision()?;
    let artifacts =
        serde_json::from_slice::<manifest::ControllerMatrix>(&std::fs::read(matrix)?)?.resolve()?;
    let empty = Fixture::build(Case::Absent).await?;
    let helper = manifest::ssh_keygen()?;
    let mut receipt = InstallReceipt {
        schema: "dravengarden.cowboy.install-reader-conformance/v1",
        source_revision: revision,
        artifacts,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_active_recovery_cold_configuration",
            "postgres_process_startup",
            "machine_installation_receipts_and_restoration",
            "production_operator_provider_credentials_and_sessions",
            "production_activation_and_full_plugin_refactor",
        ],
    };
    for artifact in &receipt.artifacts {
        for case in cases() {
            let root = tempfile::tempdir()?;
            let setup = seed(root.path(), &empty, &helper.path, case).await;
            let mut previous = None;
            for cold_read in 1..=2 {
                let result = if setup.is_ok() {
                    probe::installation_reader(artifact, &empty, root.path(), case).await
                } else {
                    Err(Failure::Setup)
                };
                let result = result.and_then(|snapshot| {
                    if previous.as_ref().is_some_and(|before| before != &snapshot) {
                        return Err(Failure::EvidenceChanged);
                    }
                    previous = Some(snapshot);
                    Ok(())
                });
                eprintln!(
                    "install/{:?}/{case:?}/{cold_read}: {result:?}",
                    artifact.role
                );
                receipt.checks.push(InstallCheck {
                    role: artifact.role,
                    case,
                    cold_read,
                    accepted: result.is_ok(),
                    failure: result.err(),
                });
            }
        }
    }
    receipt.accepted = receipt.checks.len() == 3 * cases().len() * 2
        && receipt.checks.iter().all(|check| check.accepted);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "immutable install reader matrix failed; inspect bounded receipt"
    );
    Ok(())
}
