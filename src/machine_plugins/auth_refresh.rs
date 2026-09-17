//! Validate changed credentials with the Plugin's signed native auth probe.
//! A CLI logout/error document is not a successful credential rotation.

use super::*;

struct RefreshProbe {
    directory: StagedProbeHome,
    command: PathBuf,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    credentials: Option<(
        cowboy_provider_sdk::AuthenticationContract,
        PortableCredentialBundle,
    )>,
}

impl MachinePluginStore {
    pub async fn validate_auth_refresh_candidate(
        self: &std::sync::Arc<Self>,
        candidate: &ProviderAuthRefreshCandidate,
    ) -> Result<()> {
        let store = std::sync::Arc::clone(self);
        let candidate = candidate.clone();
        let probe =
            tokio::task::spawn_blocking(move || store.prepare_refresh_probe(&candidate)).await??;
        let Some(probe) = probe else {
            return Ok(());
        };
        probe.validate(Duration::from_secs(5)).await
    }
}

impl RefreshProbe {
    async fn validate(&self, timeout: Duration) -> Result<()> {
        let home = self.directory.0.join("home");
        let status = tokio::time::timeout(
            timeout,
            tokio::process::Command::new(&self.command)
                .args(&self.arguments)
                .env_clear()
                .envs(&self.environment)
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("XDG_DATA_HOME", home.join(".local/share"))
                .env("XDG_CACHE_HOME", home.join(".cache"))
                .current_dir(&home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await
        .context("credential refresh probe timed out")??;
        ensure!(
            status.success(),
            "changed credentials failed the Plugin authentication probe"
        );
        if let Some((auth, expected)) = &self.credentials {
            ensure!(
                projected_credential_bundle(auth, &self.directory.0, &expected.method_id)?
                    == *expected,
                "authentication probe changed the candidate credentials"
            );
        }
        Ok(())
    }
}

impl MachinePluginStore {
    fn prepare_refresh_probe(
        &self,
        candidate: &ProviderAuthRefreshCandidate,
    ) -> Result<Option<RefreshProbe>> {
        let content = self
            .plugin_root(&candidate.provider_id)
            .join("generations")
            .join(digest_generation_name(&candidate.generation_digest)?)
            .join("content");
        // Retained legacy releases predate signed host probe contracts.
        if !content.join("package.cowboy-plugin").is_file() {
            return Ok(None);
        }
        let (plugin, release, content) =
            self.verified_plugin_descriptor(&candidate.provider_id, &candidate.generation_digest)?;
        let Some(host) = verified_plugin_host_bundle(&plugin, &release, &content)? else {
            return Ok(None);
        };
        let spec = PluginHostSpec::from_json(host.files["host.json"].as_bytes())?;
        if !spec.cli_auth.is_exit() {
            return Ok(None);
        }
        let package = plugin
            .agent_provider()
            .context("refresh probe requires an Agent Provider")?;
        let auth = &package.manifest.authentication;
        let method = auth
            .methods
            .iter()
            .find(|method| method.id == candidate.bundle.method_id)
            .context("unknown refresh authentication method")?;
        let cowboy_provider_sdk::AuthExecutor::CommandV1 { component, .. } = &method.executor
        else {
            return Ok(None);
        };
        // Execution, unlike observation, must verify every immutable runtime byte.
        let (_, _, path, _) =
            self.verified_generation(&candidate.provider_id, &candidate.generation_digest)?;
        let payload = matching_payload(package, &self.platform, &self.architecture)?;
        let native = payload
            .private_components
            .iter()
            .find(|entry| entry.kind == component.kind && entry.slot == component.slot)
            .context("refresh probe has no exact native component")?;
        let command = runtime_command(&path, &native.command)?;
        let root = std::env::temp_dir().join(format!(
            "cowboy-auth-refresh-{:032x}",
            rand::random::<u128>()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let directory = StagedProbeHome(root);
        restore_projected_bundle(auth, &directory.0, &candidate.bundle)?;
        let mut environment: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(directory.0.join("environment.json"))?)?;
        for key in [
            "PATH",
            "LANG",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NIX_SSL_CERT_FILE",
        ] {
            if let Ok(value) = std::env::var(key) {
                environment.insert(key.to_owned(), value);
            }
        }
        Ok(Some(RefreshProbe {
            directory,
            command,
            arguments: spec.cli_auth_argv,
            environment,
            credentials: Some((auth.clone(), candidate.bundle.clone())),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an explicit immutable native CLI; uses synthetic credentials only"]
    async fn native_claude_refresh_probe_conformance() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("home/.claude");
        fs::create_dir_all(&config).unwrap();
        let probe = RefreshProbe {
            directory: StagedProbeHome(root.path().to_path_buf()),
            command: std::env::var_os("COWBOY_TEST_AUTH_PROBE_CLI")
                .expect("explicit native CLI")
                .into(),
            arguments: vec!["auth".into(), "status".into(), "--json".into()],
            environment: BTreeMap::new(),
            credentials: None,
        };
        let path = config.join(".credentials.json");
        fs::write(&path, br#"{"claudeAiOauth":{"accessToken":"synthetic-access","refreshToken":"synthetic-refresh","expiresAt":4102444800000,"scopes":["user:inference"],"subscriptionType":"max"}}"#).unwrap();
        probe.validate(Duration::from_secs(5)).await.unwrap();
        fs::write(&path, br#"{"claudeAiOauth":{"accessToken":null,"refreshToken":null,"expiresAt":0,"scopes":["user:inference"],"subscriptionType":"max"}}"#).unwrap();
        assert!(probe.validate(Duration::from_secs(5)).await.is_err());
    }

    /// The shared credential store is what makes one refresh lock cover every
    /// auth generation. Accept a new native CLI only while it still reads
    /// credentials from that directory instead of its private config home.
    #[tokio::test]
    #[ignore = "requires an explicit immutable native CLI; uses synthetic credentials only"]
    async fn native_claude_shared_credential_store_conformance() {
        let cli: PathBuf = std::env::var_os("COWBOY_TEST_AUTH_PROBE_CLI")
            .expect("explicit native CLI")
            .into();
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let shared = root.path().join("shared");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(&shared).unwrap();
        let valid = br#"{"claudeAiOauth":{"accessToken":"synthetic-access","refreshToken":"synthetic-refresh","expiresAt":4102444800000,"scopes":["user:inference"],"subscriptionType":"max"}}"#;
        fs::write(shared.join(".credentials.json"), valid).unwrap();
        let status = |store: Option<&Path>| {
            let mut command = std::process::Command::new(&cli);
            command
                .args(["auth", "status", "--json"])
                .env_clear()
                .env("HOME", &home)
                .env("PATH", std::env::var("PATH").unwrap_or_default())
                .current_dir(&home)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(store) = store {
                command.env("CLAUDE_SECURESTORAGE_CONFIG_DIR", store);
            }
            command.status().unwrap().success()
        };
        // Credentials only in the shared store: honoured through the variable,
        // invisible without it.
        assert!(status(Some(&shared)));
        assert!(!status(None));
    }

    #[tokio::test]
    async fn refresh_probe_checks_exact_snapshot_and_rejects_signed_out_state() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir(&home).unwrap();
        let probe = RefreshProbe {
            directory: StagedProbeHome(root.path().to_path_buf()),
            command: PathBuf::from("/bin/sh"),
            arguments: vec!["-c".to_owned(), "test -z \"${ANTHROPIC_API_KEY:-}\" && test -z \"${CLAUDE_CONFIG_DIR:-}\" && read -r value < \"$HOME/credential\" && test \"$value\" = valid".to_owned()],
            environment: BTreeMap::new(),
            credentials: None,
        };
        fs::write(home.join("credential"), b"valid\n").unwrap();
        probe.validate(Duration::from_secs(1)).await.unwrap();
        fs::write(home.join("credential"), b"cleared\n").unwrap();
        assert!(
            probe
                .validate(Duration::from_secs(1))
                .await
                .unwrap_err()
                .to_string()
                .contains("authentication probe")
        );
    }

    #[tokio::test]
    async fn stalled_refresh_probe_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("home")).unwrap();
        let probe = RefreshProbe {
            directory: StagedProbeHome(root.path().to_path_buf()),
            command: PathBuf::from("/bin/sh"),
            arguments: vec!["-c".to_owned(), "while :; do :; done".to_owned()],
            environment: BTreeMap::new(),
            credentials: None,
        };
        assert!(
            probe
                .validate(Duration::from_millis(100))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }

    #[tokio::test]
    async fn successful_exit_cannot_validate_a_different_credential_snapshot() {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../plugins/grok/provider.json")).unwrap();
        let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        let auth = package.manifest.authentication;
        let bundle = PortableCredentialBundle {
            portable_schema: auth.portable_schema.clone(),
            method_id: "xai-account".into(),
            values: BTreeMap::from([(
                "auth_json".into(),
                base64::engine::general_purpose::STANDARD.encode(b"original"),
            )]),
        };
        let root = tempfile::tempdir().unwrap();
        restore_projected_bundle(&auth, root.path(), &bundle).unwrap();
        let probe = RefreshProbe {
            directory: StagedProbeHome(root.path().to_path_buf()),
            command: PathBuf::from("/bin/sh"),
            arguments: vec![
                "-c".into(),
                "printf changed > \"$HOME/.grok/auth.json\"".into(),
            ],
            environment: BTreeMap::new(),
            credentials: Some((auth, bundle)),
        };
        assert!(
            probe
                .validate(Duration::from_secs(1))
                .await
                .unwrap_err()
                .to_string()
                .contains("changed the candidate")
        );
    }
}
