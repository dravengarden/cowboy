//! Machine side of the `account_sandbox_v1` runtime isolation
//! (`docs/antigravity-real-accounts-design.md`, "Machine changes"): the
//! daemon this Machine uses, the per-generation runtime archive, the worker
//! launch of a sandboxed named session and the daemon's credential evidence.
//!
//! A sandboxed Provider never runs as the Machine user and its account home
//! lives in the daemon. The Machine keeps only the native profile metadata
//! and an empty reservation directory, so generation retention, revocation
//! and logout tombstones keep their existing meaning; credential evidence
//! comes from the daemon instead of a local home.
use super::*;
use crate::account_sandbox as daemon;

mod runtime_archive;

/// The daemon this Machine reaches and the Service its accounts belong to.
#[derive(Debug, Clone)]
pub(super) struct Host {
    socket: PathBuf,
    service_id: Option<String>,
}

/// One daemon connection for a named account, its key and the generation's
/// registered runtime.
pub struct AccountConnection {
    pub client: daemon::Client,
    pub key: String,
    pub runtime_sha256: String,
    pub runtime_archive: PathBuf,
}

/// Whether the signed manifest requires the account sandbox.
pub(super) fn sandboxed(package: &ProviderPackage) -> bool {
    package.manifest.runtime.isolation
        == Some(cowboy_provider_sdk::RuntimeIsolationPolicy::AccountSandboxV1)
}

impl MachinePluginStore {
    /// Wire the daemon. Without it, or without a pinned Service id, every
    /// sandboxed Provider is unavailable on this Machine.
    pub fn set_account_sandbox(&self, socket: PathBuf, service_id: Option<String>) {
        let _ = self.account_sandbox.set(Host { socket, service_id });
    }

    fn sandbox_host(&self) -> Result<(&Path, &str)> {
        let host = self
            .account_sandbox
            .get()
            .context("this Machine has no account sandbox configured")?;
        let service = host
            .service_id
            .as_deref()
            .context("the account sandbox requires a pinned Cowboy Service id")?;
        Ok((&host.socket, service))
    }

    /// Why a sandboxed Provider cannot run here; `None` when it can, or when
    /// the Provider is not sandboxed.
    pub(super) fn account_sandbox_unavailable(&self, package: &ProviderPackage) -> Option<String> {
        if !sandboxed(package) {
            return None;
        }
        self.sandbox_host()
            .and_then(|(socket, _)| daemon::Client::connect(socket).map(drop))
            .err()
            .map(|error| {
                format!(
                    "{} runs only in the account sandbox, which is unavailable: {error:#}",
                    package.manifest.display.name
                )
            })
    }

    /// Connect for this named account and make sure the daemon has the
    /// runtime of generation `digest`, registering it once per generation.
    pub fn account_sandbox_connection(
        &self,
        provider_id: &str,
        digest: &str,
    ) -> Result<AccountConnection> {
        ensure!(
            !self.account_id.is_default(),
            "the account sandbox serves only named accounts"
        );
        let (socket, service) = self.sandbox_host()?;
        let (package, package_path) = self.package_for_generation(provider_id, digest)?;
        ensure!(
            sandboxed(&package),
            "Provider generation does not run in the account sandbox"
        );
        let mut client = daemon::Client::connect(socket)?;
        let account = client.ensure_account(service, self.account_id.as_str())?;
        let generation = package_path
            .parent()
            .and_then(Path::parent)
            .context("Provider generation has no directory")?;
        let runtime = if let Some(cached) = runtime_archive::cached(generation)? {
            cached
        } else {
            let payload = matching_payload(&package, &self.platform, &self.architecture)?;
            let components = runtime_archive::Components::select(payload, |command| {
                runtime_command(&package_path, command)
            })?;
            let archive = runtime_archive::build(&components, generation)?;
            daemon::register_file(&mut client, &archive.path, &archive.sha256)?;
            runtime_archive::record_registered(generation, &archive)?;
            archive
        };
        Ok(AccountConnection {
            client,
            key: account.key,
            runtime_sha256: runtime.sha256,
            runtime_archive: runtime.path,
        })
    }

    /// The worker launch of a sandboxed named session on its exact native
    /// credential generation. The worker starts the unit itself.
    pub(super) fn account_sandbox_launch(
        &self,
        package: &ProviderPackage,
        generation_digest: &str,
        auth_generation: Option<u64>,
    ) -> Result<daemon::Launch> {
        let generation = auth_generation.context("session has no Provider auth generation")?;
        ensure!(
            self.native_session(package, generation),
            "the account sandbox runs only native named sessions"
        );
        self.ensure_native_release(package, generation, generation_digest)?;
        // Profile metadata, bound identity and the daemon's signed-in evidence.
        self.native_profile_home(package, generation)?;
        let (socket, service) = self.sandbox_host()?;
        let connection =
            self.account_sandbox_connection(&package.manifest.id, generation_digest)?;
        Ok(daemon::Launch {
            socket: socket.to_path_buf(),
            service_id: service.to_owned(),
            account_id: self.account_id.as_str().to_owned(),
            credential_generation: generation,
            runtime_sha256: connection.runtime_sha256,
            runtime_archive: connection.runtime_archive,
        })
    }

    /// The daemon's evidence that `generation` is this account's current
    /// signed-in credential. A missing account is never created here.
    pub(super) fn account_sandbox_signed_in(&self, generation: u64) -> Result<()> {
        let (socket, service) = self.sandbox_host()?;
        let units = daemon::Client::connect(socket)?
            .list_units(&daemon::account_key(service, self.account_id.as_str()))?;
        ensure!(
            units.credential.signed_in(generation),
            "the account sandbox holds no signed-in credential of generation {generation}"
        );
        Ok(())
    }

    /// Fence or admit one account in the daemon for the Controller's account
    /// controls. `stop_account` answers only after it confirmed every unit of
    /// the account gone; an account the daemon never created has no unit.
    pub fn account_sandbox_fence(
        &self,
        request: &crate::provider_account::AccountFenceRequest,
    ) -> Result<crate::provider_account::AccountFenceObservation> {
        use crate::provider_account::{AccountFenceAction, AccountFenceObservation};
        ensure!(
            !request.account_id.is_default()
                && crate::provider_account::named_only(&request.provider_id),
            "account fences apply only to named accounts of account-sandbox Providers"
        );
        let (socket, service) = self.sandbox_host()?;
        let key = daemon::account_key(service, request.account_id.as_str());
        let mut client = daemon::Client::connect(socket)?;
        let result = match request.action {
            AccountFenceAction::Stop => client.stop_account(&key),
            AccountFenceAction::Admit => client.admit_account(&key),
        };
        let observation = |fence_epoch, stopped| AccountFenceObservation {
            request: request.clone(),
            fence_epoch,
            fenced: fence_epoch.is_some() && request.action == AccountFenceAction::Stop,
            stopped,
        };
        match result {
            Ok(fence) => Ok(observation(Some(fence.fence_epoch), fence.stopped)),
            Err(error) if daemon::DaemonError::code_of(&error) == Some("not_found") => {
                Ok(observation(None, Vec::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// Confirm with the daemon that `unit` is gone, stopping it if a lost
    /// worker left it running.
    pub fn confirm_account_unit_gone(&self, unit: &str) -> Result<()> {
        let (socket, _) = self.sandbox_host()?;
        let mut client = daemon::Client::connect(socket)?;
        if client.unit_state(unit)?.alive {
            ensure!(
                client.stop_unit(unit)?,
                "the account sandbox did not stop {unit}"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, Write as _};

    fn package(source: &str) -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(source).unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    fn store(root: &Path) -> MachinePluginStore {
        MachinePluginStore::new(root, Platform::Linux, "x86_64".into()).unwrap()
    }

    #[test]
    fn sandboxed_providers_are_unavailable_without_a_reachable_daemon() {
        let antigravity = package(include_str!(
            "../../../../plugins/antigravity/provider.json"
        ));
        let codex = package(include_str!("../../../../plugins/codex/provider.json"));
        assert!(sandboxed(&antigravity) && !sandboxed(&codex));
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        assert!(store.account_sandbox_unavailable(&codex).is_none());
        let detail = store.account_sandbox_unavailable(&antigravity).unwrap();
        assert!(detail.contains("no account sandbox configured"), "{detail}");
        store.set_account_sandbox(root.path().join("missing.sock"), Some("service".into()));
        let detail = store.account_sandbox_unavailable(&antigravity).unwrap();
        assert!(detail.contains("unreachable"), "{detail}");
    }

    #[test]
    fn account_fences_drive_the_daemon_and_report_its_confirmation() {
        use crate::provider_account::{AccountFenceAction, AccountFenceRequest};
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let key = daemon::account_key("service", account.as_str());
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in [
                serde_json::json!({"ok": true, "result": {
                    "key": "k", "fence_epoch": 4, "stopped": ["cga-k-runtime-1.service"]}}),
                serde_json::json!({"ok": false, "error": {
                    "code": "not_found", "message": "unknown account"}}),
                serde_json::json!({"ok": false, "error": {
                    "code": "internal", "message": "units still running: u"}}),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&stream)
                    .read_line(&mut line)
                    .unwrap();
                requests.push(serde_json::from_str::<serde_json::Value>(&line).unwrap());
                writeln!(stream, "{reply}").unwrap();
            }
            requests
        });
        let store = store(root.path());
        store.set_account_sandbox(socket, Some("service".into()));
        let request = |action| AccountFenceRequest {
            operation_id: "op".into(),
            provider_id: "antigravity".into(),
            account_id: account.clone(),
            action,
        };
        let stopped = store
            .account_sandbox_fence(&request(AccountFenceAction::Stop))
            .unwrap();
        assert_eq!(stopped.fence_epoch, Some(4));
        assert!(stopped.fenced);
        assert_eq!(stopped.stopped, ["cga-k-runtime-1.service"]);
        // An account the daemon never created has no unit to stop.
        let absent = store
            .account_sandbox_fence(&request(AccountFenceAction::Admit))
            .unwrap();
        assert_eq!((absent.fence_epoch, absent.fenced), (None, false));
        let error = store
            .account_sandbox_fence(&request(AccountFenceAction::Stop))
            .unwrap_err();
        assert!(format!("{error:#}").contains("still running"), "{error:#}");
        let ops: Vec<_> = server.join().unwrap();
        assert_eq!(
            ops[0],
            serde_json::json!({"op": "stop_account", "key": key})
        );
        assert_eq!(
            ops[1],
            serde_json::json!({"op": "admit_account", "key": key})
        );
        // Only named accounts of account-sandbox Providers are fenced.
        let mut codex = request(AccountFenceAction::Stop);
        codex.provider_id = "codex".into();
        assert!(store.account_sandbox_fence(&codex).is_err());
    }

    #[test]
    fn credential_evidence_comes_from_the_daemon_without_creating_the_account() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let key = daemon::account_key("service", account.as_str());
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&stream)
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let reply = serde_json::json!({"ok": true, "result": {
                    "key": request["key"], "boot_id": "boot", "fenced": null,
                    "credential": {"generation": 3, "state": "signed_in"},
                    "units": [], "unrecorded": [],
                }});
                writeln!(stream, "{reply}").unwrap();
                requests.push(request);
            }
            requests
        });
        let store = store(root.path()).for_account(&account).unwrap();
        store.set_account_sandbox(socket, Some("service".into()));
        store.account_sandbox_signed_in(3).unwrap();
        let error = store.account_sandbox_signed_in(2).unwrap_err().to_string();
        assert!(error.contains("generation 2"), "{error}");
        for request in server.join().unwrap() {
            assert_eq!(request, serde_json::json!({"op": "list_units", "key": key}));
        }
    }
}
