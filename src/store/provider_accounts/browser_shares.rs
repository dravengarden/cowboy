//! Owner-defined shared Machine browser profiles. A share lets several
//! browser identities (named accounts and legacy default usage cards) use
//! one browser profile, so an owner signs in to a common identity provider
//! once. Isolation stays per browser identity: a share holds at most one
//! member per account-usage family (`anthropic`, `openai`, `xai`, ...), so
//! two accounts of the same kind can never share a profile. That rule is a
//! unique constraint of the table, a mutation refusal and, on the server, a
//! refusal at resolution.
//!
//! The browser key of a share is a deterministic UUID derived from the owner
//! and the share id under its own domain separator, so it never equals a
//! registry identity (version 4), a legacy card key (another domain) or the
//! literal `default`. The Machine keys profiles only by that key and needs no
//! knowledge of shares. Share state changes only through the account
//! registry writer (owner lock, registry revision, operation receipt).
use super::super::{StorageBackend, Store};
use super::browser_placements::Refusal;
use super::{Change, Mutation, MutationResult, Receipt, Registry, label_valid, legacy_browsers};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use std::collections::{BTreeMap, BTreeSet};

const DOMAIN: &[u8] = b"cowboy-browser-share-v1\0";
/// Live shares per owner.
const MAX_SHARES: usize = 32;

/// The browser key of `owner`'s share `share_id`.
pub fn share_key(owner: &str, share_id: &str) -> ProviderAccountId {
    let mut hasher = sha2::Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(owner.as_bytes());
    hasher.update([0]);
    hasher.update(share_id.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 version 8 (custom) with the RFC variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ProviderAccountId::try_from(uuid::Uuid::from_bytes(bytes).hyphenated().to_string())
        .expect("canonical lowercase UUID")
}

/// A share id is a canonical lowercase UUID.
pub fn share_id_valid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.hyphenated().to_string() == id)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountMember {
    pub account_id: ProviderAccountId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardMember {
    pub card: String,
}

/// A browser identity that may join a share: `{account_id}` for a named
/// account, `{card}` for a legacy default usage card.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Member {
    Account(AccountMember),
    Card(CardMember),
}

impl Member {
    pub fn account(account_id: ProviderAccountId) -> Self {
        Self::Account(AccountMember { account_id })
    }

    pub fn card(card: &str) -> Self {
        Self::Card(CardMember {
            card: card.to_owned(),
        })
    }

    /// The stored form: `account:<id>` or `card:<card>`.
    pub fn encode(&self) -> String {
        match self {
            Self::Account(member) => format!("account:{}", member.account_id.as_str()),
            Self::Card(member) => format!("card:{}", member.card),
        }
    }

    pub fn decode(value: &str) -> Option<Self> {
        if let Some(id) = value.strip_prefix("account:") {
            return ProviderAccountId::try_from(id.to_owned())
                .ok()
                .filter(|id| !id.is_default())
                .map(Self::account);
        }
        value
            .strip_prefix("card:")
            .filter(|card| legacy_browsers::card_valid(card))
            .map(Self::card)
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Account(member) => !member.account_id.is_default(),
            Self::Card(member) => legacy_browsers::card_valid(&member.card),
        }
    }

    /// The member's own browser key, used whenever it is in no share.
    pub fn own_key(&self, owner: &str) -> ProviderAccountId {
        match self {
            Self::Account(member) => member.account_id.clone(),
            Self::Card(member) => legacy_browsers::account(owner, &member.card),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Share {
    pub share_id: String,
    pub browser_key: ProviderAccountId,
    pub label: String,
    pub created_ms: i64,
    pub revision: i64,
    /// Set while the deleted share's profile is being revoked.
    pub deleting_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShareMember {
    pub member: Member,
    pub share_id: String,
    pub family: String,
}

/// The member a family of a share's profile belongs to. Recorded at join
/// and kept after that member leaves or is revoked: the profile may hold its
/// cookies, so no other account of the family may use it until the profile
/// is fresh again (a Machine-confirmed reset) or the share is deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Binding {
    pub share_id: String,
    pub family: String,
    pub member: Member,
}

/// One owner's shares, memberships and family bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shares {
    pub shares: Vec<Share>,
    pub members: Vec<ShareMember>,
    pub bindings: Vec<Binding>,
}

impl Shares {
    pub fn share(&self, share_id: &str) -> Option<&Share> {
        self.shares.iter().find(|share| share.share_id == share_id)
    }

    pub fn by_key(&self, key: &ProviderAccountId) -> Option<&Share> {
        self.shares.iter().find(|share| &share.browser_key == key)
    }

    pub fn members_of(&self, share_id: &str) -> Vec<&ShareMember> {
        self.members
            .iter()
            .filter(|member| member.share_id == share_id)
            .collect()
    }

    pub fn membership(&self, member: &Member) -> Option<&ShareMember> {
        self.members.iter().find(|row| &row.member == member)
    }

    /// The member `family` of the share's profile belongs to, if any.
    pub fn binding(&self, share_id: &str, family: &str) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|row| row.share_id == share_id && row.family == family)
    }

    /// Forget a share's family bindings except those of its current
    /// members: its profile is fresh.
    pub fn rebind_current(&mut self, share_id: &str) {
        let current: Vec<Member> = self
            .members_of(share_id)
            .into_iter()
            .map(|row| row.member.clone())
            .collect();
        self.bindings
            .retain(|row| row.share_id != share_id || current.contains(&row.member));
    }

    fn live(&self, share_id: &str) -> Result<&Share, &'static str> {
        self.share(share_id)
            .filter(|share| share.deleting_ms.is_none())
            .ok_or("browser_share_not_found")
    }

    fn bump(&mut self, share_id: &str) {
        if let Some(share) = self
            .shares
            .iter_mut()
            .find(|share| share.share_id == share_id)
        {
            share.revision = share.revision.saturating_add(1);
        }
    }
}

/// Everything a share change is checked against, read under the owner lock.
pub struct ShareContext<'a> {
    pub owner: &'a str,
    pub registry: &'a Registry,
    /// Account-usage family of each Provider id (from the Provider catalog).
    pub families: &'a BTreeMap<String, String>,
    /// Browser keys of this owner that hold a placement somewhere.
    pub placed: &'a BTreeSet<ProviderAccountId>,
    pub now_ms: i64,
    /// The id a created share receives.
    pub new_share_id: String,
}

/// What a share change touched: the browser key it is about and, for a
/// share-level change, the share id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub subject: ProviderAccountId,
    pub share_id: Option<String>,
}

/// The family of a member: a legacy card is its own account-usage provider;
/// a named account maps its Provider through the catalog. A member whose
/// family cannot be resolved never joins a share.
pub fn member_family(
    member: &Member,
    registry: &Registry,
    families: &BTreeMap<String, String>,
) -> Result<String, &'static str> {
    match member {
        Member::Card(card) => Ok(card.card.clone()),
        Member::Account(member) => {
            let account = registry
                .accounts
                .iter()
                .find(|account| account.id == member.account_id)
                .ok_or("account_not_found")?;
            if account.revoking {
                return Err("account_revoking");
            }
            families
                .get(&account.provider_id)
                .cloned()
                .ok_or("browser_share_family_unknown")
        }
    }
}

/// Whether a share's members are of pairwise distinct families. Always true
/// for stored state (a unique constraint); checked again before use.
pub fn families_distinct(members: &[&ShareMember]) -> bool {
    let families: BTreeSet<&str> = members.iter().map(|row| row.family.as_str()).collect();
    families.len() == members.len()
}

fn join(
    shares: &mut Shares,
    member: &Member,
    share_id: &str,
    ctx: &ShareContext<'_>,
) -> Result<(), &'static str> {
    shares.live(share_id)?;
    let family = member_family(member, ctx.registry, ctx.families)?;
    if shares
        .membership(member)
        .is_some_and(|row| row.share_id == share_id && row.family == family)
    {
        return Ok(());
    }
    if shares
        .members_of(share_id)
        .iter()
        .any(|row| &row.member != member && row.family == family)
    {
        return Err("browser_share_family_conflict");
    }
    // The profile may still hold an earlier member's cookies of this family.
    if shares
        .binding(share_id, &family)
        .is_some_and(|row| &row.member != member)
    {
        return Err("browser_share_family_bound");
    }
    leave(shares, member);
    if shares.binding(share_id, &family).is_none() {
        shares.bindings.push(Binding {
            share_id: share_id.to_owned(),
            family: family.clone(),
            member: member.clone(),
        });
    }
    shares.members.push(ShareMember {
        member: member.clone(),
        share_id: share_id.to_owned(),
        family,
    });
    shares.bump(share_id);
    Ok(())
}

fn leave(shares: &mut Shares, member: &Member) {
    if let Some(index) = shares.members.iter().position(|row| &row.member == member) {
        let row = shares.members.remove(index);
        shares.bump(&row.share_id);
    }
}

/// Apply one share change to an owner's shares.
pub fn apply(
    shares: &mut Shares,
    change: &Change,
    ctx: &ShareContext<'_>,
) -> Result<Applied, &'static str> {
    match change {
        Change::CreateBrowserShare { label } => {
            if !label_valid(label) {
                return Err("invalid_browser_share");
            }
            let live = shares
                .shares
                .iter()
                .filter(|share| share.deleting_ms.is_none())
                .count();
            if live >= MAX_SHARES {
                return Err("browser_share_capacity");
            }
            let share_id = ctx.new_share_id.clone();
            let browser_key = share_key(ctx.owner, &share_id);
            shares.shares.push(Share {
                share_id: share_id.clone(),
                browser_key: browser_key.clone(),
                label: label.trim().to_owned(),
                created_ms: ctx.now_ms,
                revision: 1,
                deleting_ms: None,
            });
            Ok(Applied {
                subject: browser_key,
                share_id: Some(share_id),
            })
        }
        Change::RenameBrowserShare { share_id, label } => {
            if !label_valid(label) {
                return Err("invalid_browser_share");
            }
            let key = shares.live(share_id)?.browser_key.clone();
            if let Some(share) = shares
                .shares
                .iter_mut()
                .find(|share| &share.share_id == share_id)
            {
                label.trim().clone_into(&mut share.label);
            }
            // A label is no membership change: the browser identity stays.
            Ok(Applied {
                subject: key,
                share_id: Some(share_id.clone()),
            })
        }
        Change::DeleteBrowserShare { share_id } => {
            let key = shares.live(share_id)?.browser_key.clone();
            shares.members.retain(|row| &row.share_id != share_id);
            shares.bindings.retain(|row| &row.share_id != share_id);
            if ctx.placed.contains(&key) {
                // Its profile lives on a Machine: revoked, then forgotten.
                shares.bump(share_id);
                if let Some(share) = shares
                    .shares
                    .iter_mut()
                    .find(|share| &share.share_id == share_id)
                {
                    share.deleting_ms = Some(ctx.now_ms);
                }
            } else {
                shares.shares.retain(|share| &share.share_id != share_id);
            }
            Ok(Applied {
                subject: key,
                share_id: Some(share_id.clone()),
            })
        }
        Change::SetBrowserShare { member, share_id } => {
            if !member.valid() {
                return Err("invalid_argument");
            }
            // Only the owner's own identities, even to leave (a no-op too).
            if let Member::Account(account) = member
                && !ctx
                    .registry
                    .accounts
                    .iter()
                    .any(|entry| entry.id == account.account_id)
            {
                return Err("account_not_found");
            }
            match share_id {
                Some(share_id) => join(shares, member, share_id, ctx)?,
                None => leave(shares, member),
            }
            Ok(Applied {
                subject: member.own_key(ctx.owner),
                share_id: share_id.clone(),
            })
        }
        _ => Err("invalid_change"),
    }
}

type ShareRow = (String, String, String, i64, i64, Option<i64>);

fn decode(
    owner: &str,
    rows: Vec<ShareRow>,
    members: Vec<(String, String, String)>,
    bindings: Vec<(String, String, String)>,
) -> Shares {
    Shares {
        shares: rows
            .into_iter()
            .filter(|(share_id, ..)| share_id_valid(share_id))
            .map(
                |(share_id, _, label, created_ms, revision, deleting_ms)| Share {
                    // Derived again: the stored key is a lookup index only.
                    browser_key: share_key(owner, &share_id),
                    share_id,
                    label,
                    created_ms,
                    revision,
                    deleting_ms,
                },
            )
            .collect(),
        members: members
            .into_iter()
            .filter_map(|(member, share_id, family)| {
                Member::decode(&member).map(|member| ShareMember {
                    member,
                    share_id,
                    family,
                })
            })
            .collect(),
        bindings: bindings
            .into_iter()
            .filter_map(|(share_id, family, member)| {
                Member::decode(&member).map(|member| Binding {
                    share_id,
                    family,
                    member,
                })
            })
            .collect(),
    }
}

macro_rules! load_shares {
    ($exec:expr, $owner:expr) => {{
        let rows: Vec<ShareRow> = sqlx::query_as("SELECT share_id,browser_key,label,created_ms,revision,deleting_ms FROM provider_browser_shares WHERE owner_user_id=$1 ORDER BY created_ms,share_id")
            .bind($owner).fetch_all($exec).await?;
        rows
    }};
}

macro_rules! load_bindings {
    ($exec:expr, $owner:expr) => {{
        let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT share_id,family,member FROM provider_browser_share_bindings WHERE owner_user_id=$1 ORDER BY share_id,family")
            .bind($owner).fetch_all($exec).await?;
        rows
    }};
}

macro_rules! write_bindings {
    ($tx:expr, $owner:expr, $shares:expr) => {{
        sqlx::query("DELETE FROM provider_browser_share_bindings WHERE owner_user_id=$1").bind($owner).execute(&mut *$tx).await?;
        for row in &$shares.bindings {
            sqlx::query("INSERT INTO provider_browser_share_bindings (owner_user_id,share_id,family,member) VALUES ($1,$2,$3,$4)")
                .bind($owner).bind(&row.share_id).bind(&row.family).bind(row.member.encode()).execute(&mut *$tx).await?;
        }
    }};
}

macro_rules! load_members {
    ($exec:expr, $owner:expr) => {{
        let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT member,share_id,family FROM provider_browser_share_members WHERE owner_user_id=$1 ORDER BY member")
            .bind($owner).fetch_all($exec).await?;
        rows
    }};
}

impl Store {
    /// The owner's shares, memberships and family bindings.
    pub async fn browser_shares(&self, owner: &str) -> Result<Shares> {
        macro_rules! read {
            ($db:expr) => {
                decode(
                    owner,
                    load_shares!(&$db.pool, owner),
                    load_members!(&$db.pool, owner),
                    load_bindings!(&$db.pool, owner),
                )
            };
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }

    /// Apply a share change through the account registry writer: the owner
    /// lock, the registry revision (compare and set) and the operation
    /// receipt, exactly like every other account change. `families` maps
    /// Provider ids to their account-usage family.
    pub async fn mutate_browser_shares(
        &self,
        owner: &str,
        operation: &str,
        input: &Mutation,
        families: &BTreeMap<String, String>,
    ) -> Result<MutationResult> {
        if input.expected_revision < 0
            || !crate::managed_calls::valid_id(operation)
            || !input.change.is_browser_share()
        {
            return Ok(MutationResult::Invalid("invalid_argument"));
        }
        let hash = crate::digest::hex_sha256(&serde_json::to_vec(input)?);
        macro_rules! write { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("INSERT INTO provider_account_registries (owner_user_id,revision,value) VALUES ($1,0,$2) ON CONFLICT (owner_user_id) DO UPDATE SET revision=provider_account_registries.revision")
                .bind(owner).bind(serde_json::to_string(&Registry::default())?).execute(&mut *tx).await?;
            let old: Option<(String,String)> = sqlx::query_as("SELECT request_hash,result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
            if let Some((old_hash, result)) = old {
                return Ok(if old_hash == hash { MutationResult::Applied(serde_json::from_str(&result)?) }
                    else { MutationResult::Invalid("operation_reused") });
            }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            if registry.revision != input.expected_revision { return Ok(MutationResult::Conflict(registry)); }
            let mut shares = decode(owner, load_shares!(&mut *tx, owner), load_members!(&mut *tx, owner), load_bindings!(&mut *tx, owner));
            let placed: Vec<String> = sqlx::query_scalar("SELECT DISTINCT account_id FROM browser_placements WHERE owner_user_id=$1")
                .bind(owner).fetch_all(&mut *tx).await?;
            let placed: BTreeSet<ProviderAccountId> = placed.into_iter().filter_map(|key| key.try_into().ok()).collect();
            let context = ShareContext { owner, registry: &registry, families, placed: &placed,
                now_ms: now_ms(), new_share_id: uuid::Uuid::new_v4().to_string() };
            let applied = match apply(&mut shares, &input.change, &context) {
                Ok(applied) => applied, Err(code) => return Ok(MutationResult::Invalid(code)),
            };
            registry.revision = match registry.revision.checked_add(1) {
                Some(revision) => revision, None => return Ok(MutationResult::Invalid("revision_exhausted")),
            };
            sqlx::query("DELETE FROM provider_browser_share_members WHERE owner_user_id=$1").bind(owner).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM provider_browser_shares WHERE owner_user_id=$1").bind(owner).execute(&mut *tx).await?;
            for share in &shares.shares {
                sqlx::query("INSERT INTO provider_browser_shares (owner_user_id,share_id,browser_key,label,created_ms,revision,deleting_ms) VALUES ($1,$2,$3,$4,$5,$6,$7)")
                    .bind(owner).bind(&share.share_id).bind(share.browser_key.as_str()).bind(&share.label)
                    .bind(share.created_ms).bind(share.revision).bind(share.deleting_ms).execute(&mut *tx).await?;
            }
            // The unique (owner, share, family) and (owner, member) keys
            // enforce the family rule here too.
            for row in &shares.members {
                sqlx::query("INSERT INTO provider_browser_share_members (owner_user_id,member,share_id,family) VALUES ($1,$2,$3,$4)")
                    .bind(owner).bind(row.member.encode()).bind(&row.share_id).bind(&row.family).execute(&mut *tx).await?;
            }
            write_bindings!(tx, owner, shares);
            sqlx::query("UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3")
                .bind(registry.revision).bind(serde_json::to_string(&registry)?).bind(owner).execute(&mut *tx).await?;
            let receipt = Receipt { registry, account_id: applied.subject, browser_share_id: applied.share_id, ..Receipt::default() };
            sqlx::query("INSERT INTO provider_account_operations (owner_user_id,operation_id,request_hash,result) VALUES ($1,$2,$3,$4)")
                .bind(owner).bind(operation).bind(&hash).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            MutationResult::Applied(receipt)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        })
    }

    /// Commit the share's placement before an open from `member` is
    /// dispatched, under the owner lock: the share still exists and is not
    /// being deleted, `member` is still in it, and the member's own browser
    /// is still admitted (an available account with its browser enabled, or
    /// an enabled legacy card). The share itself has no admission.
    pub async fn admit_shared_browser_placement(
        &self,
        owner: &str,
        (member, share_id): (&Member, &str),
        machine_id: &str,
        instance_id: &str,
        now_ms: i64,
    ) -> Result<std::result::Result<(), Refusal>> {
        let key = share_key(owner, share_id);
        macro_rules! admit { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            if locked.rows_affected() != 1 {
                return Ok(Err(Refusal::ShareChanged));
            }
            let shares = decode(owner, load_shares!(&mut *tx, owner), load_members!(&mut *tx, owner), load_bindings!(&mut *tx, owner));
            let current = shares.share(share_id).is_some_and(|share| share.deleting_ms.is_none())
                && shares.membership(member).is_some_and(|row| row.share_id == share_id);
            if !current || !families_distinct(&shares.members_of(share_id)) {
                return Ok(Err(Refusal::ShareChanged));
            }
            // One placement per share: a concurrent first open elsewhere won.
            let elsewhere: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 AND machine_id<>$3")
                .bind(owner).bind(key.as_str()).bind(machine_id).fetch_one(&mut *tx).await?;
            if elsewhere > 0 {
                return Ok(Err(Refusal::ShareElsewhere));
            }
            match member {
                Member::Account(member) => {
                    let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                        .bind(owner).fetch_one(&mut *tx).await?;
                    let registry: Registry = serde_json::from_str(&value)?;
                    match registry.accounts.iter().find(|entry| entry.id == member.account_id) {
                        Some(entry) if !entry.available() => return Ok(Err(Refusal::AccountUnavailable)),
                        Some(entry) if !entry.browser_enabled => return Ok(Err(Refusal::BrowserNotEnabled)),
                        Some(_) => {}
                        None => return Ok(Err(Refusal::AccountUnavailable)),
                    }
                }
                Member::Card(member) => {
                    let enabled: Option<String> = sqlx::query_scalar("SELECT card_id FROM provider_legacy_browsers WHERE owner_user_id=$1 AND card_id=$2")
                        .bind(owner).bind(&member.card).fetch_optional(&mut *tx).await?;
                    if enabled.is_none() {
                        return Ok(Err(Refusal::BrowserNotEnabled));
                    }
                }
            }
            sqlx::query("INSERT INTO browser_placements (owner_user_id,account_id,machine_id,instance_id,profile_revision,last_open_ms) VALUES ($1,$2,$3,$4,0,$5) ON CONFLICT (owner_user_id,account_id,machine_id) DO UPDATE SET instance_id=excluded.instance_id,last_open_ms=excluded.last_open_ms")
                .bind(owner).bind(key.as_str()).bind(machine_id).bind(instance_id).bind(now_ms)
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }} }
        match &self.backend {
            StorageBackend::Postgres(db) => admit!(db),
            StorageBackend::Sqlite(db) => admit!(db),
        }
        Ok(Ok(()))
    }

    /// Record that `machine_id` confirmed the revocation of a deleted
    /// share's profile: its placement there is removed, and the share is
    /// forgotten once no placement remains. Returns whether it is gone.
    /// Only a share being deleted is affected, under the owner lock, while
    /// the reporting connection is still current.
    pub async fn record_browser_share_revocation(
        &self,
        owner: &str,
        share_id: &str,
        machine_id: &str,
        connection_is_current: impl FnOnce() -> bool,
    ) -> Result<bool> {
        let key = share_key(owner, share_id);
        macro_rules! record { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let deleting: Option<Option<i64>> = sqlx::query_scalar("SELECT deleting_ms FROM provider_browser_shares WHERE owner_user_id=$1 AND share_id=$2")
                .bind(owner).bind(share_id).fetch_optional(&mut *tx).await?;
            let Some(Some(_)) = deleting else {
                anyhow::ensure!(deleting.is_none(), "browser_share_not_deleting");
                return Ok(true);
            };
            anyhow::ensure!(connection_is_current(), "machine_connection_changed");
            sqlx::query("DELETE FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3")
                .bind(owner).bind(key.as_str()).bind(machine_id).execute(&mut *tx).await?;
            let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2")
                .bind(owner).bind(key.as_str()).fetch_one(&mut *tx).await?;
            if remaining == 0 {
                sqlx::query("DELETE FROM provider_browser_shares WHERE owner_user_id=$1 AND share_id=$2")
                    .bind(owner).bind(share_id).execute(&mut *tx).await?;
                sqlx::query("DELETE FROM provider_browser_share_bindings WHERE owner_user_id=$1 AND share_id=$2")
                    .bind(owner).bind(share_id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
            remaining == 0
        }} }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => record!(db),
            StorageBackend::Sqlite(db) => record!(db),
        })
    }
}

impl Store {
    /// The Machine confirmed a reset of the share's profile on `machine_id`,
    /// its only placement: the profile is fresh, so the family bindings of
    /// members that left are forgotten (current members stay bound). Nothing
    /// changes while the share has a placement on another Machine or is
    /// being deleted. Returns whether the bindings were renewed.
    pub async fn renew_browser_share_bindings(
        &self,
        owner: &str,
        share_id: &str,
        machine_id: &str,
    ) -> Result<bool> {
        let key = share_key(owner, share_id);
        macro_rules! renew { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            let mut shares = decode(owner, load_shares!(&mut *tx, owner), load_members!(&mut *tx, owner), load_bindings!(&mut *tx, owner));
            let live = shares.share(share_id).is_some_and(|share| share.deleting_ms.is_none());
            let elsewhere: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM browser_placements WHERE owner_user_id=$1 AND account_id=$2 AND machine_id<>$3")
                .bind(owner).bind(key.as_str()).bind(machine_id).fetch_one(&mut *tx).await?;
            if !live || elsewhere > 0 {
                return Ok(false);
            }
            shares.rebind_current(share_id);
            write_bindings!(tx, owner, shares);
            tx.commit().await?;
            true
        }} }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => renew!(db),
            StorageBackend::Sqlite(db) => renew!(db),
        })
    }

    /// Every share being deleted, across owners: the deletion reconciler's
    /// work after a Controller restart.
    pub async fn deleting_browser_shares(&self) -> Result<Vec<(String, String)>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_as::<_, (String, String)>("SELECT owner_user_id,share_id FROM provider_browser_shares WHERE deleting_ms IS NOT NULL ORDER BY owner_user_id,share_id")
                .fetch_all(&$db.pool).await?
        }; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
#[path = "browser_shares_tests.rs"]
mod tests;
