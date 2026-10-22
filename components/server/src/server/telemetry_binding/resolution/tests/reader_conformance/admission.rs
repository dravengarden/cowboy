//! Separate executable policy/effect gate. Synthetic local Operator and signed
//! Machine peer only; never opens a production writer or remote destination.
use super::*;
use fixtures::{MACHINE, SERVICE};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Scenario {
    ServiceResolution,
    MachineBinding,
    MachineRecovery,
}

impl Scenario {
    const ALL: [Self; 3] = [
        Self::ServiceResolution,
        Self::MachineBinding,
        Self::MachineRecovery,
    ];

    fn lane(self) -> Lane {
        if self == Self::ServiceResolution {
            Lane::Controller
        } else {
            Lane::Machine
        }
    }

    pub fn purpose(self) -> u8 {
        match self {
            Self::MachineBinding => 1,
            Self::MachineRecovery => 2,
            Self::ServiceResolution => 4,
        }
    }

    fn cold_reads(self) -> u8 {
        // The Service must first lose an *unsubmitted* preview, then reopen a
        // separately confirmed durable result. Machines reopen exact receipts.
        if self == Self::ServiceResolution {
            3
        } else {
            2
        }
    }

    async fn fixture(self) -> Result<Fixture> {
        if self == Self::MachineRecovery {
            return Fixture::build(Case::Prepared).await;
        }
        let mut f = Fixture::build(Case::Absent).await?;
        if self == Self::ServiceResolution {
            let intent = crate::telemetry_binding::Intent {
                schema: 2,
                operation_id: f.step.operation_id.clone(),
                service_id: SERVICE.into(),
                machine_id: MACHINE.into(),
                actor: crate::plugin_operation::Actor::Product {
                    user_id: crate::product_auth::local_product_principal().user_id,
                },
                expected: None,
                change: f.step.change.clone(),
                expires_at_ms: f.step.expires_at_ms,
            };
            ensure!(
                intent.machine_step()? == f.step,
                "exact fixture intent required"
            );
            let mut ledger = None;
            crate::telemetry_binding::writer::apply(
                &mut ledger,
                &crate::telemetry_binding::writer::Change::Begin(&intent),
            )?;
            f.document = Some(ledger.unwrap().encode(SERVICE)?);
            f.case = Case::Prepared;
        }
        Ok(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PolicyCase {
    Absent,
    Closed,
    Binding,
    Recovery,
    Resolution,
    BindingRecovery,
    BindingResolution,
    RecoveryResolution,
    All,
    ForeignService,
    ForeignMachine,
    InvalidSchema,
    PublicFile,
    Symlink,
}

impl PolicyCase {
    const ALL: [Self; 14] = [
        Self::Absent,
        Self::Closed,
        Self::Binding,
        Self::Recovery,
        Self::Resolution,
        Self::BindingRecovery,
        Self::BindingResolution,
        Self::RecoveryResolution,
        Self::All,
        Self::ForeignService,
        Self::ForeignMachine,
        Self::InvalidSchema,
        Self::PublicFile,
        Self::Symlink,
    ];

    fn bits(self) -> u8 {
        match self {
            Self::Absent | Self::Closed => 0,
            Self::Binding => 1,
            Self::Recovery => 2,
            Self::Resolution => 4,
            Self::BindingRecovery => 3,
            Self::BindingResolution => 5,
            Self::RecoveryResolution => 6,
            _ => 7,
        }
    }

    pub fn admits(self, purpose: u8) -> bool {
        !matches!(self, Self::ForeignService | Self::ForeignMachine)
            && self.failure_marker(Lane::Machine).is_none()
            && self.bits() & purpose != 0
    }

    pub fn startup_binding(self) -> bool {
        // Controller startup reports Service admission without a Machine caller.
        self.bits() & 1 != 0
    }

    pub fn bytes(self) -> Option<Vec<u8>> {
        (self != Self::Absent).then(|| serde_json::to_vec(&json!({
            "schema": if self == Self::InvalidSchema { 2 } else { 1 },
            "service_id": if self == Self::ForeignService { "svc-00000000000000000000000000000002" } else { SERVICE },
            "machine_id": if self == Self::ForeignMachine { "foreign-machine" } else { MACHINE },
            "purposes": {"binding": self.bits() & 1 != 0, "machine_recovery": self.bits() & 2 != 0, "service_resolution": self.bits() & 4 != 0},
            "legacy_fence": "retain_managed_namespace",
        })).unwrap())
    }

    pub fn failure_marker(self, lane: Lane) -> Option<&'static str> {
        match self {
            Self::InvalidSchema => Some("invalid telemetry writer policy owner or schema"),
            Self::PublicFile => {
                Some("telemetry configuration must be an owned private regular file")
            }
            Self::Symlink => Some("opening private telemetry configuration"),
            Self::ForeignService if lane == Lane::Controller => {
                Some("telemetry writer policy requires its exact durable Service")
            }
            _ => None,
        }
    }

    pub fn write(self, path: &Path) -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        if let Some(bytes) = self.bytes() {
            if self == Self::Symlink {
                let target = path.with_extension("target");
                probe::private_write(&target, &bytes)?;
                symlink(target, path)?;
            } else {
                probe::private_write(path, &bytes)?;
                if self == Self::PublicFile {
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))?;
                }
            }
        }
        Ok(())
    }
}

pub(super) struct State {
    pub evidence: Fixture,
    pub confirmation: Option<Value>,
    pub receipt: Option<Value>,
}

#[derive(Serialize)]
struct AdmissionCheck {
    role: Role,
    scenario: Scenario,
    policy: PolicyCase,
    cold_read: u8,
    policy_fixture_sha256: Option<String>,
    service_before_sha256: Option<String>,
    service_after_sha256: Option<String>,
    machine_before_sha256: Option<String>,
    machine_after_sha256: Option<String>,
    machine_protocol: Option<u16>,
    accepted: bool,
    failure: Option<Failure>,
}

#[derive(Serialize)]
struct AdmissionReceipt {
    schema: u16,
    purpose: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    ssh_keygen: manifest::Executable,
    checks: Vec<AdmissionCheck>,
    accepted: bool,
    not_checked: [&'static str; 6],
}

#[tokio::test]
#[ignore = "just telemetry-writer-conformance; immutable isolated inputs required"]
async fn immutable_writer_admission() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_WRITER_MATRIX")?);
    let path = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_WRITER_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let revision = manifest::clean_revision()?;
    let matrix: Matrix = serde_json::from_slice(&std::fs::read(matrix)?)?;
    let mut receipt = AdmissionReceipt {
        schema: 1,
        purpose: "supplied_immutable_telemetry_writer_policy_matrix",
        source_revision: revision,
        artifacts: matrix.resolve()?,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_roles_and_complete_production_configuration",
            "production_operator_credentials_and_postgres_startup",
            "connected_service_machine_coordination_and_network_faults",
            "signed_plugin_selection_and_external_otlp_delivery",
            "native_clients_provider_auth_and_existing_sessions",
            "production_policy_cutover_and_host_activation",
        ],
    };
    for artifact in &receipt.artifacts {
        for scenario in Scenario::ALL
            .into_iter()
            .filter(|s| s.lane() == artifact.lane)
        {
            for policy in PolicyCase::ALL {
                let root = tempfile::tempdir()?;
                let mut state = State {
                    evidence: scenario.fixture().await?,
                    confirmation: None,
                    receipt: None,
                };
                let mut ready = probe::seed(root.path(), &state.evidence, &receipt.ssh_keygen.path)
                    .await
                    .is_ok();
                for cold_read in 1..=scenario.cold_reads() {
                    let service_before_sha256 = state
                        .evidence
                        .document
                        .as_deref()
                        .map(|s| sha256(s.as_bytes()));
                    let machine_before_sha256 = state.evidence.machine.as_deref().map(sha256);
                    let result = if ready {
                        probe::writer_admission(
                            artifact,
                            scenario,
                            policy,
                            &mut state,
                            root.path(),
                            cold_read,
                        )
                        .await
                    } else {
                        Err(Failure::Setup)
                    };
                    ready = result.is_ok();
                    eprintln!(
                        "{:?}/{scenario:?}/{policy:?}/{cold_read}: {result:?}",
                        artifact.role
                    );
                    receipt.checks.push(AdmissionCheck {
                        role: artifact.role,
                        scenario,
                        policy,
                        cold_read,
                        policy_fixture_sha256: policy.bytes().as_deref().map(sha256),
                        service_before_sha256,
                        machine_before_sha256,
                        service_after_sha256: state
                            .evidence
                            .document
                            .as_deref()
                            .map(|s| sha256(s.as_bytes())),
                        machine_after_sha256: state.evidence.machine.as_deref().map(sha256),
                        machine_protocol: result.as_ref().ok().copied().flatten(),
                        accepted: result.is_ok(),
                        failure: result.err(),
                    });
                }
            }
        }
    }
    let required_reads: usize = Scenario::ALL
        .iter()
        .map(|s| usize::from(s.cold_reads()))
        .sum();
    receipt.accepted = receipt.checks.len() == 3 * PolicyCase::ALL.len() * required_reads
        && receipt.checks.iter().all(|c| c.accepted);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "immutable telemetry writer matrix failed; inspect bounded receipt"
    );
    Ok(())
}

#[test]
fn policy_fixtures_cover_all_purposes_and_private_startup_failures() {
    use crate::telemetry_plugin::writer_admission::{
        BindingWrites, MachineRecovery, ServiceResolution, WriterAdmission,
    };
    let mut bits = std::collections::BTreeSet::new();
    for case in PolicyCase::ALL {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("policy.json");
        case.write(&path).unwrap();
        let loaded = WriterAdmission::load_optional(&path);
        if case.failure_marker(Lane::Machine).is_some() {
            assert!(loaded.is_err());
        } else if let Some(policy) = loaded.unwrap() {
            assert_eq!(
                policy.allows::<BindingWrites>(SERVICE, Some(MACHINE)),
                case.admits(1)
            );
            assert_eq!(
                policy.allows::<MachineRecovery>(SERVICE, Some(MACHINE)),
                case.admits(2)
            );
            assert_eq!(
                policy.allows::<ServiceResolution>(SERVICE, Some(MACHINE)),
                case.admits(4)
            );
            if !matches!(
                case,
                PolicyCase::ForeignService | PolicyCase::ForeignMachine
            ) {
                bits.insert(case.bits());
            }
        } else {
            assert_eq!(case, PolicyCase::Absent);
        }
    }
    assert_eq!(bits, (0..8).collect());
}

#[tokio::test]
async fn resolution_fixture_is_only_local_prepared_and_machine_has_no_namespace() {
    let f = Scenario::ServiceResolution.fixture().await.unwrap();
    let ledger =
        crate::telemetry_binding::Ledger::decode(f.document.as_deref().unwrap(), SERVICE).unwrap();
    assert_eq!(ledger.operations.len(), 1);
    assert_eq!(
        ledger.operations[0].progress,
        crate::telemetry_binding::Progress::Prepared
    );
    assert!(ledger.current.is_none() && ledger.resolutions.is_empty() && f.machine.is_none());
}
