//! Startup configuration acceptance is separate from populated reader support.
//! All policy and journal bytes here are synthetic, never production inputs.
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingChange, BindingOutcome};
use crate::telemetry_binding::{
    Attention, Intent, Ledger, Progress,
    tests::{applied, observed},
    writer::{Change, apply},
};
use fixtures::{MACHINE, SERVICE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StartupCase {
    Unconfigured,
    Absent,
    Exact,
    Prepared,
    Unknown,
    Aborted,
    Revoked,
    Restored,
    Advanced,
    ChecksumCorrupt,
    AuditCorrupt,
    InvalidPolicy,
    ConflictingModes,
}

impl StartupCase {
    const ALL: [Self; 13] = [
        Self::Unconfigured,
        Self::Absent,
        Self::Exact,
        Self::Prepared,
        Self::Unknown,
        Self::Aborted,
        Self::Revoked,
        Self::Restored,
        Self::Advanced,
        Self::ChecksumCorrupt,
        Self::AuditCorrupt,
        Self::InvalidPolicy,
        Self::ConflictingModes,
    ];

    pub fn export_active(self) -> bool {
        matches!(self, Self::Exact | Self::Aborted)
    }

    pub fn failure_marker(self) -> Option<&'static str> {
        match self {
            Self::ChecksumCorrupt => Some("invalid Service binding checksum or capacity"),
            Self::AuditCorrupt => Some("invalid binding resolution audit"),
            Self::InvalidPolicy => Some("invalid managed telemetry policy owner or schema"),
            Self::ConflictingModes => Some("cannot be used with"),
            _ => None,
        }
    }
}

pub(super) struct StartupFixture {
    pub case: StartupCase,
    pub evidence: Fixture,
    pub policy: Option<Vec<u8>>,
}

fn complete(ledger: &mut Option<Ledger>, intent: &Intent) -> Result<()> {
    let prepared = apply(ledger, &Change::Begin(intent))?.operation;
    let dispatching = apply(
        ledger,
        &Change::Advance {
            expected: &prepared,
            progress: Progress::Dispatching,
        },
    )?
    .operation;
    apply(
        ledger,
        &Change::Advance {
            expected: &dispatching,
            progress: Progress::Completed {
                observation: applied(intent),
            },
        },
    )?;
    Ok(())
}

impl StartupFixture {
    async fn build(case: StartupCase) -> Result<Self> {
        let mut intent = crate::telemetry_binding::fixture("background-startup");
        intent.schema = 2;
        intent.service_id = SERVICE.into();
        intent.machine_id = MACHINE.into();
        let binding = intent.machine_step()?.after()?;
        let mut ledger = None;
        if case != StartupCase::Absent {
            complete(&mut ledger, &intent)?;
        }
        let mut next = intent.clone();
        next.operation_id = "background-startup-next".into();
        next.expected = Some(binding.clone());
        next.change = BindingChange::Revoke {
            policy_epoch: "2".to_owned().try_into().unwrap(),
        };
        match case {
            StartupCase::Prepared | StartupCase::Unknown | StartupCase::Aborted => {
                let pending = apply(&mut ledger, &Change::Begin(&next))?.operation;
                if case != StartupCase::Prepared {
                    let progress = if case == StartupCase::Aborted {
                        Progress::Aborted
                    } else {
                        Progress::NeedsAttention {
                            reason: Attention::Uncertain,
                            observation: Some(observed(&next, BindingOutcome::Unknown {})),
                        }
                    };
                    apply(
                        &mut ledger,
                        &Change::Advance {
                            expected: &pending,
                            progress,
                        },
                    )?;
                }
            }
            StartupCase::Revoked | StartupCase::Restored => {
                complete(&mut ledger, &next)?;
                if case == StartupCase::Restored {
                    let mut restore = next.clone();
                    restore.operation_id = "background-startup-restore".into();
                    restore.expected = Some(next.machine_step()?.after()?);
                    restore.change = BindingChange::Restore {
                        forward_request_digest: next.machine_step()?.request_digest()?,
                        selection: binding.selection.clone(),
                        policy_epoch: "3".to_owned().try_into().unwrap(),
                    };
                    complete(&mut ledger, &restore)?;
                }
            }
            StartupCase::Advanced => {
                next.change = intent.change.clone();
                let BindingChange::Select { policy_epoch, .. } = &mut next.change else {
                    anyhow::bail!("selected fixture required");
                };
                *policy_epoch = "2".to_owned().try_into().unwrap();
                complete(&mut ledger, &next)?;
            }
            _ => {}
        }
        let mut evidence = if case == StartupCase::AuditCorrupt {
            Fixture::build(Case::AuditCorrupt).await?
        } else {
            Fixture::build(Case::Absent).await?
        };
        if case != StartupCase::AuditCorrupt {
            evidence.document = ledger.map(|l| l.encode(SERVICE)).transpose()?;
            evidence.case = if case == StartupCase::ChecksumCorrupt {
                Case::ChecksumCorrupt
            } else if evidence.document.is_some() {
                Case::Completed
            } else {
                Case::Absent
            };
        }
        let policy = (case != StartupCase::Unconfigured)
            .then(|| {
                serde_json::to_vec(&serde_json::json!({
                    "schema": if case == StartupCase::InvalidPolicy { 2 } else { 1 },
                    "service_id": SERVICE, "machine_id": MACHINE, "binding": binding,
                    "signals": {"logs": true, "metrics": true, "traces": true},
                    "startup": "activate_exact_binding"
                }))
            })
            .transpose()?;
        Ok(Self {
            case,
            evidence,
            policy,
        })
    }
}

#[derive(Serialize)]
struct StartupCheck {
    role: Role,
    case: StartupCase,
    cold_read: u8,
    service_fixture_sha256: Option<String>,
    policy_fixture_sha256: Option<String>,
    /// None for a correctly rejected startup. Otherwise the observed optional
    /// queue state, checked with real local OTLP intake and bounded counters.
    export_active: Option<bool>,
    accepted: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct StartupReceipt {
    schema: u16,
    purpose: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    ssh_keygen: manifest::Executable,
    checks: Vec<StartupCheck>,
    accepted: bool,
    not_checked: [&'static str; 6],
}

#[tokio::test]
#[ignore = "just telemetry-background-startup-conformance; immutable isolated inputs required"]
async fn immutable_background_startup() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_STARTUP_MATRIX")?);
    let path = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_STARTUP_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let revision = manifest::clean_revision()?;
    let matrix: manifest::ControllerMatrix = serde_json::from_slice(&std::fs::read(matrix)?)?;
    let mut receipt = StartupReceipt {
        schema: 1,
        purpose: "supplied_immutable_managed_background_startup_matrix",
        source_revision: revision,
        artifacts: matrix.resolve()?,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_roles_and_complete_production_configuration",
            "production_data_credentials_and_postgres_startup",
            "machine_startup_and_cross_end_external_otlp_delivery",
            "writer_admission_and_operator_confirmation",
            "native_clients_provider_auth_and_existing_sessions",
            "host_activation_and_full_plugin_refactor",
        ],
    };
    for artifact in &receipt.artifacts {
        for case in StartupCase::ALL {
            let fixture = StartupFixture::build(case).await?;
            let root = tempfile::tempdir()?;
            let setup = probe::seed(root.path(), &fixture.evidence, &receipt.ssh_keygen.path).await;
            for cold_read in 1..=2 {
                let result = if setup.is_ok() {
                    probe::background_startup(artifact, &fixture, root.path(), cold_read).await
                } else {
                    Err(Failure::Setup)
                };
                eprintln!("{:?}/{case:?}/{cold_read}: {result:?}", artifact.role);
                receipt.checks.push(StartupCheck {
                    role: artifact.role,
                    case,
                    cold_read,
                    service_fixture_sha256: fixture
                        .evidence
                        .document
                        .as_ref()
                        .map(|s| sha256(s.as_bytes())),
                    policy_fixture_sha256: fixture.policy.as_deref().map(sha256),
                    export_active: result.as_ref().ok().copied().flatten(),
                    accepted: result.is_ok(),
                    failure: result.err(),
                });
            }
        }
    }
    receipt.accepted = receipt.checks.len() == 3 * StartupCase::ALL.len() * 2
        && receipt.checks.iter().all(|check| check.accepted);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "managed background startup matrix failed; inspect bounded receipt"
    );
    Ok(())
}

#[tokio::test]
async fn background_startup_fixtures_distinguish_core_integrity_from_optional_egress() -> Result<()>
{
    use crate::telemetry_plugin::background_policy::{Activation, BackgroundPolicy};
    for case in StartupCase::ALL {
        let fixture = StartupFixture::build(case).await?;
        let root = tempfile::tempdir()?;
        probe::seed(
            root.path(),
            &fixture.evidence,
            Path::new("/unused-fixture-helper"),
        )
        .await?;
        let store = crate::store::Store::connect(
            &format!(
                "sqlite://{}",
                root.path().join("controller/store.sqlite3").display()
            ),
            root.path().join("controller/artifacts"),
        )
        .await?;
        let fence = crate::telemetry_binding::LegacyFence::recover(Some(&store), SERVICE).await;
        if matches!(
            case,
            StartupCase::ChecksumCorrupt | StartupCase::AuditCorrupt
        ) {
            ensure!(fence.is_err(), "corruption must still fail core recovery");
            continue;
        }
        ensure!(
            fence?.allows_legacy() == (case == StartupCase::Absent),
            "fixture namespace"
        );
        if let Some(bytes) = &fixture.policy {
            use std::os::unix::fs::PermissionsExt as _;
            let path = root.path().join("policy.json");
            std::fs::write(&path, bytes)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            let policy = BackgroundPolicy::load(&path, SERVICE);
            if case == StartupCase::InvalidPolicy {
                ensure!(
                    policy.is_err(),
                    "invalid configuration must not be silently ignored"
                );
            } else if case != StartupCase::ConflictingModes {
                let active = matches!(policy?.activate(&store).await, Activation::Active(_));
                ensure!(
                    active == case.export_active(),
                    "wrong fixture activation: {case:?}"
                );
            }
        }
    }
    Ok(())
}
