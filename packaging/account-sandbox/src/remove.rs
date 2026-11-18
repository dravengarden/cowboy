//! `remove_account`: destroy an account's persistent home and its system
//! user (a revocation; a logout keeps both). The removal is idempotent per
//! `operation_id` and safe to retry after a crash at any step:
//!
//! 1. under the account lock, require the fence and a logged-out (or never
//!    signed-in) credential, then stop and confirm every unit of the account;
//! 2. persist the fence tombstone with the removal record, so the fence and
//!    the removal survive the metadata;
//! 3. unmount the home, delete the tree except `meta.json`, remove the user,
//!    and only then delete `meta.json` with the account directory.
//!
//! An account without a directory (removed, or never created here) has no
//! home: the daemon stops any unit of its name, removes its user if one
//! remains and answers the removal confirmed (a replay when the tombstone
//! records the same operation).

use std::os::fd::{AsFd as _, OwnedFd};

use serde_json::json;

use crate::daemon::{Daemon, Reply, guard};
use crate::fence::{self, Removal};
use crate::ids;
use crate::meta::{self, AccountMeta, CredentialState};
use crate::protocol::{Code, Outcome, fail};
use crate::safefs::{self, STRICT};

impl Daemon {
    pub(crate) fn remove_account(&self, key: &str, operation_id: Option<&str>) -> Outcome<Reply> {
        let result = {
            let lock = self.lock(key);
            let _guard = guard(&lock);
            self.remove_locked(key, operation_id)
        };
        if result.is_ok() {
            guard(&self.locks).remove(key);
        }
        result
    }

    fn remove_locked(&self, key: &str, operation_id: Option<&str>) -> Outcome<Reply> {
        let accounts = self.accounts()?;
        let account = if safefs::exists(accounts.as_fd(), key)? {
            let dir = safefs::open_dir(accounts.as_fd(), key, STRICT)?;
            meta::load(dir.as_fd())?.map(|meta| (dir, meta))
        } else {
            None
        };
        let Some((dir, mut meta)) = account else {
            return self.remove_absent(key, operation_id);
        };
        Self::check_removable(&meta)?;
        self.stop_all(&dir, &mut meta, |_| true)?;
        Self::save(&dir, &meta)?;
        self.keep_tombstone(
            key,
            &meta,
            Removal {
                operation_id: operation_id.map(str::to_owned),
                user: meta.user.clone(),
            },
        )?;
        self.remove_tree(key, &dir, &meta)?;
        Ok(removed(key, operation_id, false))
    }

    fn check_removable(meta: &AccountMeta) -> Outcome<()> {
        if meta.fenced.is_none() {
            return Err(fail(
                Code::Precondition,
                "remove_account requires a fenced account",
            ));
        }
        if !matches!(
            meta.credential.state,
            CredentialState::SignedOut
                | CredentialState::LoggedOut
                | CredentialState::LoginInterrupted
        ) {
            return Err(fail(
                Code::Precondition,
                "remove_account requires a logged-out account",
            ));
        }
        Ok(())
    }

    /// Delete the account's home and user; `meta.json` goes last, so a crash
    /// leaves the metadata for the retry to finish with.
    fn remove_tree(&self, key: &str, dir: &OwnedFd, meta: &AccountMeta) -> Outcome<()> {
        let paths = self.paths(key);
        if self.host.is_mounted(&paths.home)? {
            let home = self.open_home(dir, key)?;
            if Self::token_present(&home)? {
                return Err(fail(
                    Code::Precondition,
                    "a token is present; run logout_cleanup first",
                ));
            }
            drop(home);
            self.host.unmount(&paths.home)?;
        }
        for name in safefs::list(dir.as_fd())? {
            if name != meta::META_FILE {
                safefs::remove_tree(dir.as_fd(), &name)?;
            }
        }
        self.host.remove_user(&meta.user)?;
        let accounts = self.accounts()?;
        safefs::remove_tree(accounts.as_fd(), key)?;
        Ok(())
    }

    /// The account has no metadata here: finish a removal a crash
    /// interrupted (its directory, its user), or confirm there is nothing.
    fn remove_absent(&self, key: &str, operation_id: Option<&str>) -> Outcome<Reply> {
        let timeout = std::time::Duration::from_secs(self.config.stop_timeout_secs);
        for unit in self
            .host
            .list_units(&format!("{}-*", ids::user_name(key)))?
        {
            if self
                .host
                .unit_status(&unit)?
                .is_some_and(|status| status.is_live())
            {
                self.host.stop_unit(&unit)?;
                self.confirm_gone(&unit, timeout)?;
            }
        }
        self.ensure_none_live(key)?;
        let mut tombstone = fence::load(self.root.as_fd(), key)?.unwrap_or_default();
        let user = tombstone
            .removal
            .as_ref()
            .map_or_else(|| ids::user_name(key), |removal| removal.user.clone());
        let accounts = self.accounts()?;
        if safefs::exists(accounts.as_fd(), key)? {
            safefs::remove_tree(accounts.as_fd(), key)?;
        }
        self.host.remove_user(&user)?;
        let replayed = tombstone
            .removal
            .as_ref()
            .is_some_and(|removal| removal.operation_id.as_deref() == operation_id);
        if tombstone.removal.is_none() {
            tombstone.removal = Some(Removal {
                operation_id: operation_id.map(str::to_owned),
                user,
            });
            fence::save(self.root.as_fd(), key, &tombstone)?;
        }
        Ok(removed(key, operation_id, replayed))
    }
}

fn removed(key: &str, operation_id: Option<&str>, replayed: bool) -> Reply {
    Reply::value(json!({
        "key": key,
        "removed": true,
        "home_removed": true,
        "user_removed": true,
        "operation_id": operation_id,
        "replayed": replayed,
    }))
}
