//! Installation fixtures reuse only the isolated process/authentication tools.
//! No production credential, destination policy or Plugin installation is used.
use super::*;
use crate::machine_plugins::MachinePluginStore;
use crate::machine_protocol::{DesiredPlugin, Platform};
use base64::Engine as _;
use futures::{StreamExt as _, stream};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Flow {
    InstallAndReinstall,
    LostReceipt,
    DisconnectAfterApplied,
    DisconnectBeforeDelivery,
    ControllerCrashAfterApplied,
}

impl Flow {
    pub const ALL: [Self; 5] = [
        Self::InstallAndReinstall,
        Self::LostReceipt,
        Self::DisconnectAfterApplied,
        Self::DisconnectBeforeDelivery,
        Self::ControllerCrashAfterApplied,
    ];

    pub fn attempts(self) -> usize {
        if self == Self::InstallAndReinstall {
            2
        } else {
            1
        }
    }
}

pub(super) const FIRST: &str = "connected-install-first";
pub(super) const SECOND: &str = "connected-install-second";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct WireCounts {
    pub connections: u32,
    pub runtime_configurations: u32,
    pub target_queries: u32,
    pub target_receipts: u32,
    pub steps_observed: u32,
    pub steps_forwarded: u32,
    pub receipts_observed: u32,
    pub receipts_forwarded: u32,
    pub dropped_receipts: u32,
    pub forced_disconnects: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    #[default]
    Setup,
    Authentication,
    Installation,
    WriterEvidence,
    Copy,
    ReaderStart,
    History,
    Duplicate,
    Reopen,
    Cleanup,
    Complete,
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct WriterReport {
    pub wire: WireCounts,
    pub elapsed_ms: u64,
    pub last_http: Option<connected::HttpObservation>,
    pub package_sha256: String,
    pub release_sha256: String,
    pub service_sha256: Option<String>,
    pub machine_sha256: Option<String>,
}

#[derive(Serialize)]
pub(super) struct Check {
    pub controller_role: Role,
    pub machine_role: Role,
    pub flow: Flow,
    pub stage: Stage,
    pub writer: WriterReport,
    pub reader_wire: WireCounts,
    pub cold_reads: u8,
    pub normalized_service_sha256: Option<String>,
    pub accepted: bool,
    pub failure: Option<Failure>,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "just plugin-install-connected-conformance; isolated immutable writers/readers required"]
async fn immutable_connected_installation() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = PathBuf::from(std::env::var("COWBOY_TEST_INSTALL_CONNECTED_MATRIX")?);
    let path = PathBuf::from(std::env::var("COWBOY_TEST_INSTALL_CONNECTED_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let revision = manifest::clean_revision()?;
    let matrix: Matrix = serde_json::from_slice(&std::fs::read(matrix)?)?;
    let mut receipt = Receipt {
        schema: 1,
        purpose: "supplied_immutable_connected_installation_writer_and_reader_matrix",
        source_revision: revision,
        artifacts: matrix.resolve()?,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_roles_complete_production_configuration_and_activation",
            "production_operator_credentials_registration_and_postgres_startup",
            "agent_auth_projection_provider_sessions_and_native_generation_upgrade",
            "post_effect_compensation_and_independently_authorized_recovery",
            "physical_filesystem_power_loss_and_arbitrary_network_faults",
            "telemetry_policy_export_activation_and_full_plugin_refactor",
        ],
    };
    let artifacts = &receipt.artifacts;
    let helper = &receipt.ssh_keygen.path;
    let mut results: Vec<_> = stream::iter(Flow::ALL.into_iter().enumerate())
        .map(|(index, flow)| async move {
            (
                index,
                probe::installation_connected(artifacts, flow, helper).await,
            )
        })
        .buffer_unordered(3)
        .collect()
        .await;
    results.sort_by_key(|(index, _)| *index);
    receipt.checks = results.into_iter().flat_map(|(_, checks)| checks).collect();
    receipt.accepted = receipt.checks.len() == 3 * 3 * Flow::ALL.len()
        && receipt
            .checks
            .iter()
            .all(|check| check.accepted && check.cold_reads == 2);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "connected installation matrix failed; inspect bounded receipt"
    );
    Ok(())
}

#[derive(Clone)]
pub(super) struct InstallFixture {
    pub reader: Fixture,
    pub desired: DesiredPlugin,
    pub password: String,
    pub package_sha256: String,
    pub release_sha256: String,
}

impl InstallFixture {
    pub async fn seed(root: &Path, helper: &Path) -> Result<Self> {
        let reader = Fixture::build(Case::Absent).await?;
        probe::seed(root, &reader, helper).await?;
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher"))?;
        let desired = crate::machine_plugins::telemetry_release_for_test(&publisher, "1.1.0");
        let package = base64::engine::general_purpose::STANDARD.decode(&desired.package_base64)?;
        let release = serde_json::to_vec(&desired.release)?;
        std::fs::create_dir_all(root.join("catalog/trusted-publishers"))?;
        probe::private_write(
            &root
                .join("catalog/trusted-publishers")
                .join(format!("{}.pub", desired.release.publisher)),
            desired.publisher_public_key.as_bytes(),
        )?;
        probe::private_write(&root.join("catalog/victoria.cowboy-plugin"), &package)?;
        probe::private_write(&root.join("catalog/victoria.release.json"), &release)?;
        let machine =
            MachinePluginStore::new(&root.join("machine"), Platform::Linux, "x86_64".into())?;
        machine.enable_installation_tracking().await?;
        let password = connected::seed_operator(root, &machine).await?;
        Ok(Self {
            reader,
            desired,
            password,
            package_sha256: sha256(&package),
            release_sha256: sha256(&release),
        })
    }
}

#[tokio::test]
async fn installation_fixture_starts_vacant_with_real_enrollment_and_no_export_policy() {
    use crate::machine_protocol::plugin_install::{
        InstallTarget, InstallTargetObservation, InstallTargetQuery,
    };
    use fixtures::{MACHINE, SERVICE};
    let root = tempfile::tempdir().unwrap();
    let helper = manifest::ssh_keygen().unwrap();
    let fixture = InstallFixture::seed(root.path(), &helper.path)
        .await
        .unwrap();
    assert!(fixture.reader.document.is_none());
    assert_eq!(
        fixture.package_sha256,
        sha256(&std::fs::read(root.path().join("catalog/victoria.cowboy-plugin")).unwrap())
    );
    assert_eq!(
        fixture.release_sha256,
        sha256(&serde_json::to_vec(&fixture.desired.release).unwrap())
    );
    let store = crate::store::Store::connect(
        &format!(
            "sqlite://{}",
            root.path().join("controller/store.sqlite3").display()
        ),
        root.path().join("controller/artifacts"),
    )
    .await
    .unwrap();
    let user = store
        .user_by_username("connected-operator")
        .await
        .unwrap()
        .unwrap();
    assert!(crate::product_auth::verify_password(
        &fixture.password,
        &user.password_hash
    ));
    assert!(store.machine_public_key(MACHINE).await.unwrap().is_some());
    let machine = MachinePluginStore::new(
        &root.path().join("machine"),
        Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let query = InstallTargetQuery {
        schema: 1,
        service_id: SERVICE.into(),
        machine_id: MACHINE.into(),
        plugin_id: fixture.desired.release.plugin_id,
    };
    assert!(matches!(
        machine
            .installation_target(&query, Some(SERVICE), MACHINE, false)
            .await,
        InstallTargetObservation::Observed {
            target: InstallTarget::Vacant {},
            admission_enabled: false,
            ..
        }
    ));
    for path in [
        "machine/plugin-operations/install-attempts-v1",
        "machine/telemetry.json",
        "machine/telemetry-writer-policy.json",
        "controller-writer.json",
    ] {
        assert!(
            matches!(root.path().join(path).symlink_metadata(), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        );
    }
}
