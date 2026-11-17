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

/// First retry delay after a failed probe; doubles per failure of the same
/// exact candidate up to [`PROBE_RETRY_MAX`].
const PROBE_RETRY_INITIAL: Duration = Duration::from_secs(30);
const PROBE_RETRY_MAX: Duration = Duration::from_mins(30);
/// Bounded memory: forget the oldest deadlines first.
const PROBE_FAILURE_ENTRIES: usize = 64;

#[derive(Default)]
pub(super) struct ProbeFailures(BTreeMap<[u8; 32], ProbeFailure>);

struct ProbeFailure {
    failures: u32,
    retry_at: Instant,
}

/// The exact candidate failed its probe recently; it was not probed again.
#[derive(Debug)]
pub struct RefreshProbeBackoff;

impl std::fmt::Display for RefreshProbeBackoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("unchanged credentials recently failed the Plugin authentication probe")
    }
}

impl std::error::Error for RefreshProbeBackoff {}

/// The exact candidate failed its probe again after its backoff. It was
/// already reported as Failed; the retry is not new evidence.
#[derive(Debug)]
pub struct RefreshProbeRepeated {
    pub failures: u32,
}

impl std::fmt::Display for RefreshProbeRepeated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "unchanged credentials failed the Plugin authentication probe {} times",
            self.failures
        )
    }
}

/// The exact candidate was not probed again: it is still backing off.
pub fn refresh_backing_off(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RefreshProbeBackoff>().is_some()
}

/// A logged-out projection otherwise warned every 30 minutes forever: only
/// an exact candidate's first rejection is a warning, repeats are debug.
pub fn log_refresh_rejection(provider: &str, error: &anyhow::Error) {
    if let Some(repeated) = error.downcast_ref::<RefreshProbeRepeated>() {
        tracing::debug!(provider, failures = repeated.failures, reason = crate::logs::failure_reason(error), %error, "rejected credential refresh failed again");
    } else {
        tracing::warn!(provider, reason = crate::logs::failure_reason(error), %error, "invalid credential refresh excluded from promotion");
    }
}

fn candidate_digest(candidate: &ProviderAuthRefreshCandidate) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    for part in [
        candidate.provider_id.as_bytes(),
        candidate.account_id.as_str().as_bytes(),
        &candidate.expected_generation.to_be_bytes(),
        candidate.generation_digest.as_bytes(),
        &serde_json::to_vec(&candidate.bundle)?,
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    Ok(hash.finalize().into())
}

impl ProbeFailures {
    fn suppressed(&self, digest: &[u8; 32], now: Instant) -> bool {
        self.0
            .get(digest)
            .is_some_and(|failure| now < failure.retry_at)
    }

    fn record(&mut self, digest: [u8; 32], now: Instant) -> u32 {
        let failures = self.0.get(&digest).map_or(0, |failure| failure.failures) + 1;
        let delay = PROBE_RETRY_INITIAL
            .saturating_mul(1 << (failures - 1).min(10))
            .min(PROBE_RETRY_MAX);
        self.0.insert(
            digest,
            ProbeFailure {
                failures,
                retry_at: now + delay,
            },
        );
        while self.0.len() > PROBE_FAILURE_ENTRIES {
            let Some(oldest) = self
                .0
                .iter()
                .min_by_key(|(_, failure)| failure.retry_at)
                .map(|(digest, _)| *digest)
            else {
                break;
            };
            self.0.remove(&oldest);
        }
        failures
    }
}

impl MachinePluginStore {
    pub async fn validate_auth_refresh_candidate(
        self: &std::sync::Arc<Self>,
        candidate: &ProviderAuthRefreshCandidate,
    ) -> Result<Option<crate::provider_account::NativeIdentity>> {
        let digest = candidate_digest(candidate)?;
        if self
            .refresh_probe_failures
            .lock()
            .suppressed(&digest, Instant::now())
        {
            return Err(RefreshProbeBackoff.into());
        }
        let result = self.probe_auth_refresh_candidate(candidate).await;
        let mut failures = self.refresh_probe_failures.lock();
        match result {
            Ok(identity) => {
                failures.0.remove(&digest);
                Ok(identity)
            }
            Err(error) => match failures.record(digest, Instant::now()) {
                1 => Err(error),
                failures => Err(error.context(RefreshProbeRepeated { failures })),
            },
        }
    }

    async fn probe_auth_refresh_candidate(
        self: &std::sync::Arc<Self>,
        candidate: &ProviderAuthRefreshCandidate,
    ) -> Result<Option<crate::provider_account::NativeIdentity>> {
        let store = std::sync::Arc::clone(self);
        let probed = candidate.clone();
        let probe =
            tokio::task::spawn_blocking(move || store.prepare_refresh_probe(&probed)).await??;
        if let Some(probe) = probe {
            probe.validate(Duration::from_secs(5)).await?;
        }
        // Identity evidence never depends on an exit-only probe being signed.
        let store = std::sync::Arc::clone(self);
        let candidate = candidate.clone();
        tokio::task::spawn_blocking(move || store.refresh_identity(&candidate)).await?
    }

    /// The identity a refreshed subscription credential proves by itself: its
    /// bundle names the portable twin of a native sign-in method whose signed
    /// recipe reads a credential file (Codex, Grok). Read from a private
    /// staged copy of the candidate, for the default account too, so the
    /// Service can check a rotation still belongs to the account it lands in.
    fn refresh_identity(
        &self,
        candidate: &ProviderAuthRefreshCandidate,
    ) -> Result<Option<crate::provider_account::NativeIdentity>> {
        let package =
            self.package_for_generation(&candidate.provider_id, &candidate.generation_digest);
        // A retained legacy default release keeps refreshing without evidence.
        let (package, _) = match package {
            Err(_) if candidate.account_id.is_default() => return Ok(None),
            package => package?,
        };
        let auth = &package.manifest.authentication;
        let Some(recipe) = portable_contract::native_method_for(auth, &candidate.bundle.method_id)
            .and_then(|method| method.native_identity.as_ref())
            .filter(|recipe| portable_contract::identity_in_credentials(auth, recipe))
        else {
            return Ok(None);
        };
        let root = std::env::temp_dir().join(format!(
            "cowboy-auth-identity-{:032x}",
            rand::random::<u128>()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let staged = StagedProbeHome(root);
        restore_projected_bundle(auth, &staged.0, &candidate.bundle)?;
        native_identity::read(&staged.0.join("home"), recipe, &candidate.account_id).map(Some)
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

    #[test]
    fn failed_probe_backs_off_per_exact_candidate_and_stays_bounded() {
        let mut failures = ProbeFailures::default();
        let start = Instant::now();
        let digest = [1; 32];
        assert!(!failures.suppressed(&digest, start));
        assert_eq!(failures.record(digest, start), 1);
        assert!(failures.suppressed(&digest, start + Duration::from_secs(29)));
        assert!(!failures.suppressed(&digest, start + PROBE_RETRY_INITIAL));
        assert!(!failures.suppressed(&[2; 32], start));
        let later = start + PROBE_RETRY_INITIAL;
        // A repeat is counted, so only the first rejection is reported.
        assert_eq!(failures.record(digest, later), 2);
        assert!(failures.suppressed(&digest, later + Duration::from_secs(59)));
        assert!(!failures.suppressed(&digest, later + Duration::from_secs(60)));
        for _ in 0..20 {
            failures.record(digest, later);
        }
        assert!(!failures.suppressed(&digest, later + PROBE_RETRY_MAX));
        for index in 0..=u8::try_from(PROBE_FAILURE_ENTRIES).unwrap() {
            failures.record([index.wrapping_add(10); 32], later);
        }
        assert_eq!(failures.0.len(), PROBE_FAILURE_ENTRIES);
        let repeated = anyhow::anyhow!("logged out").context(RefreshProbeRepeated { failures: 2 });
        assert!(repeated.downcast_ref::<RefreshProbeRepeated>().is_some());
        assert!(!refresh_backing_off(&repeated));
    }

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
            serde_json::from_str(include_str!("../../../../plugins/grok/provider.json")).unwrap();
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
