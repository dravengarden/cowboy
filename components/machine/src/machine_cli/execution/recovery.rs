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
        let mut record: RecoveryRecord = if record_path.exists() {
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
            // A replacement keeper carries the session's call context even
            // when the environment it replaces predates managed calls.
            if next.managed.is_none() {
                next.call_context = self
                    .calls
                    .as_ref()
                    .and_then(|calls| calls.context_path(session_id))
                    .map(|path| path.display().to_string());
            }
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
        let mut previous = intent.previous.clone();
        let mut attempt = 0usize;
        // Each successor is published before moving its predecessor. Replaying
        // the same operation resumes the last published transition, including
        // a crash between archiving the old lifetime and installing the new one.
        loop {
            let path = transaction.join(format!("attempt-{}.json", attempt + 1));
            if !path.exists() {
                break;
            }
            let bytes = crate::logs::storage::private_read(&path, 128 * 1024)
                .map_err(|_| Refusal::IdentityMismatch)?;
            let next: RecoveryRecord =
                serde_json::from_slice(&bytes).map_err(|_| Refusal::IdentityMismatch)?;
            validate_successor(&record, &next)?;
            previous = record.next.binding.clone();
            record = next;
            attempt += 1;
        }
        loop {
            let archive = if attempt == 0 {
                transaction.join("previous")
            } else {
                transaction.join(format!("attempt-{}-previous", attempt))
            };
            if !archive.exists() {
                let old = read_contract(&active.join("contract.json"))
                    .map_err(|_| Refusal::EnvironmentLost)?;
                if old.binding != previous {
                    return Err(Refusal::IdentityMismatch);
                }
                // The existing close fences requests before stopping the exact
                // systemd unit and all of its target command descendants.
                self.close_locked(session_id, &previous).await?;
                std::fs::rename(&active, &archive).map_err(|_| Refusal::Unavailable)?;
                sync(&self.root)?;
                sync(&transaction)?;
            }
            validate_close_marker(&archive.join("stopped.json"), &previous)?;
            if !active.exists() {
                let staged = transaction.join(if attempt == 0 {
                    "next".into()
                } else {
                    format!("next-{attempt}")
                });
                directory(&staged)?;
                let contract_path = staged.join("contract.json");
                if !contract_path.exists() {
                    write(&contract_path, &record.next)?;
                }
                let saved = read_contract(&contract_path).map_err(|_| Refusal::IdentityMismatch)?;
                if saved.binding != record.next.binding
                    || saved.capability != record.next.capability
                {
                    return Err(Refusal::IdentityMismatch);
                }
                std::fs::rename(&staged, &active).map_err(|_| Refusal::Unavailable)?;
                sync(&self.root)?;
                sync(&transaction)?;
            }
            let current = read_contract(&active.join("contract.json"))
                .map_err(|_| Refusal::IdentityMismatch)?;
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
            if active.join("launch-requested.json").exists() || active.join("started.json").exists()
            {
                // A broken socket alone cannot authorize replacement. A changed
                // host boot proves that this attempted lifetime and its descendants
                // are gone; retain its journal and give the successor a new identity.
                if !liveness::boot_changed(&active) {
                    return Err(Refusal::EnvironmentLost);
                }
                let next = successor(&record);
                write(
                    &transaction.join(format!("attempt-{}.json", attempt + 1)),
                    &next,
                )?;
                previous = record.next.binding.clone();
                record = next;
                attempt += 1;
                continue;
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
            return tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if matches!(exchange(&active, &current, keeper::Command::Describe).await,
                    Ok(keeper::Response::Ready { scope, .. }) if scope == Scope::from_binding(&current.binding))
                { return current.binding.clone(); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.map_err(|_| Refusal::EnvironmentLost);
        }
    }
}

fn successor(record: &RecoveryRecord) -> RecoveryRecord {
    let mut next = record.next.clone();
    next.binding.environment.id = random_id();
    next.binding.environment.incarnation = random_id();
    next.capability = format!("{}{}", random_id(), random_id());
    RecoveryRecord {
        schema: 1,
        intent: record.intent.clone(),
        next,
    }
}

fn validate_successor(previous: &RecoveryRecord, next: &RecoveryRecord) -> Result<(), Refusal> {
    let mut expected = previous.next.clone();
    expected.binding.environment.id = next.next.binding.environment.id.clone();
    expected.binding.environment.incarnation = next.next.binding.environment.incarnation.clone();
    expected.capability = next.next.capability.clone();
    if next.schema != 1
        || next.intent != previous.intent
        || !next.intent.accepts(&next.next.binding)
        || next.next.binding.environment.id == previous.next.binding.environment.id
        || next.next.binding.environment.incarnation
            == previous.next.binding.environment.incarnation
        || next.next.capability == previous.next.capability
        || serde_json::to_value(&expected).ok() != serde_json::to_value(&next.next).ok()
    {
        return Err(Refusal::IdentityMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> (Manager, RecoveryRecord, PathBuf) {
        let executor = keeper::Executor {
            command: "/unused-executor".into(),
            sha256: "ab".repeat(32),
            version: "1.0.0".into(),
        };
        let manager = Manager::new(
            Some(format!("svc-{}", "1".repeat(32))),
            "hawk".into(),
            root,
            Some(Configuration {
                schema: 1,
                host_command: "/nonexistent-keeper".into(),
                executor: executor.clone(),
                retention: None,
            }),
            false,
        )
        .unwrap();
        let previous = crate::execution_environment::fixture().decode().unwrap();
        let intent = RecoveryV1 {
            schema: 1,
            phase: "recovering".into(),
            operation_id: "recover-test".into(),
            previous: previous.clone(),
        };
        let mut binding = previous;
        binding.revision += 1;
        binding.environment.id = random_id();
        binding.environment.incarnation = random_id();
        let record = RecoveryRecord {
            schema: 1,
            intent,
            next: LaunchContract {
                schema: 1,
                session_id: "worktree-one".into(),
                binding,
                executor,
                capability: "private-test".into(),
                environment: BTreeMap::new(),
                call_context: None,
                managed: None,
            },
        };
        let transaction = manager.root.join("recoveries/worktree-one/recover-test");
        std::fs::create_dir_all(transaction.join("previous")).unwrap();
        for path in [
            manager.root.join("recoveries"),
            manager.root.join("recoveries/worktree-one"),
            transaction.clone(),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        write(&transaction.join("intent.json"), &record).unwrap();
        write_close_marker(
            &transaction.join("previous"),
            "stopped.json",
            &record.intent.previous,
        )
        .unwrap();
        let active = manager.root.join("worktree-one");
        directory(&active).unwrap();
        write(&active.join("contract.json"), &record.next).unwrap();
        write(
            &active.join("launch-requested.json"),
            &Scope::from_binding(&record.next.binding),
        )
        .unwrap();
        liveness::record_boot(&active).unwrap();
        (manager, record, transaction)
    }

    #[tokio::test]
    async fn rebooted_recovery_retains_attempt_and_never_relaunches_uncertain_successor() {
        let root = tempfile::tempdir().unwrap();
        let (manager, record, transaction) = fixture(root.path());
        let active = manager.root.join("worktree-one");
        assert!(matches!(
            manager.recover("worktree-one", &record.intent).await,
            Err(Refusal::EnvironmentLost)
        ));
        assert!(!transaction.join("attempt-1.json").exists());
        std::fs::write(
            active.join("host-boot-id"),
            "00000000-0000-0000-0000-000000000000",
        )
        .unwrap();
        assert!(matches!(
            manager.recover("worktree-one", &record.intent).await,
            Err(Refusal::PreparationFailed)
        ));
        let next = read_contract(&active.join("contract.json")).unwrap();
        assert!(record.intent.accepts(&next.binding));
        assert_ne!(
            next.binding.environment.incarnation,
            record.next.binding.environment.incarnation
        );
        assert!(
            transaction
                .join("attempt-1-previous/launch-requested.json")
                .exists()
        );
        let saved = std::fs::read(transaction.join("attempt-1.json")).unwrap();
        assert!(matches!(
            manager.recover("worktree-one", &record.intent).await,
            Err(Refusal::EnvironmentLost)
        ));
        assert_eq!(
            saved,
            std::fs::read(transaction.join("attempt-1.json")).unwrap()
        );
        assert!(!transaction.join("attempt-2.json").exists());
    }

    #[tokio::test]
    async fn successor_resumes_after_publication_or_archive_without_reusing_identity() {
        for archived in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (manager, record, transaction) = fixture(root.path());
            let active = manager.root.join("worktree-one");
            std::fs::write(
                active.join("host-boot-id"),
                "00000000-0000-0000-0000-000000000000",
            )
            .unwrap();
            let next = successor(&record);
            write(&transaction.join("attempt-1.json"), &next).unwrap();
            if archived {
                manager
                    .close_locked("worktree-one", &record.next.binding)
                    .await
                    .unwrap();
                std::fs::rename(&active, transaction.join("attempt-1-previous")).unwrap();
            }
            assert!(matches!(
                manager.recover("worktree-one", &record.intent).await,
                Err(Refusal::PreparationFailed)
            ));
            assert_eq!(
                read_contract(&active.join("contract.json"))
                    .unwrap()
                    .binding,
                next.next.binding
            );
        }
    }

    #[test]
    fn successor_cannot_change_workspace_executor_or_reuse_lifetime() {
        let root = tempfile::tempdir().unwrap();
        let (_, record, _) = fixture(root.path());
        let mut next = successor(&record);
        assert!(validate_successor(&record, &next).is_ok());
        next.next.executor.command = "/another-executor".into();
        assert!(validate_successor(&record, &next).is_err());
        let mut next = successor(&record);
        next.next.binding.workspace.cwd = "/another-worktree".into();
        assert!(validate_successor(&record, &next).is_err());
        assert!(validate_successor(&record, &record).is_err());
    }

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
