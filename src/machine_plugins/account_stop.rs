//! Durable per-session account-stop intent, not proof of process exit or logout.
use super::{MachinePluginStore, atomic_write, native_identity};
use crate::provider_account::{ProviderAccountId, prepare_private_directory};
use anyhow::{Result, ensure};
use sha2::{Digest as _, Sha256};
use std::{fs, path::PathBuf};
pub(crate) mod transition;
mod worker_exit;
pub(crate) use worker_exit::SystemdWorker;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    operation: String,
    launch: uuid::Uuid,
    binding: worker_exit::WorkerBinding,
}

fn managed_marker(bytes: &[u8]) -> Result<Option<(Admission, bool)>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get(0).and_then(serde_json::Value::as_u64) != Some(3) {
        return Ok(None);
    }
    let (_, _, _, _, admission, stopped): (
        u32,
        String,
        ProviderAccountId,
        String,
        Admission,
        bool,
    ) = serde_json::from_slice(bytes)?;
    ensure!(
        crate::managed_calls::valid_id(&admission.operation),
        "invalid admission operation"
    );
    ensure!(!admission.launch.is_nil(), "invalid admission launch");
    admission.binding.validate()?;
    Ok(Some((admission, stopped)))
}

fn marker_name(session: &str) -> String {
    format!("{:x}", Sha256::digest(session.as_bytes()))
}

fn decode_marker(bytes: &[u8]) -> Result<(String, ProviderAccountId, Option<String>)> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Record {
        Legacy((u32, String, ProviderAccountId)),
        Scoped((u32, String, ProviderAccountId, String)),
        Managed((u32, String, ProviderAccountId, String, Admission, bool)),
    }
    let (session, account, command) = match serde_json::from_slice(bytes)? {
        Record::Legacy((schema, session, account)) => {
            ensure!(schema == 1, "invalid account stop schema");
            (session, account, None)
        }
        Record::Scoped((schema, session, account, command)) => {
            ensure!(
                schema == 2 && crate::managed_calls::valid_id(&command),
                "invalid account stop command"
            );
            (session, account, Some(command))
        }
        Record::Managed((schema, session, account, command, admission, stopped)) => {
            ensure!(!admission.launch.is_nil(), "invalid admission launch");
            ensure!(
                schema == 3
                    && crate::managed_calls::valid_id(&command)
                    && crate::managed_calls::valid_id(&admission.operation),
                "invalid managed stop"
            );
            admission.binding.validate()?;
            if !stopped {
                ensure!(
                    command
                        == crate::provider_account::account_stop_command_id(
                            &format!("migration:{}", admission.operation),
                            &session,
                            &account
                        ),
                    "released stop operation mismatch"
                );
            }
            (session, account, Some(command))
        }
    };
    ensure!(
        !session.is_empty()
            && session.len() <= 512
            && !session.contains('\0')
            && !account.is_default(),
        "invalid account stop record"
    );
    Ok((session, account, command))
}

impl MachinePluginStore {
    /// Validate the complete recovery set before any process-stop effect.
    pub(crate) fn account_stops(&self) -> Result<Vec<(String, ProviderAccountId)>> {
        let root = self.account_stop_root();
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            result => ensure!(result?.is_dir(), "invalid account stop namespace"),
        }
        let mut stops = Vec::new();
        for (index, entry) in fs::read_dir(&root)?.enumerate() {
            ensure!(index < 65536, "account stop recovery budget exceeded");
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid account stop filename"))?;
            // Uncommitted atomic-write staging files have no authority.
            if name.starts_with('.') && name.ends_with(".partial") {
                continue;
            }
            let bytes = native_identity::read_login_file(&root, &name, 4096)?;
            let (session, account, _) = decode_marker(&bytes)?;
            ensure!(
                !session.is_empty()
                    && session.len() <= 512
                    && !session.contains('\0')
                    && !account.is_default()
                    && marker_name(&session) == name,
                "invalid account stop recovery record"
            );
            if managed_marker(&bytes)?.is_none_or(|(_, stopped)| stopped) {
                stops.push((session, account));
            }
        }
        stops.sort();
        Ok(stops)
    }

    pub(crate) fn account_stop_matches(
        &self,
        session: &str,
        account: &ProviderAccountId,
    ) -> Result<bool> {
        if !self.account_session_stopped(session)? {
            return Ok(false);
        }
        let bytes = native_identity::read_login_file(
            &self.account_stop_root(),
            &marker_name(session),
            4096,
        )?;
        let (record_session, record_account, _) = decode_marker(&bytes)?;
        Ok(record_session == session && record_account == *account)
    }

    pub(crate) fn account_stop_command(
        &self,
        session: &str,
        account: &ProviderAccountId,
    ) -> Result<Option<String>> {
        if !self.account_stop_matches(session, account)? {
            return Ok(None);
        }
        let bytes = native_identity::read_login_file(
            &self.account_stop_root(),
            &marker_name(session),
            4096,
        )?;
        Ok(decode_marker(&bytes)?.2)
    }
    fn account_stop_root(&self) -> PathBuf {
        self.auth_base.join("session-account-stops")
    }

    /// Any present marker (including damaged state) refuses admission. No
    /// account-bearing launch declaration is needed to fence a reconnecting peer.
    pub(crate) fn account_session_stopped(&self, session: &str) -> Result<bool> {
        let root = self.account_stop_root();
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            result => ensure!(result?.is_dir(), "invalid account stop namespace"),
        }
        let file = root.join(marker_name(session));
        match fs::symlink_metadata(file) {
            Ok(_) => {
                let Ok(bytes) =
                    native_identity::read_login_file(&root, &marker_name(session), 4096)
                else {
                    return Ok(true);
                };
                // Malformed records continue to fence rather than granting access.
                let Ok((record_session, account, _)) = decode_marker(&bytes) else {
                    return Ok(true);
                };
                if record_session != session {
                    return Ok(true);
                }
                let Some((admission, stopped)) = managed_marker(&bytes)? else {
                    return Ok(true);
                };
                if stopped {
                    return Ok(true);
                }
                Ok(!self.account_worker_exit(session)?.is_some_and(|worker| {
                    worker.account == account && worker.binding.as_ref() == Some(&admission.binding)
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Persist intent before the broker reports or attempts a stop. Replays are
    /// idempotent; no operation here removes the fence or touches credentials.
    #[cfg(test)]
    pub(crate) fn stop_account_session(
        &self,
        session: &str,
        account: &ProviderAccountId,
    ) -> Result<()> {
        self.persist_account_stop(session, account, None)
    }

    pub(crate) fn stop_account_session_scoped(
        &self,
        session: &str,
        account: &ProviderAccountId,
        command: &str,
    ) -> Result<()> {
        ensure!(
            crate::managed_calls::valid_id(command),
            "invalid account stop command"
        );
        self.persist_account_stop(session, account, Some(command))
    }

    fn persist_account_stop(
        &self,
        session: &str,
        account: &ProviderAccountId,
        command: Option<&str>,
    ) -> Result<()> {
        ensure!(!account.is_default(), "named account required");
        ensure!(
            !session.is_empty() && session.len() <= 512 && !session.contains('\0'),
            "invalid session identity"
        );
        let _guard = self.auth_candidates.lock();
        let root = self.account_stop_root();
        prepare_private_directory(&root)?;
        // A retry also re-syncs a marker left by an uncertain first commit.
        // Presence is intentionally sufficient to fence; never repair or
        // replace an existing malformed marker in the stop path.
        let file = root.join(marker_name(session));
        let previous = match fs::symlink_metadata(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            result => {
                ensure!(result?.is_file(), "invalid account stop file");
                let bytes = native_identity::read_login_file(&root, &marker_name(session), 4096)?;
                let (record_session, record_account, _) = decode_marker(&bytes)?;
                ensure!(
                    record_session == session && record_account == *account,
                    "account stop scope mismatch"
                );
                Some(bytes)
            }
        };
        let current_command = previous
            .as_ref()
            .map(|bytes| decode_marker(bytes))
            .transpose()?
            .and_then(|record| record.2);
        let admission = previous
            .as_ref()
            .map(|bytes| managed_marker(bytes))
            .transpose()?
            .flatten();
        if !file.exists()
            || admission.as_ref().is_some_and(|(_, stopped)| !stopped)
            || command.is_some_and(|command| current_command.as_deref() != Some(command))
        {
            let bytes = if let Some((admission, _)) = admission {
                serde_json::to_vec(&(
                    3u32,
                    session,
                    account,
                    command
                        .or(current_command.as_deref())
                        .ok_or_else(|| anyhow::anyhow!("missing managed stop command"))?,
                    admission,
                    true,
                ))?
            } else {
                match command {
                    Some(command) => serde_json::to_vec(&(2u32, session, account, command))?,
                    None => serde_json::to_vec(&(1u32, session, account))?,
                }
            };
            atomic_write(&file, &bytes, 0o600)?;
        }
        fs::File::open(&root)?.sync_all()?;
        fs::File::open(&self.auth_base)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::Platform;

    #[test]
    fn stops_survive_reopen_and_damaged_markers_still_refuse() {
        let temp = tempfile::tempdir().unwrap();
        let open =
            || MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let store = open();
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(!store.account_session_stopped("session").unwrap());
        assert!(
            store
                .stop_account_session("session", &ProviderAccountId::default())
                .is_err()
        );
        store.stop_account_session("session", &account).unwrap();
        store.stop_account_session("session", &account).unwrap();
        fs::write(
            store.account_stop_root().join(".unfinished.partial"),
            b"not committed",
        )
        .unwrap();
        assert_eq!(
            store.account_stops().unwrap(),
            vec![("session".into(), account.clone())]
        );
        assert!(store.account_stop_matches("session", &account).unwrap());
        let other = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(!store.account_stop_matches("session", &other).unwrap());
        assert!(store.stop_account_session("session", &other).is_err());
        let marker = store.account_stop_root().join(marker_name("session"));
        fs::write(&marker, b"damaged").unwrap();
        assert!(store.account_stops().is_err());
        assert!(store.stop_account_session("session", &account).is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"damaged");
        drop(store);
        let store = open();
        assert!(store.account_session_stopped("session").unwrap());
        assert!(store.stop_account_session("session", &account).is_err());
        assert!(store.account_stops().is_err());
        assert!(!store.account_session_stopped("other").unwrap());
        fs::remove_file(&marker).unwrap();
        std::os::unix::fs::symlink("/nonexistent-account-stop", &marker).unwrap();
        assert!(store.account_session_stopped("session").unwrap());
        assert!(store.stop_account_session("session", &account).is_err());
        assert!(fs::symlink_metadata(&marker).unwrap().is_symlink());
    }
}
