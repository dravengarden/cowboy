//! A new explicit login may retire a conclusively failed, unbound generation.
//! Unknown exits and successful native logins still require reconciliation.
use super::*;

impl MachinePluginStore {
    /// Caller holds `auth_candidates` and is admitting a fresh explicit login.
    pub(super) fn retire_failed_native_logins(&self, provider: &str, request: &str) -> Result<()> {
        let root = self.native_profile_root(provider);
        ensure_no_revocation(&root)?;
        let mut failed = Vec::new();
        // Validate the complete inventory before changing anything. Never let a
        // replay of an old request clean up another pending generation.
        for entry in fs::read_dir(root.join("generations"))? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            let generation = name.parse::<u64>().context("invalid native generation")?;
            ensure!(
                generation > 0 && generation.to_string() == name,
                "invalid native generation"
            );
            let (directory, profile) =
                self.read_native_profile_for_cleanup(provider, generation)?;
            ensure!(
                profile.request != request,
                "native login operation already reserved"
            );
            if profile.identity.is_none()
                && !profile.login_succeeded
                && profile.leader_exited
                && profile.process_tree_exited
                && profile.logout.is_none()
            {
                failed.push((directory, profile));
            }
        }
        for (directory, mut profile) in failed {
            // Persist the fence before removing any private bytes. Retrying a
            // partially deleted failed generation finishes the same cleanup.
            profile.revoked = true;
            logout::persist(&directory, &profile)?;
            remove_projection::remove(&directory.join("home"))?;
            fs::File::open(&directory)?.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_login_cleanup_requires_tree_exit_and_keeps_the_replay_fence() {
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
        let home = super::super::tests::ready(&scoped, &super::super::tests::package());
        let root = scoped.native_profile_root("claude-code");
        let path = home.parent().unwrap().join("profile.json");
        let original: Profile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut failed = original.clone();
        failed.identity = None;
        failed.login_succeeded = false;
        // Each unresolved or successful variant must retain its home and bytes.
        for case in 0..4 {
            let mut pending = failed.clone();
            match case {
                0 => pending.process_tree_exited = false,
                1 => pending.leader_exited = false,
                2 => pending.login_succeeded = true,
                _ => pending.identity = original.identity.clone(),
            }
            let bytes = serde_json::to_vec(&pending).unwrap();
            fs::write(&path, &bytes).unwrap();
            scoped
                .retire_failed_native_logins("claude-code", "new-login")
                .unwrap();
            assert!(home.is_dir());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::write(&path, serde_json::to_vec(&failed).unwrap()).unwrap();
        assert!(
            scoped
                .retire_failed_native_logins("claude-code", "login-1")
                .is_err()
        );
        assert!(home.is_dir());
        assert!(validate_reservation(&root, "new-login").is_err());
        scoped
            .retire_failed_native_logins("claude-code", "new-login")
            .unwrap();
        assert!(!home.exists());
        assert!(path.is_file());
        assert!(validate_reservation(&root, "new-login").is_ok());
        assert!(validate_reservation(&root, "login-1").is_err());
        // Reconstruct interrupted deletion after the durable tombstone.
        fs::create_dir(&home).unwrap();
        fs::write(home.join("partial"), b"fixture").unwrap();
        scoped
            .retire_failed_native_logins("claude-code", "next-login")
            .unwrap();
        assert!(!home.exists());
    }

    #[test]
    fn failed_login_cleanup_refuses_foreign_metadata_and_account_revocation() {
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
        let home = super::super::tests::ready(&scoped, &super::super::tests::package());
        let root = scoped.native_profile_root("claude-code");
        let path = home.parent().unwrap().join("profile.json");
        let mut profile: Profile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        profile.identity = None;
        profile.login_succeeded = false;
        let original = profile.clone();
        for case in 0..3 {
            profile = original.clone();
            match case {
                0 => profile.account = uuid::Uuid::new_v4().to_string().try_into().unwrap(),
                1 => profile.provider = "other".into(),
                _ => profile.generation = 2,
            }
            fs::write(&path, serde_json::to_vec(&profile).unwrap()).unwrap();
            assert!(
                scoped
                    .retire_failed_native_logins("claude-code", "new-login")
                    .is_err()
            );
            assert!(home.is_dir());
        }
        fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        fs::write(root.join("revoking"), b"1").unwrap();
        assert!(
            scoped
                .retire_failed_native_logins("claude-code", "new-login")
                .is_err()
        );
        assert!(home.is_dir());
    }
}
