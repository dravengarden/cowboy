//! Durable Machine browser placements: where an account's browser profile
//! lives. Committed under the owner lock before an open is dispatched, read
//! for state queries and captured into the account's revocation plan under
//! the same lock. Removed only by confirmed revocation evidence. Never
//! dispatch authority or evidence that an instance is running.
use super::super::{StorageBackend, Store};
use super::Registry;
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

/// Why the owner lock refused to admit a placement; the open fails with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The account does not exist, is disabled or is being revoked.
    AccountUnavailable,
    /// The owner has not enabled the browser for the account.
    BrowserNotEnabled,
    /// The member left the share, or the share is gone or being deleted.
    ShareChanged,
    /// The share's browser already lives on another Machine.
    ShareElsewhere,
}

impl Refusal {
    pub const fn code(self) -> &'static str {
        match self {
            Self::AccountUnavailable => "account_unavailable",
            Self::BrowserNotEnabled => "browser_not_enabled",
            Self::ShareChanged => "browser_share_changed",
            Self::ShareElsewhere => "browser_not_placed",
        }
    }
}

impl Store {
    /// Commit the placement an open is about to create, before the open is
    /// dispatched, under the owner lock that revocation capture takes. A
    /// revocation that begins afterwards captures it; one that began before
    /// is seen here as an unavailable account and refuses the open. An
    /// existing placement keeps its profile revision; a new one starts at 0
    /// until the Machine reports its revision. The placement is retained
    /// whatever the open's outcome: an unanswered open may still have run.
    pub async fn admit_browser_placement(
        &self,
        owner: &str,
        (account, card): (&ProviderAccountId, Option<&str>),
        machine_id: &str,
        instance_id: &str,
        now_ms: i64,
    ) -> Result<std::result::Result<(), Refusal>> {
        macro_rules! admit { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            if locked.rows_affected() != 1 {
                return Ok(Err(if card.is_some() { Refusal::BrowserNotEnabled } else { Refusal::AccountUnavailable }));
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            let entry = registry.accounts.iter().find(|entry| &entry.id == account);
            match (card, entry) {
                // A legacy card's browser: its own key and its own enablement,
                // never a registry account's.
                (Some(card), None) if super::legacy_browsers::account(owner, card) == *account => {
                    let enabled: Option<String> = sqlx::query_scalar("SELECT card_id FROM provider_legacy_browsers WHERE owner_user_id=$1 AND card_id=$2")
                        .bind(owner).bind(card).fetch_optional(&mut *tx).await?;
                    if enabled.is_none() {
                        return Ok(Err(Refusal::BrowserNotEnabled));
                    }
                }
                (None, Some(entry)) if !entry.available() => return Ok(Err(Refusal::AccountUnavailable)),
                (None, Some(entry)) if !entry.browser_enabled => return Ok(Err(Refusal::BrowserNotEnabled)),
                (None, Some(_)) => {}
                _ => return Ok(Err(Refusal::AccountUnavailable)),
            }
            sqlx::query("INSERT INTO browser_placements (owner_user_id,account_id,machine_id,instance_id,profile_revision,last_open_ms) VALUES ($1,$2,$3,$4,0,$5) ON CONFLICT (owner_user_id,account_id,machine_id) DO UPDATE SET instance_id=excluded.instance_id,last_open_ms=excluded.last_open_ms")
                .bind(owner).bind(account.as_str()).bind(machine_id).bind(instance_id).bind(now_ms)
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }} }
        match &self.backend {
            StorageBackend::Postgres(db) => admit!(db),
            StorageBackend::Sqlite(db) => admit!(db),
        }
        Ok(Ok(()))
    }

    /// Record the profile revision the Machine reported for an existing
    /// placement. Never creates one: a placement removed by revocation
    /// evidence stays removed.
    pub async fn record_browser_profile_revision(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        machine_id: &str,
        profile_revision: u64,
    ) -> Result<()> {
        macro_rules! write { ($db:expr) => {{
            sqlx::query("UPDATE browser_placements SET profile_revision=$4 WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                .bind(owner).bind(account.as_str()).bind(machine_id)
                .bind(i64::try_from(profile_revision).unwrap_or(i64::MAX))
                .execute(&$db.pool).await?;
        }} }
        match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        }
        Ok(())
    }

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

    /// Every placement on one Machine, across owners: what a reconnected
    /// Machine is asked about.
    pub async fn browser_placements_on_machine(
        &self,
        machine_id: &str,
    ) -> Result<Vec<(String, ProviderAccountId)>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_as::<_, (String, String)>("SELECT owner_user_id,account_id FROM browser_placements WHERE machine_id=$1 ORDER BY owner_user_id,account_id")
                .bind(machine_id).fetch_all(&$db.pool).await?
        }; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(rows
            .into_iter()
            .filter_map(|(owner, account)| {
                ProviderAccountId::try_from(account)
                    .ok()
                    .map(|account| (owner, account))
            })
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

    async fn sqlite_store(dir: &tempfile::TempDir) -> Store {
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
        store
    }

    async fn apply(
        store: &Store,
        owner: &str,
        operation: &str,
        change: super::super::Change,
    ) -> super::super::Receipt {
        let revision = store
            .provider_accounts(owner)
            .await
            .map_or(0, |registry| registry.revision);
        match store
            .mutate_provider_accounts(
                owner,
                operation,
                &super::super::Mutation {
                    expected_revision: revision,
                    change,
                },
            )
            .await
            .unwrap()
        {
            super::super::MutationResult::Applied(receipt) => receipt,
            other => panic!("{operation}: {other:?}"),
        }
    }

    async fn created_account(store: &Store, owner: &str) -> ProviderAccountId {
        apply(
            store,
            owner,
            "create",
            super::super::Change::Create {
                provider_id: "claude-code".into(),
                label: "Test".into(),
            },
        )
        .await
        .account_id
    }

    async fn enable_browser(store: &Store, owner: &str, account: &ProviderAccountId) {
        apply(
            store,
            owner,
            "enable-browser",
            super::super::Change::SetBrowserEnabled {
                id: account.clone(),
                enabled: true,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn placements_are_admitted_only_for_an_enabled_browser_and_keep_their_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = sqlite_store(&dir).await;
        let owner = "owner";
        let missing: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert_eq!(
            store
                .admit_browser_placement(owner, (&missing, None), "ovh", "b-1", 1)
                .await
                .unwrap(),
            Err(Refusal::AccountUnavailable)
        );
        let account = created_account(&store, owner).await;
        // The account exists, but its browser is off: nothing is placed.
        assert_eq!(
            store
                .admit_browser_placement(owner, (&account, None), "ovh", "b-1", 1)
                .await
                .unwrap(),
            Err(Refusal::BrowserNotEnabled)
        );
        assert!(
            store
                .browser_placements(owner, &account)
                .await
                .unwrap()
                .is_empty()
        );
        enable_browser(&store, owner, &account).await;
        store
            .admit_browser_placement(owner, (&account, None), "ovh", "b-1", 10)
            .await
            .unwrap()
            .unwrap();
        let placed = store.browser_placements(owner, &account).await.unwrap();
        assert_eq!(
            (placed[0].profile_revision, placed[0].last_open_ms),
            (0, 10)
        );
        store
            .record_browser_profile_revision(owner, &account, "ovh", 4)
            .await
            .unwrap();
        // A later admission keeps the reported revision.
        store
            .admit_browser_placement(owner, (&account, None), "ovh", "b-1", 20)
            .await
            .unwrap()
            .unwrap();
        let placed = store.browser_placements(owner, &account).await.unwrap();
        assert_eq!(
            (placed[0].profile_revision, placed[0].last_open_ms),
            (4, 20)
        );
        assert_eq!(
            store.browser_placements_on_machine("ovh").await.unwrap(),
            vec![(owner.to_owned(), account.clone())]
        );
    }

    #[tokio::test]
    async fn revocation_captures_admitted_placements_and_refuses_later_opens() {
        let dir = tempfile::tempdir().unwrap();
        let store = sqlite_store(&dir).await;
        let owner = "owner";
        let account = created_account(&store, owner).await;
        enable_browser(&store, owner, &account).await;
        // Committed before dispatch: an open that never answered still
        // leaves its placement for the revocation plan.
        store
            .admit_browser_placement(owner, (&account, None), "ovh", "b-1", 10)
            .await
            .unwrap()
            .unwrap();
        let receipt = apply(
            &store,
            owner,
            "revoke",
            super::super::Change::BeginRevocation {
                id: account.clone(),
            },
        )
        .await;
        assert_eq!(receipt.revocation.unwrap().browsers.len(), 1);
        assert_eq!(
            store
                .admit_browser_placement(owner, (&account, None), "hawk", "b-2", 30)
                .await
                .unwrap(),
            Err(Refusal::AccountUnavailable)
        );
        assert_eq!(
            store
                .browser_placements(owner, &account)
                .await
                .unwrap()
                .len(),
            1
        );
        // A revision report never recreates a removed placement.
        store
            .remove_browser_placement(owner, &account, "ovh")
            .await
            .unwrap();
        store
            .record_browser_profile_revision(owner, &account, "ovh", 5)
            .await
            .unwrap();
        assert!(
            store
                .browser_placements(owner, &account)
                .await
                .unwrap()
                .is_empty()
        );
    }

    async fn set_legacy(store: &Store, owner: &str, operation: &str, card: &str, enabled: bool) {
        apply(
            store,
            owner,
            operation,
            super::super::Change::SetLegacyBrowserEnabled {
                card: card.into(),
                enabled,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn a_legacy_card_browser_is_off_until_its_owner_enables_that_card() {
        let dir = tempfile::tempdir().unwrap();
        let store = sqlite_store(&dir).await;
        let owner = "owner";
        let lynx = super::super::legacy_browsers::account(owner, "anthropic");
        let mole = super::super::legacy_browsers::account(owner, "deepseek");
        // Off by default, also before the owner has any registry.
        assert!(store.legacy_browser_cards(owner).await.unwrap().is_empty());
        assert_eq!(
            store
                .admit_browser_placement(owner, (&lynx, Some("anthropic")), "ovh", "b-1", 1)
                .await
                .unwrap(),
            Err(Refusal::BrowserNotEnabled)
        );
        // Enabling one card enables only that card.
        set_legacy(&store, owner, "mole-on", "deepseek", true).await;
        assert_eq!(
            store.legacy_browser_cards(owner).await.unwrap(),
            vec!["deepseek".to_owned()]
        );
        assert_eq!(
            store
                .admit_browser_placement(owner, (&lynx, Some("anthropic")), "ovh", "b-1", 1)
                .await
                .unwrap(),
            Err(Refusal::BrowserNotEnabled)
        );
        // A card key never borrows another card's enablement, nor another owner's.
        assert_eq!(
            store
                .admit_browser_placement(owner, (&lynx, Some("deepseek")), "ovh", "b-1", 1)
                .await
                .unwrap(),
            Err(Refusal::AccountUnavailable)
        );
        assert!(
            store
                .legacy_browser_cards("other")
                .await
                .unwrap()
                .is_empty()
        );
        store
            .admit_browser_placement(owner, (&mole, Some("deepseek")), "ovh", "b-2", 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.browser_placements(owner, &mole).await.unwrap().len(),
            1
        );
        assert!(
            store
                .browser_placements(owner, &lynx)
                .await
                .unwrap()
                .is_empty()
        );
        // Disabling withdraws admission; the placement (retained profile) stays.
        set_legacy(&store, owner, "mole-off", "deepseek", false).await;
        assert!(store.legacy_browser_cards(owner).await.unwrap().is_empty());
        assert_eq!(
            store
                .admit_browser_placement(owner, (&mole, Some("deepseek")), "ovh", "b-2", 3)
                .await
                .unwrap(),
            Err(Refusal::BrowserNotEnabled)
        );
        assert_eq!(
            store.browser_placements(owner, &mole).await.unwrap().len(),
            1
        );
        // The registry account list is untouched by card enablement.
        assert!(
            store
                .provider_accounts(owner)
                .await
                .unwrap()
                .accounts
                .is_empty()
        );
    }

    #[tokio::test]
    async fn named_and_legacy_browsers_never_share_admission_or_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let store = sqlite_store(&dir).await;
        let owner = "owner";
        let named = created_account(&store, owner).await;
        enable_browser(&store, owner, &named).await;
        set_legacy(&store, owner, "lynx-on", "anthropic", true).await;
        let lynx = super::super::legacy_browsers::account(owner, "anthropic");
        assert_ne!(lynx, named);
        // A named account cannot be admitted as a card, nor a card key as a
        // named account.
        assert_eq!(
            store
                .admit_browser_placement(owner, (&named, Some("anthropic")), "ovh", "b-n", 1)
                .await
                .unwrap(),
            Err(Refusal::AccountUnavailable)
        );
        assert_eq!(
            store
                .admit_browser_placement(owner, (&lynx, None), "ovh", "b-l", 1)
                .await
                .unwrap(),
            Err(Refusal::AccountUnavailable)
        );
        store
            .admit_browser_placement(owner, (&named, None), "ovh", "b-n", 1)
            .await
            .unwrap()
            .unwrap();
        store
            .admit_browser_placement(owner, (&lynx, Some("anthropic")), "ovh", "b-l", 1)
            .await
            .unwrap()
            .unwrap();
        // Revoking the named account captures only its own browser.
        let receipt = apply(
            &store,
            owner,
            "revoke",
            super::super::Change::BeginRevocation { id: named.clone() },
        )
        .await;
        let plan = receipt.revocation.unwrap();
        assert_eq!(plan.browsers.len(), 1);
        assert_eq!(plan.browsers[0].instance_id, "b-n");
        // The card's browser stays admitted and placed.
        store
            .admit_browser_placement(owner, (&lynx, Some("anthropic")), "ovh", "b-l", 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.browser_placements(owner, &lynx).await.unwrap().len(),
            1
        );
    }
}
