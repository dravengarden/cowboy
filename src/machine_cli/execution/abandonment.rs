//! Closing a pending session must fence even a delayed preparation request.
use super::*;
use crate::execution_environment::PreparationV1;

impl Manager {
    pub(super) async fn abandon(
        &self,
        session_id: &str,
        preparation: &PreparationV1,
    ) -> Result<(), Refusal> {
        if !keeper::valid_operation_id(session_id)
            || preparation.validate().is_err()
            || preparation.machine_id != self.machine_id
        {
            return Err(Refusal::InvalidRequest);
        }
        let _gate = self.prepare.lock().await;
        let directory = self.root.join(session_id);
        // A persisted contract is written before spawning any process. An
        // unreadable contract is uncertainty, never evidence of an absent job.
        let contract = match read_contract(&directory.join("contract.json")) {
            Ok(contract) => {
                if contract.session_id != session_id || !preparation.accepts(&contract.binding) {
                    return Err(Refusal::IdentityMismatch);
                }
                Some(contract)
            }
            Err(_) if !directory.exists() => None,
            Err(_) => return Err(Refusal::EnvironmentLost),
        };
        let abandoned = self.root.join("abandoned");
        std::fs::create_dir_all(&abandoned).map_err(|_| Refusal::Unavailable)?;
        std::fs::set_permissions(&abandoned, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Refusal::Unavailable)?;
        let path = abandoned.join(session_id);
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => {
                serde_json::to_writer(&file, preparation).map_err(|_| Refusal::Unavailable)?;
                file.sync_all().map_err(|_| Refusal::Unavailable)?;
                for parent in [&abandoned, &self.root] {
                    std::fs::File::open(parent)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| Refusal::Unavailable)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata =
                    std::fs::symlink_metadata(&path).map_err(|_| Refusal::Unavailable)?;
                if !metadata.is_file()
                    || metadata.len() > 8192
                    || metadata.mode() & 0o077 != 0
                    || metadata.uid() != rustix::process::geteuid().as_raw()
                {
                    return Err(Refusal::IdentityMismatch);
                }
                let retained: PreparationV1 = serde_json::from_slice(
                    &std::fs::read(&path).map_err(|_| Refusal::Unavailable)?,
                )
                .map_err(|_| Refusal::IdentityMismatch)?;
                if &retained != preparation {
                    return Err(Refusal::IdentityMismatch);
                }
            }
            Err(_) => return Err(Refusal::Unavailable),
        }
        if let Some(contract) = contract {
            self.close_locked(session_id, &contract.binding).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bound_runtime_entry_requires_original_private_service_and_session_marker() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(
            Some(format!("svc-{}", "1".repeat(32))),
            "runtime".into(),
            root.path(),
            None,
            false,
        )
        .unwrap();
        let runtime = manager.prepare_runtime("session").await.unwrap();
        let mut binding = crate::execution_environment::fixture().record().clone();
        binding["runtime"] = serde_json::to_value(&runtime).unwrap();
        let session: crate::runtime_wire::StartSession =
            serde_json::from_value(serde_json::json!({
                "session_id":"session", "provider":"fixture", "generation":"fixture",
                "cwd":runtime.cwd, "execution_binding":binding,
            }))
            .unwrap();
        assert!(manager.owns_runtime_entry(&session));
        let other_service = Manager::new(
            Some(format!("svc-{}", "2".repeat(32))),
            "runtime".into(),
            root.path(),
            None,
            false,
        )
        .unwrap();
        assert!(!other_service.owns_runtime_entry(&session));
        let mut other_session = session.clone();
        other_session.session_id = "other".into();
        assert!(!manager.owns_runtime_entry(&other_session));
        other_session = session.clone();
        other_session.cwd = root.path().display().to_string();
        assert!(!manager.owns_runtime_entry(&other_session));
        let marker = Path::new(&session.cwd).join("entry.json");
        std::fs::remove_file(&marker).unwrap();
        assert!(!manager.owns_runtime_entry(&session));
        assert!(!marker.exists(), "validation must never repair the entry");
    }

    #[tokio::test]
    async fn abandoned_preparation_survives_restart_and_refuses_changed_identity() {
        let root = tempfile::tempdir().unwrap();
        let binding = crate::execution_environment::fixture().decode().unwrap();
        let intent = PreparationV1 {
            schema: 1,
            phase: "preparing".into(),
            runtime: binding.runtime.clone(),
            machine_id: binding.environment.machine_id.clone(),
            workspace_id: binding.workspace.id.clone(),
            source_path: binding.workspace.source_path.clone(),
            executor_digest: binding.environment.executor_digest,
        };
        let manager = || {
            Manager::new(
                Some(format!("svc-{}", "1".repeat(32))),
                intent.machine_id.clone(),
                root.path(),
                None,
                false,
            )
            .unwrap()
        };
        manager().abandon("session", &intent).await.unwrap();
        let reopened = manager();
        reopened.abandon("session", &intent).await.unwrap();
        assert!(reopened.root.join("abandoned/session").is_file());
        assert!(!reopened.root.join("session").exists());
        let mut changed = intent.clone();
        changed.runtime.cwd = "/different".into();
        assert_eq!(
            reopened.abandon("session", &changed).await,
            Err(Refusal::IdentityMismatch)
        );
        // An interrupted target may contain work or an unreadable contract.
        // Preserve it instead of claiming every process has stopped.
        std::fs::create_dir(reopened.root.join("uncertain")).unwrap();
        assert_eq!(
            reopened.abandon("uncertain", &intent).await,
            Err(Refusal::EnvironmentLost)
        );
    }
}
