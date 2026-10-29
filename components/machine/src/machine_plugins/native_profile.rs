//! Machine-owned native profiles. Credential files are neither opened nor
//! copied here; native login and runtime use the same private home in place.
use super::*;
use crate::provider_account::{NativeIdentity, NativeProfileReceipt, prepare_private_directory};
use serde::{Deserialize, Serialize};
mod failed;
mod logout;
mod processes;
pub use logout::LogoutPreparation;

pub struct PreparedNativeLogin {
    pub home: PathBuf,
    pub command: PathBuf,
    pub method: cowboy_provider_sdk::AuthMethod,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
// Independent historical evidence flags preserve legacy on-disk meanings;
// merging them into a lifecycle enum would imply transitions not yet accepted.
#[allow(clippy::struct_excessive_bools)]
struct Profile {
    schema: u32,
    account: crate::provider_account::ProviderAccountId,
    provider: String,
    generation: u64,
    request: String,
    contract: String,
    version: String,
    digest: String,
    method: String,
    identity: Option<NativeIdentity>,
    #[serde(default)]
    revoked: bool,
    /// Historical leader-exit evidence, not descendant termination or logout.
    #[serde(default)]
    leader_exited: bool,
    #[serde(default)]
    process_tree_exited: bool,
    #[serde(default)]
    login_succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    logout: Option<logout::Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process: Option<processes::Reservation>,
}

impl MachinePluginStore {
    /// Retain the exact executable needed to reconcile native login/logout.
    /// Caller holds `auth_candidates` through retirement to fence new profiles.
    pub(super) fn retained_native_generations(&self, provider: &str) -> Result<BTreeSet<String>> {
        let accounts = self.auth_base.join("accounts");
        match fs::symlink_metadata(&accounts) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeSet::new());
            }
            result => ensure!(result?.is_dir(), "invalid account directory"),
        }
        let mut keep = BTreeSet::new();
        for entry in fs::read_dir(accounts)? {
            let entry = entry?;
            ensure!(entry.file_type()?.is_dir(), "invalid account directory");
            let account = entry.file_name().to_string_lossy().into_owned();
            let root = entry.path().join("providers").join(provider).join("native");
            match fs::symlink_metadata(&root) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                result => ensure!(result?.is_dir(), "invalid native profile root"),
            }
            let generations = root.join("generations");
            ensure!(
                fs::symlink_metadata(&generations)?.is_dir(),
                "invalid native generations"
            );
            for generation in fs::read_dir(&generations)? {
                let name = generation?.file_name().to_string_lossy().into_owned();
                let id = name.parse::<u64>()?;
                ensure!(
                    id > 0 && id.to_string() == name,
                    "invalid native generation"
                );
                let profile: Profile = serde_json::from_slice(&native_identity::read_login_file(
                    &root,
                    &format!("generations/{name}/profile.json"),
                    16 * 1024,
                )?)?;
                ensure!(
                    profile.schema == 1
                        && profile.account.as_str() == account
                        && profile.provider == provider
                        && profile.generation == id,
                    "native profile scope mismatch"
                );
                let home = generations.join(name).join("home");
                if profile.revoked
                    && matches!(fs::symlink_metadata(&home),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                {
                    continue;
                }
                let digest = profile.digest.trim_start_matches("sha256:");
                ensure!(
                    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "invalid native package digest"
                );
                keep.insert(digest.to_owned());
            }
        }
        Ok(keep)
    }

    /// Metadata lookup must not create an account or run authentication.
    pub fn observe_native_profile(
        &self,
        account: &crate::provider_account::ProviderAccountId,
        provider: &str,
        generation: u64,
        request: &str,
    ) -> Result<NativeProfileReceipt> {
        ensure!(
            self.account_id.is_default() && !account.is_default(),
            "named native profile required"
        );
        let mut scoped = self.clone();
        scoped.account_id = account.clone();
        scoped.auth_root = self.auth_base.join("accounts").join(account.as_str());
        scoped.native_profile_receipt(provider, generation, request)
    }

    /// Called only by the authenticated wipe writer. Retain operation fences
    /// and bound identity; deleting them would let old login requests be reused.
    pub(super) fn revoke_native_profiles(&self, provider: &str) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        let root = self.native_profile_root(provider);
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            result => ensure!(result?.is_dir(), "native profile root is linked or invalid"),
        }
        // Fence every generation before the first per-profile mutation. A
        // partial cleanup or process loss must not leave later profiles usable.
        atomic_write(&root.join("revoking"), b"1", 0o600)?;
        fs::File::open(&root)?.sync_all()?;
        let generations = root.join("generations");
        prepare_private_directory(&generations)?;
        for entry in fs::read_dir(&generations)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let generation = name.parse::<u64>().context("invalid native generation")?;
            ensure!(
                generation > 0 && generation.to_string() == name,
                "invalid native generation"
            );
            let mut profile: Profile = serde_json::from_slice(&native_identity::read_login_file(
                &root,
                &format!("generations/{name}/profile.json"),
                16 * 1024,
            )?)?;
            ensure!(
                profile.schema == 1
                    && profile.account == self.account_id
                    && profile.provider == provider
                    && profile.generation == generation,
                "native profile scope mismatch"
            );
            profile.revoked = true;
            ensure!(
                profile
                    .logout
                    .as_ref()
                    .is_none_or(|logout| logout.succeeded == Some(true)),
                "native logout is unconfirmed; preserve its home"
            );
            let directory = generations.join(name);
            // A crash after this durable marker can never readmit the old home.
            atomic_write(
                &directory.join("profile.json"),
                &serde_json::to_vec(&profile)?,
                0o600,
            )?;
            fs::File::open(&directory)?.sync_all()?;
            remove_projection::remove(&directory.join("home"))?;
            fs::File::open(&directory)?.sync_all()?;
        }
        fs::remove_file(root.join("revoking"))?;
        fs::File::open(&root)?.sync_all()?;
        Ok(())
    }

    pub(super) fn native_credential_directories(
        package: &ProviderPackage,
        home: &Path,
    ) -> Result<BTreeMap<String, String>> {
        let mut directories = BTreeMap::new();
        for credential in &package.manifest.authentication.credential_files {
            let relative = Path::new(&credential.relative_path);
            ensure!(
                relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
                "invalid native credential path"
            );
            let mut directory = home.to_path_buf();
            for part in relative
                .parent()
                .context("native credential parent missing")?
                .components()
            {
                directory.push(part);
                prepare_private_directory(&directory)?;
            }
            directories.insert(
                credential.bundle_key.clone(),
                directory.to_string_lossy().into_owned(),
            );
        }
        Ok(directories)
    }

    fn native_profile_root(&self, provider: &str) -> PathBuf {
        self.auth_provider_root(provider).join("native")
    }

    /// Reserve a fresh native home exactly once. An existing operation, even
    /// an incomplete one, never permits dispatching another login process.
    pub fn prepare_native_profile(
        &self,
        provider: &str,
        request: &str,
        spec: &crate::provider_account::NativeLoginSpec,
    ) -> Result<PreparedNativeLogin> {
        let generation = spec.generation;
        let method = &spec.method;
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(request)?;
        ensure!(
            !self.account_id.is_default() && generation > 0,
            "named native profile required"
        );
        let (package, digest) = self
            .active_package(provider)?
            .context("Provider must be installed")?;
        ensure!(
            package.manifest.version == spec.provider_version && digest == spec.generation_digest,
            "native login installation changed"
        );
        let auth = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|candidate| candidate.id == *method)
            .context("native login method unavailable")?;
        ensure!(
            auth.native_identity.is_some(),
            "native identity verification unavailable"
        );
        ensure!(
            matches!(
                auth.executor,
                cowboy_provider_sdk::AuthExecutor::CommandV1 { .. }
            ),
            "native command login required"
        );
        let cowboy_provider_sdk::AuthExecutor::CommandV1 { component, .. } = &auth.executor else {
            unreachable!("validated command executor")
        };
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let requirement = payload
            .private_components
            .iter()
            .find(|candidate| candidate.kind == component.kind && candidate.slot == component.slot)
            .context("native authentication component unavailable")?;
        let (_, package_path) = self.package_for_generation(provider, &digest)?;
        let command = runtime_command(&package_path, &requirement.command)?;
        let root = self.native_profile_root(provider);
        for path in [
            self.auth_provider_root(provider),
            root.clone(),
            root.join("generations"),
        ] {
            prepare_private_directory(&path)?;
        }
        self.retire_failed_native_logins(provider, request)?;
        validate_reservation(&root, request)?;
        let directory = root.join("generations").join(generation.to_string());
        // create_dir is exclusive, unlike prepare_private_directory. Retain an
        // interrupted reservation rather than infer that login never started.
        fs::create_dir(&directory).context("native profile generation already reserved")?;
        prepare_private_directory(&directory)?;
        let profile = Profile {
            schema: 1,
            account: self.account_id.clone(),
            provider: provider.into(),
            generation,
            request: request.into(),
            contract: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            method: method.clone(),
            version: package.manifest.version.clone(),
            digest,
            identity: None,
            revoked: false,
            leader_exited: false,
            process_tree_exited: false,
            login_succeeded: false,
            logout: None,
            process: Some(processes::Reservation::new()?),
        };
        atomic_write(
            &directory.join("profile.json"),
            &serde_json::to_vec(&profile)?,
            0o600,
        )?;
        let home = directory.join("home");
        prepare_private_directory(&home)?;
        fs::File::open(&directory)?.sync_all()?;
        fs::File::open(root.join("generations"))?.sync_all()?;
        Ok(PreparedNativeLogin {
            home,
            command,
            method: auth.clone(),
        })
    }

    /// After native login exits, read only its signed identity recipe. The
    /// caller must still bind this evidence at the Controller before admission.
    pub fn complete_native_profile(
        &self,
        provider: &str,
        generation: u64,
        request: &str,
    ) -> Result<NativeProfileReceipt> {
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(request)?;
        let (_, reserved) = self.read_native_profile_metadata(provider, generation)?;
        ensure!(reserved.request == request, "native login request mismatch");
        ensure!(
            reserved.leader_exited && reserved.process_tree_exited && reserved.login_succeeded,
            "native login success is unconfirmed"
        );
        // A plugin update must not substitute a new identity recipe for a
        // process that was launched under the previously reserved release.
        let (package, _) = self.package_for_generation(provider, &reserved.digest)?;
        let (directory, mut profile) = self.read_native_profile(&package, generation)?;
        ensure!(profile.request == request, "native login request mismatch");
        ensure!(
            profile.version == package.manifest.version,
            "native login installation changed"
        );
        if profile.identity.is_some() {
            return self.native_profile_receipt(provider, generation, request);
        }
        let method = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|method| method.id == profile.method)
            .context("native login method unavailable")?;
        let recipe = method
            .native_identity
            .as_ref()
            .context("native identity verification unavailable")?;
        let identity = native_identity::read(&directory.join("home"), recipe)?;
        let root = self.native_profile_root(provider);
        match native_identity::read_login_file(&root, "identity.json", 4096) {
            Ok(bytes) => {
                let bound: NativeIdentity = serde_json::from_slice(&bytes)?;
                ensure!(
                    bound == identity,
                    "native profile belongs to another subscription"
                );
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                atomic_write(
                    &root.join("identity.json"),
                    &serde_json::to_vec(&identity)?,
                    0o600,
                )?;
                fs::File::open(&root)?.sync_all()?;
            }
            Err(error) => return Err(error),
        }
        profile.identity = Some(identity.clone());
        atomic_write(
            &directory.join("profile.json"),
            &serde_json::to_vec(&profile)?,
            0o600,
        )?;
        fs::File::open(&directory)?.sync_all()?;
        self.native_profile_receipt(provider, generation, request)
    }

    /// Recover completion without rerunning login or opening native credential
    /// files. Retained metadata is not sufficient after revocation or home loss.
    pub fn native_profile_receipt(
        &self,
        provider: &str,
        generation: u64,
        request: &str,
    ) -> Result<NativeProfileReceipt> {
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(request)?;
        let (directory, profile) = self.read_native_profile_metadata(provider, generation)?;
        ensure!(profile.request == request, "native login request mismatch");
        ensure!(
            profile.leader_exited && profile.process_tree_exited && profile.login_succeeded,
            "native login success is unconfirmed"
        );
        let identity = profile
            .identity
            .context("native profile login is incomplete")?;
        let bound: NativeIdentity = serde_json::from_slice(&native_identity::read_login_file(
            &self.native_profile_root(provider),
            "identity.json",
            4096,
        )?)?;
        ensure!(identity == bound, "native profile identity mismatch");
        ensure!(
            fs::symlink_metadata(directory.join("home"))?.is_dir(),
            "native profile home is missing or linked"
        );
        Ok(NativeProfileReceipt {
            account_id: profile.account,
            provider_id: profile.provider,
            request_id: profile.request,
            generation: profile.generation,
            method: profile.method,
            provider_version: profile.version,
            generation_digest: profile.digest,
            auth_contract_fingerprint: profile.contract,
            identity,
        })
    }

    /// Record only after `wait()` has confirmed this reserved login's leader.
    /// A dropped task or missing process table entry is not eligible evidence.
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_native_login_exit(
        &self,
        provider: &str,
        generation: u64,
        request: &str,
        login_succeeded: bool,
    ) -> Result<()> {
        self.record_native_login_outcome(provider, generation, request, login_succeeded, false)
    }

    pub fn record_native_login_tree_exit(
        &self,
        provider: &str,
        generation: u64,
        request: &str,
        login_succeeded: bool,
    ) -> Result<()> {
        self.record_native_login_outcome(provider, generation, request, login_succeeded, true)
    }

    fn record_native_login_outcome(
        &self,
        provider: &str,
        generation: u64,
        request: &str,
        login_succeeded: bool,
        process_tree_exited: bool,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        validate_plugin_id(provider)?;
        validate_auth_candidate_id(request)?;
        let (directory, mut profile) = self.read_native_profile_metadata(provider, generation)?;
        ensure!(profile.request == request, "native login request mismatch");
        ensure!(
            !profile.leader_exited || profile.login_succeeded == login_succeeded,
            "native login exit conflict"
        );
        ensure!(
            !profile.leader_exited || profile.process_tree_exited == process_tree_exited,
            "native login containment evidence conflict"
        );
        if !profile.leader_exited {
            profile.leader_exited = true;
            profile.process_tree_exited = process_tree_exited;
            profile.login_succeeded = login_succeeded;
            atomic_write(
                &directory.join("profile.json"),
                &serde_json::to_vec(&profile)?,
                0o600,
            )?;
        }
        // Repeat sync after an unknown previous result, even when bytes match.
        fs::File::open(&directory)?.sync_all()?;
        Ok(())
    }

    pub(super) fn ensure_native_release(
        &self,
        package: &ProviderPackage,
        generation: u64,
        digest: &str,
    ) -> Result<()> {
        let (_, profile) = self.read_native_profile(package, generation)?;
        if profile.digest != digest {
            // `package` must be exactly the release the caller resolved, and
            // read_native_profile already proved it compatible with the login.
            let (resolved, _) = self
                .package_for_generation(&package.manifest.id, digest)
                .context("native profile release mismatch")?;
            ensure!(
                resolved.manifest.version == package.manifest.version,
                "native profile release mismatch"
            );
        }
        Ok(())
    }

    fn read_native_profile(
        &self,
        package: &ProviderPackage,
        generation: u64,
    ) -> Result<(PathBuf, Profile)> {
        self.read_native_profile_with(package, generation, &|provider, digest| {
            self.package_for_generation(provider, digest)
                .map(|(package, _)| package)
        })
    }

    /// [`Self::read_native_profile`] with an explicit retained-package lookup.
    fn read_native_profile_with(
        &self,
        package: &ProviderPackage,
        generation: u64,
        retained: &dyn Fn(&str, &str) -> Result<ProviderPackage>,
    ) -> Result<(PathBuf, Profile)> {
        let (directory, profile) =
            self.read_native_profile_metadata(&package.manifest.id, generation)?;
        ensure!(
            profile.contract == package.manifest.compatibility.auth_contract_fingerprint,
            "native profile scope or contract mismatch"
        );
        // A later release may use this login's home in place only when its
        // signed authentication is identical to the retained signed-in release.
        if profile.version != package.manifest.version {
            let signed_in = retained(&package.manifest.id, &profile.digest)
                .context("native profile release is not retained")?;
            ensure!(
                signed_in.manifest.version == profile.version
                    && crate::provider_account::native_release_compatible(
                        &signed_in.manifest,
                        &package.manifest
                    ),
                "native profile scope or contract mismatch"
            );
        }
        Ok((directory, profile))
    }

    fn read_native_profile_metadata(
        &self,
        provider: &str,
        generation: u64,
    ) -> Result<(PathBuf, Profile)> {
        ensure_no_revocation(&self.native_profile_root(provider))?;
        let result = self.read_native_profile_for_cleanup(provider, generation)?;
        ensure!(!result.1.revoked, "native profile was revoked");
        ensure!(result.1.logout.is_none(), "native logout has been reserved");
        Ok(result)
    }

    fn read_native_profile_for_cleanup(
        &self,
        provider: &str,
        generation: u64,
    ) -> Result<(PathBuf, Profile)> {
        validate_plugin_id(provider)?;
        ensure!(
            !self.account_id.is_default() && generation > 0,
            "named native profile required"
        );
        let root = self.native_profile_root(provider);
        // Traverse every intermediate directory with NOFOLLOW, rather than
        // opening only the final path with a trusted-looking account prefix.
        let bytes = native_identity::read_login_file(
            &root,
            &format!("generations/{generation}/profile.json"),
            16 * 1024,
        )?;
        let profile: Profile =
            serde_json::from_slice(&bytes).context("invalid native profile metadata")?;
        ensure!(
            profile.schema == 1
                && profile.account == self.account_id
                && profile.provider == provider
                && profile.generation == generation,
            "native profile scope or contract mismatch"
        );
        Ok((
            root.join("generations").join(generation.to_string()),
            profile,
        ))
    }

    pub(super) fn native_profile_home(
        &self,
        package: &ProviderPackage,
        generation: u64,
    ) -> Result<PathBuf> {
        self.native_profile_home_with(package, generation, &|provider, digest| {
            self.package_for_generation(provider, digest)
                .map(|(package, _)| package)
        })
    }

    /// [`Self::native_profile_home`] with an explicit retained-package lookup.
    pub(super) fn native_profile_home_with(
        &self,
        package: &ProviderPackage,
        generation: u64,
        retained: &dyn Fn(&str, &str) -> Result<ProviderPackage>,
    ) -> Result<PathBuf> {
        let (directory, profile) = self.read_native_profile_with(package, generation, retained)?;
        ensure!(
            profile.leader_exited && profile.process_tree_exited && profile.login_succeeded,
            "native login success is unconfirmed"
        );
        let identity = profile
            .identity
            .context("native profile login is incomplete")?;
        let bound: NativeIdentity = serde_json::from_slice(&native_identity::read_login_file(
            &self.native_profile_root(&package.manifest.id),
            "identity.json",
            4096,
        )?)?;
        ensure!(identity == bound, "native profile identity mismatch");
        let home = directory.join("home");
        ensure!(
            fs::symlink_metadata(&home)?.is_dir(),
            "native profile home is missing or linked"
        );
        prepare_private_directory(&home)?;
        Ok(home)
    }
}

fn ensure_no_revocation(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root.join("revoking")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("checking native profile revocation"),
        Ok(_) => anyhow::bail!("native profile revocation requires reconciliation"),
    }
}

/// Retained tombstones are not live credential projections. Unknown or pending
/// reservations still fence uninstall until their process ownership is resolved.
pub(super) fn ensure_revoked(root: &Path, account: &str, provider: &str) -> Result<()> {
    ensure_no_revocation(root)?;
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        result => ensure!(result?.is_dir(), "invalid native profile root"),
    }
    let generations = root.join("generations");
    ensure!(
        fs::symlink_metadata(&generations)?.is_dir(),
        "invalid native generations"
    );
    for entry in fs::read_dir(&generations)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        let generation = name.parse::<u64>()?;
        ensure!(
            generation > 0 && generation.to_string() == name,
            "invalid native generation"
        );
        let profile: Profile = serde_json::from_slice(&native_identity::read_login_file(
            root,
            &format!("generations/{name}/profile.json"),
            16 * 1024,
        )?)?;
        ensure!(
            profile.schema == 1
                && profile.account.as_str() == account
                && profile.provider == provider
                && profile.generation == generation
                && profile.revoked,
            "Revoke named Provider account projections before uninstalling this Plugin"
        );
        match fs::symlink_metadata(generations.join(name).join("home")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => anyhow::bail!("native profile cleanup is incomplete"),
        }
    }
    Ok(())
}

fn validate_reservation(root: &Path, request: &str) -> Result<()> {
    ensure_no_revocation(root)?;
    for entry in fs::read_dir(root.join("generations"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        ensure!(
            name.parse::<u64>()
                .is_ok_and(|id| id > 0 && id.to_string() == name),
            "unknown native profile reservation"
        );
        let profile: Profile = serde_json::from_slice(&native_identity::read_login_file(
            root,
            &format!("generations/{name}/profile.json"),
            16 * 1024,
        )?)?;
        if profile.revoked {
            // Wiping an interrupted login retains its operation fence but
            // cannot manufacture an identity. Only a completely removed home
            // permits a fresh operation; a surviving writer can recreate it.
            match fs::symlink_metadata(root.join("generations").join(&name).join("home")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => anyhow::bail!("native profile cleanup is incomplete"),
            }
        } else {
            ensure!(
                profile.identity.is_some(),
                "native login reconciliation required"
            );
        }
        ensure!(
            profile.request != request,
            "native login operation already reserved"
        );
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::machine_plugins) mod tests {
    use super::*;

    #[test]
    fn native_launch_rejects_another_release_even_with_the_same_auth_contract() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let mut package = package();
        let home = ready(&scoped, &package);
        assert_eq!(
            scoped
                .prepare_launch_auth_home("claude-code", &package, 1, "fixture")
                .unwrap(),
            home
        );
        assert!(
            scoped
                .prepare_launch_auth_home("claude-code", &package, 1, "different-digest")
                .is_err()
        );
        package.manifest.version = "999.0.0".into();
        assert!(
            scoped
                .prepare_launch_auth_home("claude-code", &package, 1, "fixture")
                .is_err()
        );
        assert!(scoped.native_profile_home(&package, 1).is_err());
        assert!(
            home.is_dir(),
            "rejection must not destroy the original profile"
        );
    }

    #[test]
    fn quota_observation_requires_the_exact_native_identity_and_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        ready(&scoped, &package);
        let profile = store
            .observe_native_profile(&id, "claude-code", 1, "login-1")
            .unwrap();
        store.validate_native_usage(&profile).unwrap();
        for field in [
            "account_id",
            "provider_id",
            "generation",
            "identity",
            "generation_digest",
            "request_id",
        ] {
            let mut wrong = profile.clone();
            match field {
                "account_id" => {
                    wrong.account_id = crate::provider_account::ProviderAccountId::default();
                }
                "provider_id" => wrong.provider_id = "codex".into(),
                "generation" => wrong.generation += 1,
                "identity" => wrong.identity.subject = "another-user".into(),
                "generation_digest" => wrong.generation_digest = "other".into(),
                _ => wrong.request_id = "other-login".into(),
            }
            assert!(store.validate_native_usage(&wrong).is_err(), "{field}");
        }
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(store.validate_native_usage(&profile).is_err());
    }

    #[test]
    fn leader_only_login_cannot_complete_recover_or_launch_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = root.for_account(&id).unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let path = home.parent().unwrap().join("profile.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("process_tree_exited");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        drop(scoped);
        drop(root);
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = root.for_account(&id).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            scoped
                .complete_native_profile("claude-code", 1, "login-1")
                .is_err()
        );
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
        assert!(scoped.native_profile_home(&package, 1).is_err());
        assert!(
            scoped
                .record_native_login_tree_exit("claude-code", 1, "login-1", true)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn native_tree_evidence_survives_cleanup_without_upgrading_legacy_exits() {
        let temp = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = root.for_account(&id).unwrap();
        let home = ready(&store, &package());
        let path = home.parent().unwrap().join("profile.json");
        let mut legacy: Profile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        legacy.process_tree_exited = false;
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(
            store
                .record_native_login_tree_exit("claude-code", 1, "login-1", true)
                .is_err()
        );
        let (directory, mut profile) = store
            .read_native_profile_metadata("claude-code", 1)
            .unwrap();
        profile.leader_exited = false;
        profile.login_succeeded = false;
        fs::write(
            directory.join("profile.json"),
            serde_json::to_vec(&profile).unwrap(),
        )
        .unwrap();
        for _ in 0..2 {
            store
                .record_native_login_tree_exit("claude-code", 1, "login-1", true)
                .unwrap();
        }
        assert!(
            store
                .record_native_login_exit("claude-code", 1, "login-1", true)
                .is_err()
        );
        let (_, mut profile) = store
            .read_native_profile_metadata("claude-code", 1)
            .unwrap();
        profile.logout = Some(logout::Record {
            request: "logout-1".into(),
            succeeded: None,
            process_tree_exited: false,
            process: None,
        });
        logout::persist(&directory, &profile).unwrap();
        assert!(
            !store
                .native_auth_trees_exited("claude-code", 1, "login-1", "logout-1")
                .unwrap()
        );
        for _ in 0..2 {
            store
                .record_native_logout_tree_exit("claude-code", 1, "logout-1", true)
                .unwrap();
        }
        assert!(
            store
                .record_native_logout_exit("claude-code", 1, "logout-1", true)
                .is_err()
        );
        store
            .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
            .unwrap();
        assert!(!home.exists());
        drop(store);
        drop(root);
        let reopened = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
        .for_account(&id)
        .unwrap();
        assert!(
            reopened
                .native_auth_trees_exited("claude-code", 1, "login-1", "logout-1")
                .unwrap()
        );
        assert!(
            reopened
                .native_auth_trees_exited("claude-code", 1, "login-1", "wrong")
                .is_err()
        );
        let mut old: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("profile.json")).unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("process_tree_exited");
        old["logout"]
            .as_object_mut()
            .unwrap()
            .remove("process_tree_exited");
        let old: Profile = serde_json::from_value(old).unwrap();
        assert!(!old.process_tree_exited && !old.logout.unwrap().process_tree_exited);
    }
    use std::os::unix::fs::symlink;

    #[test]
    fn interrupted_revocation_fences_all_profiles_across_restart() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let root = scoped.native_profile_root("claude-code");
        let profile_path = root.join("generations/1/profile.json");
        let original = fs::read(&profile_path).unwrap();
        let damaged = root.join("generations/2");
        fs::create_dir(&damaged).unwrap();
        fs::write(damaged.join("profile.json"), b"invalid").unwrap();
        assert!(scoped.revoke_native_profiles("claude-code").is_err());
        assert!(root.join("revoking").is_file());
        // Model a generation not reached before failure, independently of
        // filesystem iteration order and of its individual revoked flag.
        fs::write(&profile_path, &original).unwrap();
        fs::create_dir_all(&home).unwrap();
        drop(scoped);
        drop(store);
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = store.for_account(&id).unwrap();
        assert!(scoped.native_profile_home(&package, 1).is_err());
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
        assert!(validate_reservation(&root, "new-login").is_err());
        assert!(ensure_revoked(&root, id.as_str(), "claude-code").is_err());
        // Repair only this synthetic corrupt fixture, then resume the wipe.
        fs::remove_file(damaged.join("profile.json")).unwrap();
        fs::remove_dir(&damaged).unwrap();
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(!root.join("revoking").exists());
        assert!(!home.exists());
        assert!(scoped.native_profile_home(&package, 1).is_err());
        assert!(validate_reservation(&root, "new-login").is_ok());
        assert!(ensure_revoked(&root, id.as_str(), "claude-code").is_ok());
    }

    pub(in crate::machine_plugins) fn package() -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(
            include_str!("../../../../plugins/claude-code/provider.json"),
        )
        .unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    #[test]
    fn wiped_incomplete_login_allows_fresh_request_but_never_replay_or_recreated_home() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let home = ready(&scoped, &package());
        let root = scoped.native_profile_root("claude-code");
        let metadata = root.join("generations/1/profile.json");
        let mut profile: Profile = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
        profile.identity = None;
        fs::write(&metadata, serde_json::to_vec(&profile).unwrap()).unwrap();
        assert!(validate_reservation(&root, "new-login").is_err());
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(validate_reservation(&root, "new-login").is_ok());
        store
            .ensure_no_named_auth_materializations("claude-code")
            .unwrap();
        assert!(validate_reservation(&root, "login-1").is_err());
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
        drop(scoped);
        drop(store);
        let reopened = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = reopened.for_account(&id).unwrap();
        assert!(validate_reservation(&root, "new-login").is_ok());
        reopened
            .ensure_no_named_auth_materializations("claude-code")
            .unwrap();
        assert!(validate_reservation(&root, "login-1").is_err());
        fs::create_dir(&home).unwrap();
        assert!(validate_reservation(&root, "new-login").is_err());
        assert!(
            reopened
                .ensure_no_named_auth_materializations("claude-code")
                .is_err()
        );
        scoped.revoke_native_profiles("claude-code").unwrap();
        symlink("missing", &home).unwrap();
        assert!(validate_reservation(&root, "new-login").is_err());
        assert!(
            reopened
                .ensure_no_named_auth_materializations("claude-code")
                .is_err()
        );
    }

    pub(in crate::machine_plugins) fn ready(
        store: &MachinePluginStore,
        package: &ProviderPackage,
    ) -> PathBuf {
        let root = store.native_profile_root("claude-code");
        let directory = root.join("generations/1");
        fs::create_dir_all(directory.join("home")).unwrap();
        let identity = NativeIdentity {
            authority: "fixture".into(),
            subject: "personal".into(),
            tenant: None,
        };
        fs::write(
            root.join("identity.json"),
            serde_json::to_vec(&identity).unwrap(),
        )
        .unwrap();
        let profile = Profile {
            schema: 1,
            account: store.account_id.clone(),
            provider: "claude-code".into(),
            generation: 1,
            request: "login-1".into(),
            contract: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            method: "fixture".into(),
            version: package.manifest.version.clone(),
            digest: "fixture".into(),
            identity: Some(identity),
            revoked: false,
            leader_exited: true,
            process_tree_exited: true,
            login_succeeded: true,
            logout: None,
            process: None,
        };
        fs::write(
            directory.join("profile.json"),
            serde_json::to_vec(&profile).unwrap(),
        )
        .unwrap();
        directory.join("home")
    }

    #[test]
    fn native_logout_cleanup_is_scoped_and_resumes_after_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = root.for_account(&id).unwrap();
        let other_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let other = root.for_account(&other_id).unwrap();
        let package = package();
        let home = ready(&store, &package);
        let other_home = ready(&other, &package);
        let (directory, mut profile) = store
            .read_native_profile_metadata("claude-code", 1)
            .unwrap();
        for succeeded in [None, Some(false)] {
            profile.logout = Some(logout::Record {
                process_tree_exited: false,
                request: "logout-1".into(),
                succeeded,
                process: None,
            });
            atomic_write(
                &directory.join("profile.json"),
                &serde_json::to_vec(&profile).unwrap(),
                0o600,
            )
            .unwrap();
            assert!(
                store
                    .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
                    .is_err()
            );
            assert!(home.exists());
        }
        profile.logout.as_mut().unwrap().succeeded = Some(true);
        // A successful leader without subtree proof must retain credentials.
        logout::persist(&directory, &profile).unwrap();
        assert!(
            store
                .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
                .is_err()
        );
        assert!(home.exists());
        profile.logout.as_mut().unwrap().process_tree_exited = true;
        logout::persist(&directory, &profile).unwrap();
        for (login, operation) in [("other-login", "logout-1"), ("login-1", "other-logout")] {
            assert!(
                store
                    .cleanup_logged_out_native_profile("claude-code", 1, login, operation)
                    .is_err()
            );
            assert!(home.exists());
        }
        assert!(
            other
                .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
                .is_err()
        );
        // Simulate interruption after committing the tombstone but before
        // deleting the home, including an account-wide wipe fence.
        profile.revoked = true;
        atomic_write(
            &directory.join("profile.json"),
            &serde_json::to_vec(&profile).unwrap(),
            0o600,
        )
        .unwrap();
        atomic_write(
            &store.native_profile_root("claude-code").join("revoking"),
            b"1",
            0o600,
        )
        .unwrap();
        drop(store);
        drop(other);
        drop(root);
        let restarted = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let store = restarted.for_account(&id).unwrap();
        assert!(matches!(
            store
                .prepare_native_logout("claude-code", 1, "login-1", "logout-1")
                .unwrap(),
            LogoutPreparation::Observed(Some(true))
        ));
        for _ in 0..2 {
            store
                .cleanup_logged_out_native_profile("claude-code", 1, "login-1", "logout-1")
                .unwrap();
            assert!(!home.exists());
            assert!(other_home.exists());
            assert!(directory.join("profile.json").exists());
        }
        assert!(store.native_profile_home(&package, 1).is_err());
    }

    #[test]
    fn logout_reservation_survives_restart_and_wipe_cannot_destroy_pending_home() {
        let temp = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = root.for_account(&id).unwrap();
        let package = package();
        let home = ready(&store, &package);
        let (directory, mut profile) = store
            .read_native_profile_metadata("claude-code", 1)
            .unwrap();
        profile.logout = Some(logout::Record {
            process_tree_exited: false,
            request: "logout-1".into(),
            succeeded: None,
            process: None,
        });
        atomic_write(
            &directory.join("profile.json"),
            &serde_json::to_vec(&profile).unwrap(),
            0o600,
        )
        .unwrap();
        drop(store);
        drop(root);
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let store = root.for_account(&id).unwrap();
        assert!(matches!(
            store
                .prepare_native_logout("claude-code", 1, "login-1", "logout-1")
                .unwrap(),
            LogoutPreparation::Observed(None)
        ));
        assert!(
            store
                .prepare_native_logout("claude-code", 1, "login-1", "other")
                .is_err()
        );
        assert!(store.native_profile_home(&package, 1).is_err());
        assert!(store.revoke_native_profiles("claude-code").is_err());
        assert!(home.exists());
        assert!(
            store
                .record_native_logout_exit("claude-code", 1, "other", true)
                .is_err()
        );
        // The wipe fence blocks reuse, but must not block the original outcome.
        store
            .record_native_logout_exit("claude-code", 1, "logout-1", true)
            .unwrap();
        store
            .record_native_logout_exit("claude-code", 1, "logout-1", true)
            .unwrap();
        assert!(
            store
                .record_native_logout_exit("claude-code", 1, "logout-1", false)
                .is_err()
        );
        store.revoke_native_profiles("claude-code").unwrap();
        assert!(!home.exists());
        assert!(directory.join("profile.json").exists());
        assert!(matches!(
            store
                .prepare_native_logout("claude-code", 1, "login-1", "logout-1")
                .unwrap(),
            LogoutPreparation::Observed(Some(true))
        ));
        store
            .record_native_logout_exit("claude-code", 1, "logout-1", true)
            .unwrap();
    }

    #[test]
    fn exit_evidence_is_scoped_durable_and_required_for_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let path = home.parent().unwrap().join("profile.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("leader_exited");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
        assert!(scoped.native_profile_home(&package, 1).is_err());
        let before = fs::read(&path).unwrap();
        assert!(
            scoped
                .record_native_login_exit("claude-code", 1, "other", true)
                .is_err()
        );
        assert!(
            store
                .record_native_login_exit("claude-code", 1, "login-1", true)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        scoped
            .record_native_login_tree_exit("claude-code", 1, "login-1", true)
            .unwrap();
        let persisted = fs::read(&path).unwrap();
        scoped
            .record_native_login_tree_exit("claude-code", 1, "login-1", true)
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), persisted);
        assert!(
            scoped
                .record_native_login_exit("claude-code", 1, "login-1", false)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), persisted);
        drop(scoped);
        drop(store);
        let reopened = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = reopened.for_account(&id).unwrap();
        scoped
            .native_profile_receipt("claude-code", 1, "login-1")
            .unwrap();
        scoped.native_profile_home(&package, 1).unwrap();
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(
            scoped
                .record_native_login_exit("claude-code", 1, "login-1", true)
                .is_err()
        );
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
    }

    #[test]
    fn observation_never_creates_missing_account_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = store.auth_base.join("accounts").join(id.as_str());
        assert!(
            store
                .observe_native_profile(&id, "claude-code", 1, "missing")
                .is_err()
        );
        assert!(!account.exists());
        let scoped = store.for_account(&id).unwrap();
        ready(&scoped, &package());
        let before =
            fs::read(account.join("providers/claude-code/native/generations/1/profile.json"))
                .unwrap();
        assert!(
            store
                .observe_native_profile(&id, "claude-code", 1, "login-1")
                .is_ok()
        );
        assert!(
            store
                .observe_native_profile(&id, "claude-code", 1, "wrong")
                .is_err()
        );
        assert_eq!(
            before,
            fs::read(account.join("providers/claude-code/native/generations/1/profile.json"))
                .unwrap()
        );
    }

    #[test]
    fn failed_login_exit_cannot_be_upgraded_to_success() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let path = home.parent().unwrap().join("profile.json");
        let mut profile: Profile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        profile.leader_exited = false;
        profile.login_succeeded = false;
        fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
        scoped
            .record_native_login_exit("claude-code", 1, "login-1", false)
            .unwrap();
        scoped
            .record_native_login_exit("claude-code", 1, "login-1", false)
            .unwrap();
        assert!(
            scoped
                .record_native_login_exit("claude-code", 1, "login-1", true)
                .is_err()
        );
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
        assert!(scoped.native_profile_home(&package, 1).is_err());
    }

    #[test]
    fn readmission_releases_only_new_binding_and_cannot_replay_after_stop() {
        use crate::provider_account::{
            PreparedBinding, WorkerExitObservation, WorkerExitRequest, account_stop_command_id,
        };
        use sha2::{Digest as _, Sha256};
        let temp = tempfile::tempdir().unwrap();
        let open = || {
            MachinePluginStore::new(
                temp.path(),
                crate::machine_protocol::Platform::Linux,
                "x86_64".into(),
            )
            .unwrap()
        };
        let store = open();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let directory = home.parent().unwrap();
        let path = directory.join("profile.json");
        let mut profile: Profile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        profile.generation = 2;
        fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
        fs::rename(directory, directory.parent().unwrap().join("2")).unwrap();
        let old: crate::runtime_wire::StartSession = serde_json::from_value(serde_json::json!({
            "session_id":"session", "provider":"claude-code", "provider_account_id":id,
            "provider_version":package.manifest.version, "provider_generation_digest":"fixture",
            "provider_auth_generation":1, "cwd":"/fixture", "generation":"worker"
        }))
        .unwrap();
        let launch = store.begin_bound_account_worker(&old).unwrap();
        let command = account_stop_command_id("migration:move", "session", &id);
        let binding = PreparedBinding {
            operation: "move".into(),
            stopped: WorkerExitObservation {
                request: WorkerExitRequest {
                    account_id: id.clone(),
                    session_id: "session".into(),
                    stop_command_id: Some(command.clone()),
                },
                launch_id: launch,
            },
            target: scoped
                .native_profile_receipt("claude-code", 2, "login-1")
                .unwrap(),
        };
        store
            .stop_account_session_scoped("session", &id, &command)
            .unwrap();
        assert!(store.readmit_account_binding(&binding, &package).is_err());
        store
            .record_account_worker_exit("session", &id, launch)
            .unwrap();
        store.readmit_account_binding(&binding, &package).unwrap();
        assert!(!store.account_session_stopped("session").unwrap());
        assert!(store.account_stops().unwrap().is_empty());
        assert!(store.begin_bound_account_worker(&old).is_err());
        // Reconstruct a crash after the new worker binding was persisted but
        // before the stop marker was released. Only this operation can resume.
        let marker = store
            .auth_base
            .join("session-account-stops")
            .join(format!("{:x}", Sha256::digest(b"session")));
        fs::write(
            &marker,
            serde_json::to_vec(&(2u32, "session", &id, &command)).unwrap(),
        )
        .unwrap();
        assert!(store.account_session_stopped("session").unwrap());
        drop(scoped);
        drop(store);
        let store = open();
        store.readmit_account_binding(&binding, &package).unwrap();
        let mut wrong = binding.clone();
        wrong.stopped.launch_id = uuid::Uuid::new_v4();
        assert!(store.readmit_account_binding(&wrong, &package).is_err());
        let mut next = old.clone();
        next.provider_auth_generation = Some(2);
        let next_launch = store.begin_bound_account_worker(&next).unwrap();
        assert_ne!(launch, next_launch);
        store
            .record_account_worker_exit("session", &id, next_launch)
            .unwrap();
        store
            .stop_account_session_scoped("session", &id, &command)
            .unwrap();
        assert!(store.account_session_stopped("session").unwrap());
        assert!(store.readmit_account_binding(&binding, &package).is_err());
        assert!(store.begin_bound_account_worker(&next).is_err());
    }

    #[test]
    fn prepared_binding_revalidates_stop_worker_and_native_identity() {
        use crate::provider_account::PreparedBinding;
        use crate::provider_account::{
            WorkerExitObservation, WorkerExitRequest, account_stop_command_id,
        };
        let temp = tempfile::tempdir().unwrap();
        let open = || {
            MachinePluginStore::new(
                temp.path(),
                crate::machine_protocol::Platform::Linux,
                "x86_64".into(),
            )
            .unwrap()
        };
        let store = open();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        ready(&scoped, &package);
        let launch = store.begin_account_worker("session", &id).unwrap();
        let command = account_stop_command_id("migration:move-1", "session", &id);
        let binding = PreparedBinding {
            operation: "move-1".into(),
            stopped: WorkerExitObservation {
                request: WorkerExitRequest {
                    account_id: id.clone(),
                    session_id: "session".into(),
                    stop_command_id: Some(command.clone()),
                },
                launch_id: launch,
            },
            target: scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .unwrap(),
        };
        assert!(store.prepare_account_binding(&binding, &package).is_err());
        store
            .stop_account_session_scoped("session", &id, &command)
            .unwrap();
        assert!(store.prepare_account_binding(&binding, &package).is_err());
        store
            .record_account_worker_exit("session", &id, launch)
            .unwrap();
        let mut wrong = binding.clone();
        wrong.stopped.launch_id = uuid::Uuid::new_v4();
        assert!(store.prepare_account_binding(&wrong, &package).is_err());
        wrong = binding.clone();
        wrong.target.identity.subject = "another-person".into();
        assert!(store.prepare_account_binding(&wrong, &package).is_err());
        store.prepare_account_binding(&binding, &package).unwrap();
        store.prepare_account_binding(&binding, &package).unwrap();
        assert!(store.begin_account_worker("session", &id).is_err());
        drop(scoped);
        drop(store);
        let store = open();
        store.prepare_account_binding(&binding, &package).unwrap();
        assert!(store.account_session_stopped("session").unwrap());
        store
            .stop_account_session_scoped("session", &id, "later-stop")
            .unwrap();
        assert!(store.prepare_account_binding(&binding, &package).is_err());
        store
            .stop_account_session_scoped("session", &id, &command)
            .unwrap();
        store
            .for_account(&id)
            .unwrap()
            .revoke_native_profiles("claude-code")
            .unwrap();
        assert!(store.prepare_account_binding(&binding, &package).is_err());
    }

    #[test]
    fn completion_receipt_survives_restart_without_opening_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        // Opening this as a credential would block. Receipt recovery only
        // reads the metadata and identity files, even after process restart.
        for credential in &package.manifest.authentication.credential_files {
            let path = home.join(&credential.relative_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            crate::test_fifo(path).unwrap();
        }
        let receipt = scoped
            .native_profile_receipt("claude-code", 1, "login-1")
            .unwrap();
        assert_eq!(receipt.account_id, id);
        assert_eq!(receipt.request_id, "login-1");
        assert_eq!(receipt.generation, 1);
        assert_eq!(receipt.provider_version, package.manifest.version);
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "different")
                .is_err()
        );
        assert!(
            scoped
                .native_profile_receipt("claude-code", 2, "login-1")
                .is_err()
        );
        assert!(
            store
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
        let mut wire = serde_json::to_value(&receipt).unwrap();
        wire["bundle"] = serde_json::json!({"token":"must not enter receipt"});
        assert!(serde_json::from_value::<NativeProfileReceipt>(wire).is_err());
        drop(scoped);
        drop(store);
        let reopened = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = reopened.for_account(&id).unwrap();
        assert_eq!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .unwrap(),
            receipt
        );
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(
            scoped
                .native_profile_receipt("claude-code", 1, "login-1")
                .is_err()
        );
    }

    #[test]
    fn wipe_rejects_recreated_homes_and_preserves_other_accounts_and_operation_fences() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let scoped = store.for_account(&id).unwrap();
        let other = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let other_home = ready(&other, &package);
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep"), b"untouched").unwrap();
        symlink(&external, home.join("link")).unwrap();
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(!home.exists());
        assert!(other_home.exists());
        assert_eq!(fs::read(external.join("keep")).unwrap(), b"untouched");
        let root = scoped.native_profile_root("claude-code");
        assert!(root.join("identity.json").is_file());
        assert!(validate_reservation(&root, "login-1").is_err());
        assert!(validate_reservation(&root, "new-request").is_ok());
        // Even a surviving process recreating its home cannot undo revocation.
        fs::create_dir(&home).unwrap();
        assert!(scoped.native_profile_home(&package, 1).is_err());
        scoped.revoke_native_profiles("claude-code").unwrap();
        assert!(!home.exists());
        assert!(other.native_profile_home(&package, 1).is_ok());
        assert!(
            store
                .ensure_no_named_auth_materializations("claude-code")
                .is_err()
        );
        other.revoke_native_profiles("claude-code").unwrap();
        store
            .ensure_no_named_auth_materializations("claude-code")
            .unwrap();
        // A recreated home, including a dangling link, fences uninstall again.
        symlink("missing", &home).unwrap();
        assert!(
            store
                .ensure_no_named_auth_materializations("claude-code")
                .is_err()
        );
        scoped.revoke_native_profiles("claude-code").unwrap();
        store
            .ensure_no_named_auth_materializations("claude-code")
            .unwrap();
        drop(other);
        drop(scoped);
        drop(store);
        let reopened = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = reopened.for_account(&id).unwrap();
        assert!(scoped.native_profile_home(&package, 1).is_err());
        assert!(
            validate_reservation(&scoped.native_profile_root("claude-code"), "login-1").is_err()
        );
        scoped.revoke_native_profiles("claude-code").unwrap();
    }

    #[tokio::test]
    async fn only_authenticated_scoped_wipes_revoke_native_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        let signer =
            crate::machine_auth::MachineIdentity::load_or_create(&temp.path().join("signer"))
                .unwrap();
        let mut envelope = SealedProviderAuth {
            envelope_schema: 2,
            account_id: scoped.account_id.clone(),
            provider_id: "claude-code".into(),
            auth_generation: 1,
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            projection_schema: package.manifest.authentication.projection_schema.clone(),
            action: ProviderAuthAction::Wipe,
            ephemeral_public_key: String::new(),
            nonce: String::new(),
            ciphertext: String::new(),
            service_public_key: signer.public_key().to_owned(),
            signature: String::new(),
        };
        assert!(store.apply_auth(&envelope).await.is_err());
        assert!(home.exists());
        envelope.signature = signer
            .sign_namespaced(
                crate::machine_auth::PROVIDER_AUTH_SIGNATURE_NAMESPACE,
                &envelope.proof(),
            )
            .unwrap();
        // A valid signature alone cannot establish trust for a named account.
        assert!(store.apply_auth(&envelope).await.is_err());
        assert!(home.exists());
        store.pin_service_key(signer.public_key()).unwrap();
        let observation = scoped.native_observation_guard().await;
        let mut wipe = std::pin::pin!(store.apply_auth(&envelope));
        assert!(futures::poll!(&mut wipe).is_pending());
        assert!(
            home.exists(),
            "an active quota process still owns this home"
        );
        assert!(
            store.lifecycle.try_lock().is_ok(),
            "waiting for a native process must not block other Providers"
        );
        drop(observation);
        let receipt = wipe.await.unwrap();
        assert_eq!(
            receipt.materialization_state,
            ProviderMaterializationState::NotInstalled
        );
        assert!(!home.exists());
        assert!(scoped.native_profile_home(&package, 1).is_err());
        // Identical retry must finish cleanup rather than recreate a home.
        store.apply_auth(&envelope).await.unwrap();
        assert!(!home.exists());
    }

    #[test]
    fn native_runtime_never_needs_a_portable_replica_or_reads_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let package = package();
        let home = ready(&scoped, &package);
        assert!(
            store
                .ensure_no_named_auth_materializations("claude-code")
                .is_err()
        );
        let root = scoped.native_profile_root("claude-code");
        assert!(validate_reservation(&root, "login-1").is_err());
        assert!(validate_reservation(&root, "login-2").is_ok());
        let metadata = root.join("generations/1/profile.json");
        let original = fs::read(&metadata).unwrap();
        let mut pending: Profile = serde_json::from_slice(&original).unwrap();
        pending.identity = None;
        fs::write(&metadata, serde_json::to_vec(&pending).unwrap()).unwrap();
        assert!(validate_reservation(&root, "login-2").is_err());
        assert!(scoped.native_profile_home(&package, 1).is_err());
        fs::write(&metadata, original).unwrap();
        fs::create_dir(root.join("generations/2")).unwrap();
        assert!(validate_reservation(&root, "login-2").is_err());
        fs::remove_dir(root.join("generations/2")).unwrap();
        // No sealed envelope exists. Even a special credential file must never
        // be opened by launch admission; only the native executable owns it.
        for credential in &package.manifest.authentication.credential_files {
            let path = home.join(&credential.relative_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            crate::test_fifo(path).unwrap();
        }
        assert_eq!(
            scoped
                .prepare_launch_auth_home("claude-code", &package, 1, "fixture")
                .unwrap(),
            home
        );
        assert!(
            !scoped
                .credential_directories(&package, Some(&home))
                .unwrap()
                .is_empty()
        );
        assert!(
            scoped
                .prepare_launch_auth_home("claude-code", &package, 2, "fixture")
                .is_err()
        );
        assert!(
            store
                .prepare_launch_auth_home("claude-code", &package, 1, "fixture")
                .is_err()
        );
        let another = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        assert!(
            another
                .prepare_launch_auth_home("claude-code", &package, 1, "fixture")
                .is_err()
        );
    }

    #[test]
    fn missing_linked_and_mismatched_native_profiles_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let scoped = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let mut package = package();
        let home = ready(&scoped, &package);
        package
            .manifest
            .compatibility
            .auth_contract_fingerprint
            .push('x');
        assert!(scoped.native_profile_home(&package, 1).is_err());
        package
            .manifest
            .compatibility
            .auth_contract_fingerprint
            .pop();
        fs::remove_dir(&home).unwrap();
        assert!(scoped.native_profile_home(&package, 1).is_err());
        assert!(!home.exists());
        symlink(temp.path(), &home).unwrap();
        assert!(scoped.native_profile_home(&package, 1).is_err());
        fs::remove_file(&home).unwrap();
        fs::create_dir(&home).unwrap();
        fs::write(
            scoped
                .native_profile_root("claude-code")
                .join("identity.json"),
            br#"{"authority":"fixture","subject":"another","tenant":null}"#,
        )
        .unwrap();
        assert!(scoped.native_profile_home(&package, 1).is_err());
    }
}
