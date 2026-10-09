//! Explicit process-lifetime replacement. The original state is archived,
//! never reused as a fresh incarnation. A saved launch intent fences unknown
//! starts, and only an exact repeated maintenance identity can observe progress.

use super::*;
use crate::execution_environment::RecoveryV1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryRecord {
    schema: u16,
    intent: RecoveryV1,
    next: LaunchContract,
}

fn directory(path: &Path) -> Result<(), Refusal> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => sync(path.parent().ok_or(Refusal::InvalidRequest)?)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(Refusal::Unavailable),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| Refusal::Unavailable)?;
    if !metadata.is_dir()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(Refusal::IdentityMismatch);
    }
    Ok(())
}

fn sync(path: &Path) -> Result<(), Refusal> {
    std::fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| Refusal::Unavailable)
}

fn write(path: &Path, value: &impl Serialize) -> Result<(), Refusal> {
    let parent = path.parent().ok_or(Refusal::InvalidRequest)?;
    let temporary = parent.join(format!(".recovery-{}.tmp", random_id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|_| Refusal::Unavailable)?;
    let result = (|| {
        serde_json::to_writer(&file, value).map_err(|_| Refusal::Unavailable)?;
        file.sync_all().map_err(|_| Refusal::Unavailable)?;
        // Publish complete bytes without replacing an existing transaction.
        std::fs::hard_link(&temporary, path).map_err(|_| Refusal::Unavailable)?;
        sync(parent)
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

impl Manager {
    pub(super) async fn recover(
        &self,
        session_id: &str,
        intent: &RecoveryV1,
    ) -> Result<BindingV1, Refusal> {
        intent.validate().map_err(|_| Refusal::InvalidRequest)?;
        if !keeper::valid_operation_id(session_id)
            || intent.previous.workspace.worktree_id != session_id
            || intent.previous.environment.machine_id != self.machine_id
        {
            return Err(Refusal::IdentityMismatch);
        }
        let config = self.configuration.as_ref().ok_or(Refusal::Unavailable)?;
        // This operation upgrades the keeper, not the signed native executor.
        if format!("sha256:{}", config.executor.sha256)
            != intent.previous.environment.executor_digest
        {
            return Err(Refusal::IdentityMismatch);
        }
        let _gate = self.prepare.lock().await;
        let active = self.root.join(session_id);
        let history = self.root.join("recoveries");
        directory(&history)?;
        let history = history.join(session_id);
        directory(&history)?;
        let transaction = history.join(&intent.operation_id);
        directory(&transaction)?;
        let record_path = transaction.join("intent.json");
        let record: RecoveryRecord = if record_path.exists() {
            let bytes = crate::logs::storage::private_read(&record_path, 128 * 1024)
                .map_err(|_| Refusal::IdentityMismatch)?;
            serde_json::from_slice(&bytes).map_err(|_| Refusal::IdentityMismatch)?
        } else {
            // Refuse a different operation after the first transaction moved
            // the original directory, even if its reply never reached Service.
            let old = read_contract(&active.join("contract.json"))
                .map_err(|_| Refusal::EnvironmentLost)?;
            if old.session_id != session_id
                || old.binding != intent.previous
                || active.join("closed.json").exists()
                || active.join("stopped.json").exists()
            {
                return Err(Refusal::IdentityMismatch);
            }
            let mut next = old;
            next.binding.revision += 1;
            next.binding.environment.id = random_id();
            next.binding.environment.incarnation = random_id();
            next.capability = format!("{}{}", random_id(), random_id());
            let record = RecoveryRecord {
                schema: 1,
                intent: intent.clone(),
                next,
            };
            write(&record_path, &record)?;
            record
        };
        if record.schema != 1
            || record.intent != *intent
            || record.next.session_id != session_id
            || !intent.accepts(&record.next.binding)
        {
            return Err(Refusal::IdentityMismatch);
        }
        let archive = transaction.join("previous");
        if !archive.exists() {
            let old = read_contract(&active.join("contract.json"))
                .map_err(|_| Refusal::EnvironmentLost)?;
            if old.binding != intent.previous {
                return Err(Refusal::IdentityMismatch);
            }
            // The existing close fences requests before stopping the exact
            // systemd unit and all of its target command descendants.
            self.close_locked(session_id, &intent.previous).await?;
            std::fs::rename(&active, &archive).map_err(|_| Refusal::Unavailable)?;
            sync(&self.root)?;
            sync(&transaction)?;
        }
        validate_close_marker(&archive.join("stopped.json"), &intent.previous)?;
        if !active.exists() {
            let staged = transaction.join("next");
            directory(&staged)?;
            let contract_path = staged.join("contract.json");
            if !contract_path.exists() {
                write(&contract_path, &record.next)?;
            }
            let saved = read_contract(&contract_path).map_err(|_| Refusal::IdentityMismatch)?;
            if saved.binding != record.next.binding || saved.capability != record.next.capability {
                return Err(Refusal::IdentityMismatch);
            }
            std::fs::rename(&staged, &active).map_err(|_| Refusal::Unavailable)?;
            sync(&self.root)?;
            sync(&transaction)?;
        }
        let current =
            read_contract(&active.join("contract.json")).map_err(|_| Refusal::IdentityMismatch)?;
        if current.binding != record.next.binding
            || current.capability != record.next.capability
            || active.join("closed.json").exists()
        {
            return Err(Refusal::IdentityMismatch);
        }
        if matches!(exchange(&active, &current, keeper::Command::Describe).await,
            Ok(keeper::Response::Ready { scope, .. }) if scope == Scope::from_binding(&current.binding))
        {
            return Ok(current.binding);
        }
        if active.join("launch-requested.json").exists() || active.join("started.json").exists() {
            return Err(Refusal::EnvironmentLost);
        }
        if let Some(retention) = &config.retention {
            let status = tokio::time::timeout(
                Duration::from_secs(20),
                tokio::process::Command::new(&retention.command)
                    .arg("--add-root")
                    .arg(active.join("executor-root"))
                    .arg("--indirect")
                    .arg("--realise")
                    .arg(&retention.closure)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .status(),
            )
            .await;
            if !matches!(status, Ok(Ok(status)) if status.success()) {
                return Err(Refusal::PreparationFailed);
            }
        }
        write(
            &active.join("launch-requested.json"),
            &Scope::from_binding(&current.binding),
        )?;
        self.spawn(
            config,
            &active.join("contract.json"),
            &active,
            &current.binding,
        )
        .await
        .map_err(|_| Refusal::PreparationFailed)?;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if matches!(exchange(&active, &current, keeper::Command::Describe).await,
                    Ok(keeper::Response::Ready { scope, .. }) if scope == Scope::from_binding(&current.binding))
                { return current.binding.clone(); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.map_err(|_| Refusal::EnvironmentLost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_record_publication_is_complete_and_exclusive() {
        struct Interrupted;
        impl Serialize for Interrupted {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut sequence = serializer.serialize_seq(None)?;
                sequence.serialize_element("partial")?;
                Err(serde::ser::Error::custom("interrupted write"))
            }
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("intent.json");
        assert!(write(&path, &Interrupted).is_err());
        assert!(!path.exists());
        write(&path, &serde_json::json!({"complete": true})).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&original).unwrap(),
            serde_json::json!({"complete": true})
        );
        assert!(write(&path, &serde_json::json!({"replacement": true})).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
