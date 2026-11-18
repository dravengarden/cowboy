//! `accounts/<key>/meta.json` (root 0600): the daemon's only durable state per
//! account. Written atomically (temporary file, fsync, rename, directory
//! fsync) beneath the account directory descriptor.

use std::os::fd::BorrowedFd;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::protocol::Mode;
use crate::safefs;

pub const META_FILE: &str = "meta.json";
const MAX_META_BYTES: u64 = 1024 * 1024;
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountMeta {
    pub schema: u32,
    pub key: String,
    pub service_id: String,
    pub account_id: String,
    pub user: String,
    pub uid: u32,
    pub gid: u32,
    /// The home image was fully formatted (an image without it is recreated).
    pub image_ready: bool,
    /// Last fence epoch issued.
    pub fence_epoch: u64,
    /// `Some(epoch)` while `stop_account` fences the account (R3-4).
    pub fenced: Option<u64>,
    pub credential: Credential,
    /// Units the daemon started and has not yet seen finish (R3-12).
    pub units: Vec<UnitRecord>,
    /// Unresolved cleanups, retried on every activation.
    pub cleanups: Vec<Cleanup>,
    pub last_logout: Option<LogoutRecord>,
    /// The last revisioned fence operation applied (see [`crate::fence`]).
    #[serde(default)]
    pub disposition: Option<crate::fence::Disposition>,
    /// The fence tombstone this metadata already adopted: a tombstone left
    /// behind by a crash between adoption and its removal is recognized and
    /// never applied over a newer fence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopted_tombstone: Option<crate::fence::TombstoneMark>,
}

/// The account's credential generation (R3-8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub generation: u64,
    pub state: CredentialState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialState {
    /// Never signed in.
    SignedOut,
    LoginRunning,
    SignedIn,
    /// The login unit ended without a token; its partial token is removed.
    LoginInterrupted,
    LogoutRunning,
    /// The logout unit ended; `logout_cleanup` still has to run.
    LogoutPending,
    /// `logout_cleanup` removed the token; history is retained.
    LoggedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitPhase {
    /// Recorded before `systemd-run` was spawned.
    Starting,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitRecord {
    pub unit: String,
    pub nonce: String,
    pub mode: Mode,
    pub phase: UnitPhase,
    /// `<daemon instance>-<connection number>` of the owning connection.
    pub connection: String,
    pub invocation_id: Option<String>,
    pub boot_id: String,
    pub credential_generation: u64,
    pub runtime_sha256: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Cleanup {
    /// Remove the partial token an interrupted login of `generation` left.
    PartialToken { generation: u64 },
    /// Remove the token after a logout unit of `generation`.
    LogoutToken { generation: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogoutRecord {
    pub generation: u64,
    pub credential_cleared: bool,
    pub history_retained: bool,
}

impl AccountMeta {
    #[must_use]
    pub fn new(
        key: &str,
        service_id: &str,
        account_id: &str,
        user: &str,
        (uid, gid): (u32, u32),
    ) -> Self {
        Self {
            schema: SCHEMA,
            key: key.to_owned(),
            service_id: service_id.to_owned(),
            account_id: account_id.to_owned(),
            user: user.to_owned(),
            uid,
            gid,
            image_ready: false,
            fence_epoch: 0,
            fenced: None,
            credential: Credential {
                generation: 0,
                state: CredentialState::SignedOut,
            },
            units: Vec::new(),
            cleanups: Vec::new(),
            last_logout: None,
            disposition: None,
            adopted_tombstone: None,
        }
    }

    #[must_use]
    pub fn owner(&self) -> (u32, u32) {
        (self.uid, self.gid)
    }

    /// Recorded units of the runtime class (`runtime`, `probe`).
    #[must_use]
    pub fn runtime_units(&self) -> usize {
        self.units
            .iter()
            .filter(|record| !record.mode.is_auth())
            .count()
    }

    /// The recorded authentication unit, if any.
    #[must_use]
    pub fn auth_unit(&self) -> Option<&UnitRecord> {
        self.units.iter().find(|record| record.mode.is_auth())
    }
}

/// Load `meta.json` from the account directory; `None` when absent.
pub fn load(account: BorrowedFd<'_>) -> Result<Option<AccountMeta>> {
    let Some(bytes) = safefs::read_file(account, META_FILE, MAX_META_BYTES)? else {
        return Ok(None);
    };
    let meta: AccountMeta = serde_json::from_slice(&bytes).context("parse meta.json")?;
    anyhow::ensure!(
        meta.schema == SCHEMA,
        "unsupported meta.json schema {}",
        meta.schema
    );
    Ok(Some(meta))
}

/// Atomically persist `meta`.
pub fn save(account: BorrowedFd<'_>, meta: &AccountMeta) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(meta)?;
    bytes.push(b'\n');
    safefs::write_atomic(account, META_FILE, &bytes, 0o600)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd as _;

    #[test]
    fn meta_round_trips_atomically_and_strictly() {
        let temp = tempfile::tempdir().unwrap();
        let fd = safefs::open_root(temp.path()).unwrap();
        assert!(load(fd.as_fd()).unwrap().is_none());
        let mut meta =
            AccountMeta::new("0123456789abcdef", "s", "a", "cga-0123456789abcdef", (1, 2));
        meta.cleanups.push(Cleanup::PartialToken { generation: 3 });
        save(fd.as_fd(), &meta).unwrap();
        assert_eq!(load(fd.as_fd()).unwrap().unwrap(), meta);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(temp.path().join(META_FILE)).unwrap()).unwrap();
        value["extra"] = serde_json::json!(1);
        std::fs::write(temp.path().join(META_FILE), value.to_string()).unwrap();
        assert!(load(fd.as_fd()).is_err());
    }
}
