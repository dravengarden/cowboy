//! The durable fence of a share reset. A reset of a share's profile is
//! recorded before it is dispatched and cleared only by its terminal outcome,
//! so a reset that timed out (still queued behind a slow open on the Machine)
//! can never delete the profile after a later membership change: those
//! changes are refused (`browser_share_reset_pending`) until the reset is
//! reconciled, also across a Controller restart.
use super::super::super::{StorageBackend, Store};
use super::{Member, PendingReset, Registry, ShareRow, decode, family_of};
use anyhow::Result;

impl Store {
    /// Record the reset of `share_id`'s profile about to be dispatched, under
    /// the owner lock: the share is live, still at the membership revision the
    /// caller observed, and no other reset of it is pending. The bindings of
    /// identities that already left are the ones a confirmed reset releases.
    pub async fn begin_share_reset(
        &self,
        owner: &str,
        (share_id, expected_membership_revision): (&str, i64),
        (machine_id, expected_profile_revision): (&str, u64),
        (operation_id, now_ms): (&str, i64),
    ) -> Result<std::result::Result<(), &'static str>> {
        macro_rules! begin { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let shares = decode(owner, load_shares!(&mut *tx, owner), load_members!(&mut *tx, owner), load_bindings!(&mut *tx, owner));
            let Some(share) = shares.share(share_id).filter(|share| share.deleting_ms.is_none()) else {
                return Ok(Err("browser_share_changed"));
            };
            if share.revision != expected_membership_revision {
                return Ok(Err("browser_identity_changed"));
            }
            if share.pending_reset.is_some() {
                return Ok(Err("browser_share_reset_pending"));
            }
            let current: Vec<&Member> = shares.members_of(share_id).into_iter().map(|row| &row.member).collect();
            let released = shares.bindings.iter()
                .filter(|row| row.share_id == share_id && !current.contains(&&row.member))
                .map(|row| row.member.encode()).collect();
            let pending = PendingReset { operation_id: operation_id.to_owned(), machine_id: machine_id.to_owned(),
                expected_profile_revision, started_ms: now_ms, released };
            sqlx::query("UPDATE provider_browser_shares SET pending_reset=$3 WHERE owner_user_id=$1 AND share_id=$2")
                .bind(owner).bind(share_id).bind(serde_json::to_string(&pending)?).execute(&mut *tx).await?;
            tx.commit().await?;
        }} }
        match &self.backend {
            StorageBackend::Postgres(db) => begin!(db),
            StorageBackend::Sqlite(db) => begin!(db),
        }
        Ok(Ok(()))
    }

    /// The terminal outcome of the pending reset `operation_id` is known:
    /// clear the fence. `fresh` (the Machine confirmed this reset itself)
    /// also releases the bindings of identities that had left before it began
    /// and binds the current members under their family now, in the same
    /// transaction, unless the share has a placement on another Machine.
    /// A renewal advances the registry revision (the account list changed),
    /// which it returns; a stale operation changes nothing.
    pub async fn finish_share_reset(
        &self,
        owner: &str,
        (share_id, operation_id): (&str, &str),
        fresh: bool,
    ) -> Result<Option<i64>> {
        let key = super::share_key(owner, share_id);
        macro_rules! finish { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let mut shares = decode(owner, load_shares!(&mut *tx, owner), load_members!(&mut *tx, owner), load_bindings!(&mut *tx, owner));
            let Some(pending) = shares.share(share_id).and_then(|share| share.pending_reset.clone())
                .filter(|pending| pending.operation_id == operation_id) else {
                return Ok(None);
            };
            let elsewhere: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 AND machine_id<>$3")
                .bind(owner).bind(key.as_str()).bind(&pending.machine_id).fetch_one(&mut *tx).await?;
            let value: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_optional(&mut *tx).await?;
            let mut registry: Registry = value.map(|value| serde_json::from_str(&value)).transpose()?.unwrap_or_default();
            let released: Vec<Member> = pending.released.iter().filter_map(|member| Member::decode(member)).collect();
            let renewed = fresh && elsewhere == 0
                && shares.renew_current(share_id, &released, |member| family_of(member, &registry));
            sqlx::query("UPDATE provider_browser_shares SET pending_reset=NULL WHERE owner_user_id=$1 AND share_id=$2")
                .bind(owner).bind(share_id).execute(&mut *tx).await?;
            if renewed {
                sqlx::query("DELETE FROM provider_browser_share_members WHERE owner_user_id=$1 AND share_id=$2")
                    .bind(owner).bind(share_id).execute(&mut *tx).await?;
                for row in shares.members_of(share_id) {
                    sqlx::query("INSERT INTO provider_browser_share_members (owner_user_id,member,share_id,family) VALUES ($1,$2,$3,$4)")
                        .bind(owner).bind(row.member.encode()).bind(&row.share_id).bind(&row.family).execute(&mut *tx).await?;
                }
                write_bindings!(tx, owner, shares);
                registry.revision = registry.revision.checked_add(1).ok_or_else(|| anyhow::anyhow!("revision_exhausted"))?;
                sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                    .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            renewed.then_some(registry.revision)
        }} }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => finish!(db),
            StorageBackend::Sqlite(db) => finish!(db),
        })
    }

    /// Every pending share reset, across owners: the reconciler's work,
    /// including after a Controller restart.
    pub async fn pending_share_resets(&self) -> Result<Vec<(String, String, PendingReset)>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_as::<_, (String, String, String)>("SELECT owner_user_id,share_id,pending_reset FROM provider_browser_shares WHERE pending_reset IS NOT NULL ORDER BY owner_user_id,share_id")
                .fetch_all(&$db.pool).await?
        }; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(rows
            .into_iter()
            .filter_map(|(owner, share, pending)| {
                serde_json::from_str(&pending)
                    .ok()
                    .map(|pending| (owner, share, pending))
            })
            .collect())
    }
}
