//! Installation fixtures reuse only the isolated process/authentication tools.
//! No production credential, destination policy or Plugin installation is used.
use super::*;
use crate::machine_plugins::MachinePluginStore;
use crate::machine_protocol::{DesiredPlugin, Platform};
use base64::Engine as _;

struct InstallFixture {
    reader: Fixture,
    desired: DesiredPlugin,
    password: String,
    package_sha256: String,
    release_sha256: String,
}

impl InstallFixture {
    async fn seed(root: &Path, helper: &Path) -> Result<Self> {
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
