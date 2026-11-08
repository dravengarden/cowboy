//! Durable Machine browser placements: where an account's browser profile
//! lives. Written when an open is admitted, read for state queries and
//! captured into the account's revocation plan. Never dispatch authority or
//! evidence that an instance is running.
use super::super::{StorageBackend, Store};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub account_id: ProviderAccountId,
    pub machine_id: String,
    pub instance_id: String,
    pub profile_revision: u64,
    pub last_open_ms: i64,
}

impl Store {
    pub async fn upsert_browser_placement(&self, owner: &str, placement: &Placement) -> Result<()> {
        macro_rules! write { ($db:expr) => {{
            sqlx::query("INSERT INTO browser_placements (owner_user_id,account_id,machine_id,instance_id,profile_revision,last_open_ms) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (owner_user_id,account_id,machine_id) DO UPDATE SET instance_id=excluded.instance_id,profile_revision=excluded.profile_revision,last_open_ms=excluded.last_open_ms")
                .bind(owner).bind(placement.account_id.as_str()).bind(&placement.machine_id).bind(&placement.instance_id)
                .bind(i64::try_from(placement.profile_revision).unwrap_or(i64::MAX)).bind(placement.last_open_ms)
                .execute(&$db.pool).await?;
        }} }
        match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        }
        Ok(())
    }

    pub async fn browser_placements(
        &self,
        owner: &str,
        account: &ProviderAccountId,
    ) -> Result<Vec<Placement>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_as::<_, (String, String, i64, i64)>("SELECT machine_id,instance_id,profile_revision,last_open_ms FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 ORDER BY machine_id")
                .bind(owner).bind(account.as_str()).fetch_all(&$db.pool).await?
        }; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(rows
            .into_iter()
            .map(
                |(machine_id, instance_id, profile_revision, last_open_ms)| Placement {
                    account_id: account.clone(),
                    machine_id,
                    instance_id,
                    profile_revision: u64::try_from(profile_revision).unwrap_or(0),
                    last_open_ms,
                },
            )
            .collect())
    }

    /// Machines holding a native credential generation of the account: where
    /// its CLI runs, so where its browser belongs.
    pub async fn native_generation_machines(
        &self,
        owner: &str,
        account: &ProviderAccountId,
    ) -> Result<Vec<String>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT machine_id FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2 ORDER BY machine_id")
                .bind(owner).bind(account.as_str()).fetch_all(&$db.pool).await?
        }; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }

    pub async fn remove_browser_placement(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        machine_id: &str,
    ) -> Result<()> {
        macro_rules! delete { ($db:expr) => {{
            sqlx::query("DELETE FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                .bind(owner).bind(account.as_str()).bind(machine_id).execute(&$db.pool).await?;
        }} }
        match &self.backend {
            StorageBackend::Postgres(db) => delete!(db),
            StorageBackend::Sqlite(db) => delete!(db),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn placements_are_owner_and_account_scoped_and_replace_per_machine() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(
            &format!(
                "sqlite://{}?mode=rwc",
                dir.path().join("browser.db").display()
            ),
            dir.path().join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let placement = Placement {
            account_id: account.clone(),
            machine_id: "ovh".into(),
            instance_id: "b-1".into(),
            profile_revision: 1,
            last_open_ms: 10,
        };
        store
            .upsert_browser_placement("owner", &placement)
            .await
            .unwrap();
        store
            .upsert_browser_placement(
                "owner",
                &Placement {
                    profile_revision: 2,
                    last_open_ms: 20,
                    ..placement.clone()
                },
            )
            .await
            .unwrap();
        let listed = store.browser_placements("owner", &account).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].profile_revision, 2);
        assert_eq!(listed[0].last_open_ms, 20);
        assert!(
            store
                .browser_placements("other", &account)
                .await
                .unwrap()
                .is_empty()
        );
        store
            .remove_browser_placement("owner", &account, "ovh")
            .await
            .unwrap();
        assert!(
            store
                .browser_placements("owner", &account)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
