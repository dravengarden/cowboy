use super::*;
use crate::machine_plugins::MachinePluginStore;
use crate::machine_protocol::{Platform, telemetry_binding::BindingInstallation};
use crate::store::{ProductUser, Store};
use crate::telemetry_binding::Ledger;
use base64::Engine as _;
use fixtures::{JOURNAL, MACHINE, SERVICE};
use rusqlite::OptionalExtension as _;
use serde_json::json;

pub(super) const ACCOUNT: &str = "connected-operator";

pub(in super::super) struct ConnectedFixture {
    pub reader: Fixture,
    pub installation: BindingInstallation,
    pub password: String,
    pub package_sha256: String,
    pub release_sha256: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(in super::super) struct Evidence {
    pub service: Option<String>,
    pub machine: Option<Vec<u8>>,
}

impl Evidence {
    pub fn read(root: &Path) -> Result<Self> {
        let db = rusqlite::Connection::open_with_flags(
            root.join("controller/store.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let row: Option<(String, String)> = db.query_row("SELECT document, document_sha256 FROM telemetry_binding_journal WHERE slot='telemetry'", [], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        let service = row
            .map(|(bytes, digest)| -> Result<_> {
                ensure!(
                    sha256(bytes.as_bytes()) == digest,
                    "Service checksum changed"
                );
                Ledger::decode(&bytes, SERVICE)?;
                Ok(bytes)
            })
            .transpose()?;
        let machine_path = root.join("machine").join(JOURNAL);
        let machine = match std::fs::read(&machine_path) {
            Ok(bytes) => Some(bytes),
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && matches!(machine_path.symlink_metadata(), Err(error) if error.kind() == std::io::ErrorKind::NotFound) =>
            {
                None
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Self { service, machine })
    }

    pub fn ledger(&self) -> Result<Ledger> {
        Ledger::decode(
            self.service
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing Service evidence"))?,
            SERVICE,
        )
    }

    pub fn matches(&self, root: &Path) -> Result<(), Failure> {
        if Self::read(root).is_ok_and(|retained| retained == *self) {
            Ok(())
        } else {
            Err(Failure::EvidenceChanged)
        }
    }
}

impl ConnectedFixture {
    pub async fn seed(root: &Path, flow: Flow, ssh_keygen: &Path) -> Result<Self> {
        let reader = Fixture::build(if flow == Flow::PreparedRecovery {
            Case::Prepared
        } else {
            Case::Absent
        })
        .await?;
        probe::seed(root, &reader, ssh_keygen).await?;
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
        let machine_root = root.join("machine");
        let machine = MachinePluginStore::new(&machine_root, Platform::Linux, "x86_64".into())?;
        machine.enable_installation_tracking().await?;
        let installed = machine.install(&desired).await?;
        let installation = BindingInstallation {
            plugin_id: installed.plugin_id.clone(),
            plugin_version: installed.plugin_version.clone(),
            generation_digest: installed
                .generation_digest
                .clone()
                .try_into()
                .map_err(anyhow::Error::msg)?,
            contract_fingerprint: installed
                .contract_fingerprint
                .clone()
                .try_into()
                .map_err(anyhow::Error::msg)?,
            installation_revision: installed
                .installation_revision
                .clone()
                .ok_or_else(|| anyhow::anyhow!("installation revision missing"))?,
        };
        probe::private_write(
            &machine_root.join("telemetry.json"),
            &serde_json::to_vec(&json!({
                "plugin":{"plugin_id":installed.plugin_id,"plugin_version":installed.plugin_version,"generation_digest":installed.generation_digest},
                "logs":{"base_url":"http://127.0.0.1:1","bearer_token":"isolated-fixture-only"},"metrics":null,"traces":null,
            }))?,
        )?;
        let identity = crate::machine_auth::MachineIdentity::load_or_create(&machine_root)?;
        let store = Store::connect(
            &format!(
                "sqlite://{}",
                root.join("controller/store.sqlite3").display()
            ),
            root.join("controller/artifacts"),
        )
        .await?;
        let enrollment = store
            .create_machine_enrollment(MACHINE, "Isolated connected Machine", 60)
            .await?;
        store
            .consume_machine_enrollment(
                &enrollment,
                identity.public_key(),
                machine.encryption_public_key(),
            )
            .await?;
        let password = crate::client_auth::new_code_verifier()?;
        let now = chrono::Utc::now().timestamp_millis();
        store
            .insert_user(&ProductUser {
                id: "c".repeat(32),
                username: ACCOUNT.into(),
                password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.into(),
                password_hash: crate::product_auth::hash_password(&password)?,
                created_at_ms: now,
                updated_at_ms: now,
                disabled_at_ms: None,
            })
            .await?;
        store
            .put_setting(
                crate::admin::PERMISSIONS_SETTING,
                &json!({"default_role":"operator","grants":[]}),
            )
            .await?;
        probe::private_write(
            &root.join("core-security.json"),
            &serde_json::to_vec(&json!({
                "schema":"dravengarden.cowboy.core-security/v1","passkeys":{"namespace_id":"passkey","source":"fresh"}
            }))?,
        )?;
        admission::PolicyCase::All.write(&root.join("controller-writer.json"))?;
        admission::PolicyCase::All.write(&machine_root.join("telemetry-writer-policy.json"))?;
        Ok(Self {
            reader,
            installation,
            password,
            package_sha256: sha256(&package),
            release_sha256: sha256(&release),
        })
    }
}

#[tokio::test]
async fn connected_seed_has_real_enrolled_identity_signed_installation_and_no_local_auth_shortcut()
{
    let root = tempfile::tempdir().unwrap();
    let helper = manifest::ssh_keygen().unwrap();
    let f = ConnectedFixture::seed(root.path(), Flow::BindingRoundTrip, &helper.path)
        .await
        .unwrap();
    let evidence = Evidence::read(root.path()).unwrap();
    assert!(evidence.service.is_none() && evidence.machine.is_none());
    let store = Store::connect(
        &format!(
            "sqlite://{}",
            root.path().join("controller/store.sqlite3").display()
        ),
        root.path().join("controller/artifacts"),
    )
    .await
    .unwrap();
    assert!(store.machine_public_key(MACHINE).await.unwrap().is_some());
    let user = store.user_by_username(ACCOUNT).await.unwrap().unwrap();
    assert!(crate::product_auth::verify_password(
        &f.password,
        &user.password_hash
    ));
    assert_eq!(f.installation.plugin_id, "victoria");
    assert!(
        store
            .telemetry_binding_ledger(SERVICE)
            .await
            .unwrap()
            .is_none()
    );
}
