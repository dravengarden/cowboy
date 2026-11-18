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
            // The generation's vendor code first runs here, confined in the
            // daemon's probe identity; a failed probe refuses the generation.
            let probe = client.probe_runtime(&archive.sha256)?;
            ensure!(
                probe.passed,
                "the account sandbox probe of this Provider generation exited {:?}",
                probe.exit_code
            );
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

    /// The daemon socket and this named account's key.
    pub fn account_sandbox_address(&self) -> Result<(PathBuf, String)> {
        let (socket, service) = self.sandbox_host()?;
        Ok((
            socket.to_path_buf(),
            daemon::account_key(service, self.account_id.as_str()),
        ))
    }

    /// The daemon's record of this named account: its credential and units.
    /// A missing account is never created here (`not_found`).
    pub fn account_sandbox_units(&self) -> Result<daemon::AccountUnits> {
        let (socket, service) = self.sandbox_host()?;
        daemon::Client::connect(socket)?
            .list_units(&daemon::account_key(service, self.account_id.as_str()))
    }

    /// The daemon's evidence that `generation` is this account's current
    /// signed-in credential. A missing account is never created here.
    pub(super) fn account_sandbox_signed_in(&self, generation: u64) -> Result<()> {
        let units = self.account_sandbox_units()?;
        ensure!(
            units.credential.signed_in(generation),
            "the account sandbox holds no signed-in credential of generation {generation}"
        );
        Ok(())
    }

    /// Fence or admit one account in the daemon for the Controller's account
    /// controls. `stop_account` answers only after the fence is durable (as
    /// a tombstone for an account the daemon has not created yet, which the
    /// account adopts when it is created) and every unit of the account is
    /// confirmed gone; a daemon refusal, including `not_found`, is never a
    /// confirmation. A revisioned request is ordered by the daemon (a stale
    /// one is refused with `conflict`).
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
        // The daemon orders revisioned operations under the account lock: an
        // older revision, or another operation at the applied one, is its
        // `conflict`, which is an error here and never a confirmation.
        let order = request.revision.map(|revision| daemon::FenceOrder {
            revision,
            operation_id: &request.operation_id,
        });
        let fence = match request.action {
            AccountFenceAction::Stop => client.stop_account(&key, order)?,
            AccountFenceAction::Admit => client.admit_account(&key, order)?,
        };
        Ok(AccountFenceObservation {
            request: request.clone(),
            fence_epoch: Some(fence.fence_epoch),
            fenced: fence.fenced,
            stopped: fence.stopped,
        })
    }

    /// Remove a revoked account's persistent home and system user. The
    /// revocation has stopped its sessions and logged out its credential;
    /// the account fence `remove_account` requires is set here first (an
    /// unrevisioned stop only tightens and stops every unit of the account),
    /// then the daemon removes both under the revocation's operation ID, so
    /// a retry is the same removal's replay. A daemon refusal is an error. A
    /// Machine without an account sandbox, or whose daemon never created the
    /// account, holds nothing of it and answers the removal confirmed.
    pub fn account_sandbox_remove(
        &self,
        request: &crate::provider_account::AccountRemoveRequest,
    ) -> Result<crate::provider_account::AccountRemoveObservation> {
        ensure!(
            !request.account_id.is_default()
                && crate::provider_account::named_only(&request.provider_id),
            "account removal applies only to named accounts of account-sandbox Providers"
        );
        if self
            .account_sandbox
            .get()
            .is_none_or(|host| !host.socket.exists())
            && !self
                .for_account(&request.account_id)?
                .has_sandboxed_native_profile(&request.provider_id)?
        {
            // No daemon is installed here (its socket is absent) and no
            // native profile of the account was ever sandboxed, so none of
            // the account exists (a historical sign-in ran unsandboxed). An
            // installed daemon that does not answer keeps the removal
            // pending.
            return Ok(crate::provider_account::AccountRemoveObservation {
                request: request.clone(),
                home_removed: true,
                user_removed: true,
            });
        }
        let (socket, service) = self.sandbox_host()?;
        let key = daemon::account_key(service, request.account_id.as_str());
        let mut client = daemon::Client::connect(socket)?;
        match client.stop_account(&key, None) {
            // The daemon never created the account: `remove_account`
            // confirms its absence (and any leftover user's removal).
            Err(error) if daemon::DaemonError::code_of(&error) == Some("not_found") => {}
            other => {
                other?;
            }
        }
        let removed = client.remove_account(&key, &request.operation_id)?;
        Ok(crate::provider_account::AccountRemoveObservation {
            request: request.clone(),
            home_removed: removed.removed && removed.home_removed,
            user_removed: removed.removed && removed.user_removed,
        })
    }

    /// Confirm with the daemon that no unit of `session` remains, from the
    /// session's durable worker launch record rather than the worker's
    /// snapshot (a lost worker reports none): every unit the daemon recorded
    /// for the session is stopped and confirmed gone, and no unrecorded unit
    /// of the account is alive. A session whose bound Provider is not
    /// sandboxed has no unit.
    pub fn confirm_session_account_units_gone(&self, session: &str) -> Result<()> {
        let Some(receipt) = self.account_worker_exit(session)? else {
            return Ok(());
        };
        let Some(binding) = &receipt.binding else {
            return Ok(());
        };
        let (provider, digest) = binding.provider_generation();
        // A retained generation decides; without it, a named-only Provider
        // is assumed sandboxed so a lost unit is never skipped.
        let isolated = self.package_for_generation(provider, digest).map_or_else(
            |_| crate::provider_account::named_only(provider),
            |(package, _)| sandboxed(&package),
        );
        if !isolated {
            return Ok(());
        }
        let account = self.for_account(&receipt.account)?;
        let units = match account.account_sandbox_units() {
            Ok(units) => units,
            // The daemon has no such account, so it runs no unit of it.
            Err(error) if daemon::DaemonError::code_of(&error) == Some("not_found") => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let (socket, _) = self.sandbox_host()?;
        let mut client = daemon::Client::connect(socket)?;
        for unit in units
            .units
            .iter()
            .filter(|unit| unit.session_id.as_deref() == Some(session))
        {
            ensure!(
                client.stop_unit(&unit.unit)?,
                "the account sandbox did not stop {}",
                unit.unit
            );
        }
        // A unit of the account the daemon holds no record of (its own
        // reconciliation stops it) may still be this session's: the worker's
        // exit stays unconfirmed until it is gone.
        ensure!(
            units.unrecorded.is_empty(),
            "unrecorded account sandbox units remain: {}",
            units.unrecorded.join(", ")
        );
        Ok(())
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
                    "key": "k", "fence_epoch": 4, "fenced": true,
                    "stopped": ["cga-k-runtime-1.service"]}}),
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
            revision: None,
        };
        let stopped = store
            .account_sandbox_fence(&request(AccountFenceAction::Stop))
            .unwrap();
        assert_eq!(stopped.fence_epoch, Some(4));
        assert!(stopped.fenced);
        assert_eq!(stopped.stopped, ["cga-k-runtime-1.service"]);
        // A daemon refusal is never fence evidence, `not_found` included.
        let error = store
            .account_sandbox_fence(&request(AccountFenceAction::Admit))
            .unwrap_err();
        assert!(format!("{error:#}").contains("not_found"), "{error:#}");
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

    /// A fake daemon that orders fence operations like the real one
    /// (`packaging/account-sandbox/src/fence.rs`): it keeps the applied
    /// disposition and refuses an older revision, or another operation at the
    /// applied one, with `conflict`.
    fn ordering_daemon(
        socket: &Path,
        requests: usize,
    ) -> std::thread::JoinHandle<Vec<serde_json::Value>> {
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let mut applied: Option<(u64, String, String)> = None;
            let mut seen = Vec::new();
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&stream)
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let op = request["op"].as_str().unwrap().to_owned();
                let revision = request["revision"].as_u64().unwrap();
                let id = request["operation_id"].as_str().unwrap().to_owned();
                let conflict = applied.as_ref().is_some_and(|(r, i, o)| {
                    revision < *r || (revision == *r && (i != &id || o != &op))
                });
                let reply = if conflict {
                    serde_json::json!({"ok": false, "error": {
                        "code": "conflict", "message": "stale fence operation"}})
                } else {
                    let replayed = applied
                        .as_ref()
                        .is_some_and(|(r, i, _)| *r == revision && *i == id);
                    let fenced = op == "stop_account";
                    applied = Some((revision, id, op));
                    serde_json::json!({"ok": true, "result": {
                        "key": request["key"], "fence_epoch": revision, "fenced": fenced,
                        "stopped": [], "replayed": replayed, "account_present": true}})
                };
                writeln!(stream, "{reply}").unwrap();
                seen.push(request);
            }
            seen
        })
    }

    #[test]
    fn a_delayed_admit_never_undoes_a_newer_confirmed_stop() {
        use crate::provider_account::{AccountFenceAction, AccountFenceRequest};
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let server = ordering_daemon(&socket, 4);
        let store = store(root.path());
        store.set_account_sandbox(socket, Some("service".into()));
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let request = |action, revision: u64| AccountFenceRequest {
            operation_id: format!("op-{revision}"),
            provider_id: "antigravity".into(),
            account_id: account.clone(),
            action,
            revision: Some(revision),
        };
        let stop = request(AccountFenceAction::Stop, 5);
        let observed = store.account_sandbox_fence(&stop).unwrap();
        assert!(observed.fenced && observed.request == stop);
        // The admit of revision 4 arrives late: refused, never confirmed.
        let error = store
            .account_sandbox_fence(&request(AccountFenceAction::Admit, 4))
            .unwrap_err();
        assert!(format!("{error:#}").contains("conflict"), "{error:#}");
        // A retry of the applied stop repeats it; a newer admit applies.
        assert!(store.account_sandbox_fence(&stop).unwrap().fenced);
        let admitted = store
            .account_sandbox_fence(&request(AccountFenceAction::Admit, 6))
            .unwrap();
        assert!(!admitted.fenced);
        let requests = server.join().unwrap();
        assert_eq!(requests[0]["revision"], 5);
        assert_eq!(requests[0]["operation_id"], "op-5");
        assert_eq!(requests[1]["op"], "admit_account");
    }

    #[test]
    fn a_revocation_fences_then_removes_the_account_under_its_operation() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let key = daemon::account_key("service", account.as_str());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut requests = Vec::new();
            for _ in 0..2 {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let reply = if request["op"] == "stop_account" {
                    serde_json::json!({"ok": true, "result": {"key": request["key"],
                        "fence_epoch": 1, "fenced": true, "stopped": []}})
                } else {
                    serde_json::json!({"ok": true, "result": {"key": request["key"],
                        "removed": true, "home_removed": true, "user_removed": true,
                        "operation_id": request["operation_id"], "replayed": false}})
                };
                writeln!(writer, "{reply}").unwrap();
                requests.push(request);
            }
            requests
        });
        let store = store(root.path());
        store.set_account_sandbox(socket, Some("service".into()));
        let request = crate::provider_account::AccountRemoveRequest {
            operation_id: "revoke-op".into(),
            provider_id: "antigravity".into(),
            account_id: account,
        };
        let removed = store.account_sandbox_remove(&request).unwrap();
        assert!(removed.home_removed && removed.user_removed && removed.request == request);
        let requests = server.join().unwrap();
        assert_eq!(
            requests,
            [
                serde_json::json!({"op": "stop_account", "key": key}),
                serde_json::json!({"op": "remove_account", "key": key,
                    "operation_id": "revoke-op"}),
            ]
        );
        let mut codex = request;
        codex.provider_id = "codex".into();
        assert!(store.account_sandbox_remove(&codex).is_err());
    }

    #[test]
    fn a_machine_without_the_daemon_confirms_removal_only_without_sandboxed_profiles() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        // Production always configures the socket path, installed or not.
        store.set_account_sandbox(root.path().join("absent.sock"), Some("service".into()));
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let request = crate::provider_account::AccountRemoveRequest {
            operation_id: "revoke-op".into(),
            provider_id: "antigravity".into(),
            account_id: account.clone(),
        };
        let removed = store.account_sandbox_remove(&request).unwrap();
        assert!(removed.home_removed && removed.user_removed);
        // An unreadable profile may be sandboxed: the removal stays pending.
        let scoped = store.for_account(&account).unwrap();
        std::fs::create_dir_all(
            scoped
                .native_profile_root("antigravity")
                .join("generations/1"),
        )
        .unwrap();
        assert!(store.account_sandbox_remove(&request).is_err());
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

    #[test]
    fn a_lost_workers_units_are_found_by_its_session_record() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let unit = |name: &str, session: &str| {
            serde_json::json!({"unit": name, "mode": "runtime", "invocation_id": "i",
                "boot_id": "b", "credential_generation": 7, "session_id": session,
                "state": "active", "alive": true, "owned": false})
        };
        let units = serde_json::json!({"ok": true, "result": {
            "key": "k", "boot_id": "b", "fenced": null,
            "credential": {"generation": 7, "state": "signed_in"},
            "units": [unit("cga-k-runtime-1.service", "lost"),
                      unit("cga-k-runtime-2.service", "other")],
            "unrecorded": [],
        }});
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in [
                units,
                serde_json::json!({"ok": true, "result": {"unit": "u", "stopped": true}}),
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
        let launch = |session: &str, provider: &str| -> crate::runtime_wire::StartSession {
            serde_json::from_value(serde_json::json!({
                "session_id": session, "provider": provider,
                "provider_account_id": uuid::Uuid::new_v4().to_string(),
                "provider_version": "1", "provider_generation_digest": "digest",
                "provider_auth_generation": 7, "cwd": "/fixture", "generation": "worker"
            }))
            .unwrap()
        };
        // No worker snapshot: the durable launch record names the session.
        store
            .begin_bound_account_worker(&launch("lost", "antigravity"))
            .unwrap();
        store.confirm_session_account_units_gone("lost").unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests[0]["op"], "list_units");
        assert_eq!(
            requests[1],
            serde_json::json!({"op": "stop_unit", "unit": "cga-k-runtime-1.service"})
        );
        // An unsandboxed binding and an unknown session have no unit.
        store
            .begin_bound_account_worker(&launch("codex", "codex"))
            .unwrap();
        store.confirm_session_account_units_gone("codex").unwrap();
        store.confirm_session_account_units_gone("unknown").unwrap();
    }
}
