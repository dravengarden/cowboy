//! Two real release processes, genuine fixture login and transparent fault proxy.
//! This accepts supplied artifact pairs, never production credentials or cutover.
use super::*;
use futures::{StreamExt as _, stream};

mod fixture;
pub(super) use fixture::{ConnectedFixture, Evidence};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Flow {
    BindingRoundTrip,
    BindingLostAck,
    BindingDisconnected,
    PreparedRecovery,
}

impl Flow {
    const ALL: [Self; 4] = [
        Self::BindingRoundTrip,
        Self::BindingLostAck,
        Self::BindingDisconnected,
        Self::PreparedRecovery,
    ];
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    #[default]
    Setup,
    Start,
    Authentication,
    Preview,
    Confirmation,
    ConnectionReplacement,
    Recovery,
    Resolution,
    Reopen,
    Cleanup,
    Complete,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct WireCounts {
    pub protocol: Option<u16>,
    pub connections: u32,
    pub binding_commands: u32,
    pub binding_queries: u32,
    pub recovery_commands: u32,
    pub recovery_queries: u32,
    pub audit_queries: u32,
    pub dropped_binding_acks: u32,
    pub dropped_recovery_acks: u32,
    pub forced_disconnects: u32,
}

#[derive(Default, Serialize)]
pub(super) struct Outcome {
    pub stage: Stage,
    pub wire: WireCounts,
    pub fixture_package_sha256: Option<String>,
    pub fixture_release_sha256: Option<String>,
    pub installation_sha256: Option<String>,
    pub service_before_sha256: Option<String>,
    pub machine_before_sha256: Option<String>,
    pub service_after_sha256: Option<String>,
    pub machine_after_sha256: Option<String>,
    pub failure: Option<Failure>,
}

#[derive(Serialize)]
struct Check {
    controller_role: Role,
    machine_role: Role,
    flow: Flow,
    accepted: bool,
    outcome: Outcome,
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
    not_checked: [&'static str; 5],
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "just telemetry-connected-conformance; isolated immutable inputs required"]
async fn immutable_connected_telemetry() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_CONNECTED_MATRIX")?);
    let path = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_CONNECTED_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let revision = manifest::clean_revision()?;
    let matrix: Matrix = serde_json::from_slice(&std::fs::read(matrix)?)?;
    let mut receipt = Receipt {
        schema: 1,
        purpose: "supplied_immutable_connected_telemetry_matrix",
        source_revision: revision,
        artifacts: matrix.resolve()?,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_roles_and_complete_production_configuration",
            "production_operator_credentials_registration_and_postgres_startup",
            "external_otlp_delivery_and_managed_background_cutover",
            "physical_filesystem_power_loss_and_arbitrary_network_faults",
            "native_clients_provider_sessions_and_host_activation",
        ],
    };
    let mut inputs = Vec::new();
    for controller in receipt
        .artifacts
        .iter()
        .filter(|a| a.lane == Lane::Controller)
    {
        for machine in receipt.artifacts.iter().filter(|a| a.lane == Lane::Machine) {
            for flow in Flow::ALL {
                inputs.push((inputs.len(), controller, machine, flow));
            }
        }
    }
    // Three isolated pairs at a time; real 45s/15s ACK deadlines stay unchanged.
    let helper = &receipt.ssh_keygen.path;
    let mut results: Vec<_> = stream::iter(inputs)
        .map(|(index, controller, machine, flow)| async move {
            let outcome = probe::connected_pair(controller, machine, flow, helper).await;
            let accepted = outcome.failure.is_none();
            eprintln!(
                "{:?}/{:?}/{flow:?}: {:?}/{:?}",
                controller.role, machine.role, outcome.stage, outcome.failure
            );
            (
                index,
                Check {
                    controller_role: controller.role,
                    machine_role: machine.role,
                    flow,
                    accepted,
                    outcome,
                },
            )
        })
        .buffer_unordered(3)
        .collect()
        .await;
    results.sort_by_key(|(index, _)| *index);
    receipt.checks = results.into_iter().map(|(_, check)| check).collect();
    receipt.accepted = receipt.checks.len() == 3 * 3 * Flow::ALL.len()
        && receipt.checks.iter().all(|check| check.accepted);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "connected telemetry matrix failed; inspect bounded receipt"
    );
    Ok(())
}
