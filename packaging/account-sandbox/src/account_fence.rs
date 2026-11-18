//! `stop_account` / `admit_account`: the account fence, ordered by the
//! Controller's control revision (see [`crate::fence`]). A stop is
//! acknowledged only after the fence is durable (in `meta.json`, or in the
//! tombstone of an account without metadata) and every unit of the account is
//! confirmed gone.

use std::os::fd::{AsFd as _, OwnedFd};

use serde_json::json;

use crate::daemon::{Daemon, Reply, guard};
use crate::fence::{self, Admission, FenceAction, FenceOp};
use crate::ids;
use crate::meta::{self, AccountMeta};
use crate::protocol::{Code, Outcome, fail};
use crate::safefs::{self, STRICT};
use crate::units;

impl Daemon {
    pub(crate) fn fence_account(&self, key: &str, op: FenceOp<'_>) -> Outcome<Reply> {
        let lock = self.lock(key);
        let _guard = guard(&lock);
        let accounts = self.accounts()?;
        let account = if safefs::exists(accounts.as_fd(), key)? {
            let dir = safefs::open_dir(accounts.as_fd(), key, STRICT)?;
            meta::load(dir.as_fd())?.map(|meta| (dir, meta))
        } else {
            None
        };
        match account {
            Some((dir, mut meta)) => self.fence_present(key, &dir, &mut meta, op),
            None => self.fence_absent(key, op),
        }
    }

    fn fence_present(
        &self,
        key: &str,
        dir: &OwnedFd,
        meta: &mut AccountMeta,
        op: FenceOp<'_>,
    ) -> Outcome<Reply> {
        let admission = op.admit(meta.disposition.as_ref())?;
        match op.action {
            FenceAction::Stop => {
                if admission == Admission::Apply || meta.fenced.is_none() {
                    meta.fence_epoch += 1;
                    meta.fenced = Some(meta.fence_epoch);
                }
            }
            FenceAction::Admit => {
                if admission == Admission::Apply {
                    meta.fenced = None;
                }
            }
        }
        if let Some(disposition) = op.disposition() {
            meta.disposition = Some(disposition);
        }
        Self::save(dir, meta)?;
        let stopped = if op.action == FenceAction::Stop {
            let stopped = self.stop_all(dir, meta, |_| true);
            Self::save(dir, meta)?;
            let stopped = stopped?;
            let slice = units::slice_name(key);
            if self
                .host
                .unit_status(&slice)?
                .is_some_and(|status| status.is_live())
            {
                self.host.stop_unit(&slice)?;
            }
            self.ensure_none_live(key)?;
            stopped
        } else {
            Vec::new()
        };
        Ok(Reply::value(json!({
            "key": key,
            "fence_epoch": meta.fence_epoch,
            "fenced": meta.fenced.is_some(),
            "stopped": stopped,
            "revision": op.revision,
            "replayed": admission == Admission::Replay,
            "account_present": true,
        })))
    }

    /// Fence an account the daemon has no metadata for: the tombstone is
    /// durable before the reply, and `ensure_account` adopts it.
    fn fence_absent(&self, key: &str, op: FenceOp<'_>) -> Outcome<Reply> {
        let mut tombstone = fence::load(self.root.as_fd(), key)?.unwrap_or_default();
        let admission = op.admit(tombstone.disposition.as_ref())?;
        match op.action {
            FenceAction::Stop => {
                if admission == Admission::Apply || tombstone.fenced.is_none() {
                    tombstone.fence_epoch += 1;
                    tombstone.fenced = Some(tombstone.fence_epoch);
                }
            }
            FenceAction::Admit => {
                if admission == Admission::Apply {
                    tombstone.fenced = None;
                }
            }
        }
        if let Some(disposition) = op.disposition() {
            tombstone.disposition = Some(disposition);
        }
        fence::save(self.root.as_fd(), key, &tombstone)?;
        let mut stopped = Vec::new();
        if op.action == FenceAction::Stop {
            // No record exists, but a unit of the account's name must not.
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
                    stopped.push(unit);
                }
            }
            self.ensure_none_live(key)?;
        }
        Ok(Reply::value(json!({
            "key": key,
            "fence_epoch": tombstone.fence_epoch,
            "fenced": tombstone.fenced.is_some(),
            "stopped": stopped,
            "revision": op.revision,
            "replayed": admission == Admission::Replay,
            "account_present": false,
        })))
    }

    /// Confirm no unit of the account runs. A systemd inspection error is
    /// never a confirmation: the stop stays unconfirmed.
    pub(crate) fn ensure_none_live(&self, key: &str) -> Outcome<()> {
        let mut live = Vec::new();
        for unit in self
            .host
            .list_units(&format!("{}-*", ids::user_name(key)))?
        {
            if self
                .host
                .unit_status(&unit)?
                .is_some_and(|status| status.is_live())
            {
                live.push(unit);
            }
        }
        if live.is_empty() {
            Ok(())
        } else {
            Err(fail(
                Code::Internal,
                format!("units still running: {}", live.join(" ")),
            ))
        }
    }

    /// Adopt the account's fence tombstone into its metadata (under the
    /// account lock) and then drop the tombstone. Restart safe: the metadata
    /// records the tombstone it adopted, so one left behind by a crash before
    /// its removal is only removed, and a tombstone never replaces a fence
    /// of a newer control revision the metadata already applied.
    pub(crate) fn adopt_tombstone(
        &self,
        key: &str,
        dir: &OwnedFd,
        meta: &mut AccountMeta,
    ) -> Outcome<()> {
        let Some(tombstone) = fence::load(self.root.as_fd(), key)? else {
            return Ok(());
        };
        let mark = tombstone.mark();
        if meta.adopted_tombstone.as_ref() != Some(&mark) {
            let revision = |disposition: Option<&fence::Disposition>| {
                disposition.map(|disposition| disposition.revision)
            };
            let newer = match (
                revision(tombstone.disposition.as_ref()),
                revision(meta.disposition.as_ref()),
            ) {
                (Some(tombstone), Some(meta)) => tombstone > meta,
                (Some(_), None) => true,
                // An unrevisioned stop only tightens an account no
                // revisioned control owns.
                (None, current) => tombstone.fenced.is_some() && current.is_none(),
            };
            meta.fence_epoch = meta.fence_epoch.max(tombstone.fence_epoch);
            if newer {
                meta.fenced = tombstone.fenced.map(|_| meta.fence_epoch);
                if tombstone.disposition.is_some() {
                    meta.disposition = tombstone.disposition;
                }
            }
            meta.adopted_tombstone = Some(mark);
            Self::save(dir, meta)?;
        }
        fence::remove(self.root.as_fd(), key)?;
        Ok(())
    }

    /// Keep a removed account's fence so a recreated account stays fenced,
    /// with the removal that is about to delete its metadata.
    pub(crate) fn keep_tombstone(
        &self,
        key: &str,
        meta: &AccountMeta,
        removal: fence::Removal,
    ) -> Outcome<()> {
        let tombstone = fence::Tombstone {
            schema: 0,
            fence_epoch: meta.fence_epoch,
            fenced: meta.fenced,
            disposition: meta.disposition.clone(),
            removal: Some(removal),
        };
        fence::save(self.root.as_fd(), key, &tombstone)?;
        Ok(())
    }
}
