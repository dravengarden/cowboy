//! Actual Controller startup on populated install evidence, including repeat
//! cold opens. This reuses only the existing isolated process/receipt harness.
use super::*;
use crate::machine_protocol::plugin_install::{
    InstallOutcome, InstallPhase as MachinePhase, InstallReceipt as MachineReceipt,
    InstallRejection, InstallTarget, InstallUncertainty,
};
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
    MachinePhase(InstallPhase),
    MachineRejected,
    MachinePending,
    MachineUnknown,
    ReceiptChecksumCorrupt,
    ReceiptFutureSchema,
    ReceiptTargetMismatch,
    ReceiptMissing,
    MachineTargetMissing,
}

impl InstallCase {
    pub(super) fn marker(self) -> Option<&'static str> {
        match self {
            Self::ChecksumCorrupt => Some("invalid install intent integrity"),
            Self::FutureSchema => Some("invalid install identity"),
            Self::ForeignOwner => Some("unfinished install belongs to another Service"),
            Self::ReceiptChecksumCorrupt => Some("invalid install Machine receipt integrity"),
            Self::ReceiptFutureSchema | Self::ReceiptTargetMismatch => {
                Some("install Machine receipt identity mismatch")
            }
            Self::ReceiptMissing => Some("install progress lacks matching Machine evidence"),
            Self::MachineTargetMissing => Some("invalid install identity"),
            _ => None,
        }
    }

    pub(super) fn phase(self) -> Option<InstallPhase> {
        match self {
            Self::Phase(phase) | Self::MachinePhase(phase) => Some(phase),
            Self::MachineRejected => Some(InstallPhase::Aborted),
            Self::MachinePending | Self::MachineUnknown => Some(InstallPhase::NeedsAttention),
            Self::ReceiptChecksumCorrupt
            | Self::ReceiptFutureSchema
            | Self::ReceiptTargetMismatch
            | Self::ReceiptMissing => Some(InstallPhase::MachineAcknowledged),
            Self::MachineTargetMissing => Some(InstallPhase::Prepared),
            _ => None,
        }
    }

    pub(super) fn durable(self) -> bool {
        !matches!(
            self,
            Self::Absent
                | Self::Phase(_)
                | Self::ChecksumCorrupt
                | Self::FutureSchema
                | Self::ForeignOwner
        )
    }

    pub(super) fn outcome(self) -> Option<InstallOutcome> {
        match self {
            Self::MachineRejected => Some(InstallOutcome::Rejected {
                reason: InstallRejection::TargetChanged,
            }),
            Self::MachinePending => Some(InstallOutcome::Pending {
                phase: MachinePhase::Staging,
            }),
            Self::MachineUnknown => Some(InstallOutcome::Unknown {
                phase: MachinePhase::Activating,
                reason: InstallUncertainty::Interrupted,
            }),
            _ if self.durable()
                && matches!(
                    self.phase(),
                    Some(
                        InstallPhase::MachineAcknowledged
                            | InstallPhase::Completed
                            | InstallPhase::AuthenticationPending
                    )
                ) =>
            {
                Some(InstallOutcome::Applied {
                    revision: format!("installation-{}", "d".repeat(64))
                        .try_into()
                        .expect("fixture revision"),
                })
            }
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
        .map(InstallCase::MachinePhase),
    );
    result.extend([
        InstallCase::MachineRejected,
        InstallCase::MachinePending,
        InstallCase::MachineUnknown,
        InstallCase::ReceiptChecksumCorrupt,
        InstallCase::ReceiptFutureSchema,
        InstallCase::ReceiptTargetMismatch,
        InstallCase::ReceiptMissing,
        InstallCase::MachineTargetMissing,
    ]);
    result
}

pub(super) fn intent(case: InstallCase) -> InstallIntent {
    let mut intent = crate::plugin_operation::installation::fixture("immutable-reader");
    intent.service_id = SERVICE.into();
    intent.machine_id = MACHINE.into();
    intent.actor = crate::plugin_operation::Actor::Product {
        user_id: crate::product_auth::local_product_principal().user_id,
    };
    if case.durable() {
        intent.schema = 2;
        intent.machine_target = Some(match case.phase() {
            Some(InstallPhase::Prepared | InstallPhase::SyncingAuthentication) => {
                InstallTarget::Vacant {}
            }
            Some(InstallPhase::Installing | InstallPhase::Aborted) => InstallTarget::Removed {
                revision: format!("installation-{}", "a".repeat(64))
                    .try_into()
                    .expect("fixture revision"),
            },
            _ => InstallTarget::Installed {
                revision: format!("installation-{}", "a".repeat(64))
                    .try_into()
                    .expect("fixture revision"),
                generation_digest: format!("sha256:{}", "e".repeat(64)),
            },
        });
    }
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
    let mut intent = intent(case);
    if matches!(case, InstallCase::ForeignOwner) {
        intent.service_id = "foreign-service".into();
    }
    store.begin_plugin_install(&intent).await?;
    let phase = case.phase().unwrap_or(InstallPhase::Prepared);
    if phase == InstallPhase::Aborted && !matches!(case, InstallCase::MachineRejected) {
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
            if from == phase
                || (from == InstallPhase::Installing
                    && (phase == InstallPhase::NeedsAttention || case.outcome().is_some()))
            {
                break;
            }
            store
                .advance_plugin_install(&intent, from, next, None)
                .await?;
            from = next;
        }
        if let Some(outcome) = case.outcome() {
            let step = intent.machine_step()?;
            let receipt = MachineReceipt {
                request_digest: step.request_digest()?,
                step,
                outcome,
            };
            from = store
                .record_plugin_install_receipt(&intent, &receipt)
                .await?
                .phase;
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
    corrupt(root, case, &mut intent)?;
    Ok(())
}

fn corrupt(root: &Path, case: InstallCase, intent: &mut InstallIntent) -> Result<()> {
    let db = rusqlite::Connection::open(database_path(root))?;
    match case {
        InstallCase::ChecksumCorrupt => {
            db.execute(
                "UPDATE plugin_install_operations SET intent_sha256 = 'invalid'",
                [],
            )?;
        }
        InstallCase::FutureSchema | InstallCase::MachineTargetMissing => {
            if matches!(case, InstallCase::FutureSchema) {
                intent.schema = 99;
            } else {
                intent.machine_target = None;
            }
            let document = serde_json::to_string(&intent)?;
            db.execute(
                "UPDATE plugin_install_operations SET intent = ?1, intent_sha256 = ?2",
                [&document, &sha256(document.as_bytes())],
            )?;
        }
        InstallCase::ReceiptChecksumCorrupt => {
            db.execute(
                "UPDATE plugin_install_operations SET machine_receipt_sha256 = ?1",
                ["f".repeat(64)],
            )?;
        }
        InstallCase::ReceiptMissing => {
            db.execute("UPDATE plugin_install_operations SET machine_receipt = NULL, machine_receipt_sha256 = NULL", [])?;
        }
        InstallCase::ReceiptFutureSchema | InstallCase::ReceiptTargetMismatch => {
            let document: String = db.query_row(
                "SELECT machine_receipt FROM plugin_install_operations",
                [],
                |row| row.get(0),
            )?;
            let mut receipt: MachineReceipt = serde_json::from_str(&document)?;
            if matches!(case, InstallCase::ReceiptFutureSchema) {
                receipt.step.schema = 99;
            } else {
                receipt.step.expected = InstallTarget::Vacant {};
            }
            let document = serde_json::to_string(&receipt)?;
            db.execute("UPDATE plugin_install_operations SET machine_receipt = ?1, machine_receipt_sha256 = ?2", [&document, &sha256(document.as_bytes())])?;
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
        schema: "dravengarden.cowboy.install-reader-conformance/v2",
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

#[tokio::test]
async fn all_install_reader_fixtures_exercise_the_exact_startup_contract() -> Result<()> {
    let empty = Fixture::build(Case::Absent).await?;
    let helper = std::env::current_exe()?; // Never executed by seed or this test.
    assert_eq!(cases().len(), 28);
    for case in cases() {
        let root = tempfile::tempdir()?;
        seed(root.path(), &empty, &helper, case).await?;
        let store = Store::connect(
            &format!("sqlite://{}", database_path(root.path()).display()),
            root.path().join("controller/artifacts"),
        )
        .await?;
        let result = store.recover_plugin_installs(SERVICE).await;
        if let Some(marker) = case.marker() {
            ensure!(
                result.unwrap_err().to_string().contains(marker),
                "fixture rejection mismatch: {case:?}"
            );
        } else {
            let before = result?;
            ensure!(
                store.recover_plugin_installs(SERVICE).await? == before,
                "fixture changed after second startup: {case:?}"
            );
            if case.phase().is_some() {
                let saved = store
                    .plugin_install_operation(&intent(case).operation_id)
                    .await?
                    .expect("seeded evidence");
                ensure!(
                    saved.machine_receipt.map(|r| r.outcome) == case.outcome(),
                    "receipt differs: {case:?}"
                );
            }
        }
    }
    Ok(())
}
