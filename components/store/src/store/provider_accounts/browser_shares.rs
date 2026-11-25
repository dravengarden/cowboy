//! Owner-defined shared Machine browser profiles. A share lets several
//! browser identities (named accounts) use one browser profile, so an owner signs in to a common identity provider
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
use super::{Change, Mutation, MutationResult, Receipt, Registry, label_valid};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use std::collections::BTreeSet;

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

/// A browser identity that may join a share: `{account_id}` of a named
/// account. The retired legacy cards' `{card}` members were removed by
/// migration; a binding a card left stays unreadable and blocks its family
/// until a confirmed reset (`docs/legacy-card-retirement.md`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Member {
    Account(AccountMember),
}

impl Member {
    pub fn account(account_id: ProviderAccountId) -> Self {
        Self::Account(AccountMember { account_id })
    }

    /// The stored form: `account:<id>`.
    pub fn encode(&self) -> String {
        match self {
            Self::Account(member) => format!("account:{}", member.account_id.as_str()),
        }
    }

    /// `None` for anything else, including a retired `card:<card>`.
    pub fn decode(value: &str) -> Option<Self> {
        ProviderAccountId::try_from(value.strip_prefix("account:")?.to_owned())
            .ok()
            .filter(|id| !id.is_default())
            .map(Self::account)
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Account(member) => !member.account_id.is_default(),
        }
    }

    /// The member's own browser key, used whenever it is in no share.
    pub fn own_key(&self, _owner: &str) -> ProviderAccountId {
        match self {
            Self::Account(member) => member.account_id.clone(),
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
    /// A reset of its profile dispatched but not yet answered.
    pub pending_reset: Option<PendingReset>,
}

/// A reset of a share's profile whose terminal outcome is not yet known: a
/// durable fence, so no membership change of the share lands while the
/// Machine may still delete the profile (also across a Controller restart).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingReset {
    pub operation_id: String,
    pub machine_id: String,
    pub expected_profile_revision: u64,
    pub started_ms: i64,
    /// Bindings of identities that had left the share when the reset began
    /// (encoded members): a confirmed reset may release only these.
    pub released: Vec<String>,
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

/// A stored binding whose member cannot be decoded. It is kept verbatim
/// (never dropped by a rewrite): the profile may still hold that identity's
/// cookies, so no member of its family joins and the share's browser is not
/// admitted anywhere until a confirmed reset releases it or the share is
/// deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableBinding {
    pub share_id: String,
    pub family: String,
    pub member: String,
}

impl UnreadableBinding {
    /// The retired legacy card this binding belongs to: its member is
    /// `card:<card>` of its own family. Such a binding is a known, read-only
    /// identity (no other family's cookies), not an unknown one.
    pub fn retired_card(&self) -> Option<&str> {
        self.member
            .strip_prefix("card:")
            .filter(|card| *card == self.family && super::legacy_cards::CARDS.contains(card))
    }
}

/// One owner's shares, memberships and family bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shares {
    pub shares: Vec<Share>,
    pub members: Vec<ShareMember>,
    pub bindings: Vec<Binding>,
    pub unreadable_bindings: Vec<UnreadableBinding>,
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

    /// The profile is fresh: release the bindings of `released` identities
    /// (they had left before the reset began, so they cannot have used the
    /// wiped profile since) and bind each current member under its family
    /// now (`current`). Other bindings stay. Refused (nothing changes) when a
    /// member's family is unknown or two members would share one; such a
    /// share is only ever deleted.
    pub fn renew_current(
        &mut self,
        share_id: &str,
        released: &[Member],
        current: impl Fn(&Member) -> Option<String>,
    ) -> bool {
        let mut renewed = Vec::new();
        for row in self.members_of(share_id) {
            let Some(family) = current(&row.member) else {
                return false;
            };
            if renewed
                .iter()
                .any(|(_, known): &(Member, String)| known == &family)
            {
                return false;
            }
            renewed.push((row.member.clone(), family));
        }
        for row in &mut self.members {
            if let Some((_, family)) = renewed.iter().find(|(member, _)| member == &row.member) {
                family.clone_into(&mut row.family);
            }
        }
        self.bindings.retain(|row| {
            row.share_id != share_id
                || !(released.contains(&row.member)
                    || renewed.iter().any(|(member, _)| member == &row.member))
        });
        // The confirmed reset wiped whatever an unreadable binding stood for.
        self.unreadable_bindings
            .retain(|row| row.share_id != share_id);
        self.bindings
            .extend(renewed.into_iter().map(|(member, family)| Binding {
                share_id: share_id.to_owned(),
                family,
                member,
            }));
        true
    }

    fn live(&self, share_id: &str) -> Result<&Share, &'static str> {
        self.share(share_id)
            .filter(|share| share.deleting_ms.is_none())
            .ok_or("browser_share_not_found")
    }

    /// Membership of `share_id` is frozen while a reset of its profile may
    /// still run on its Machine.
    fn settled(&self, share_id: &str) -> Result<(), &'static str> {
        if self
            .share(share_id)
            .is_some_and(|share| share.pending_reset.is_some())
        {
            return Err("browser_share_reset_pending");
        }
        Ok(())
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

/// The family of a member that may join a share, from the static table
/// (`browser_families`): a named account by its Provider. A member without
/// one, or a revoking account, never joins.
pub fn member_family(member: &Member, registry: &Registry) -> Result<String, &'static str> {
    let Member::Account(account) = member;
    let account = registry
        .accounts
        .iter()
        .find(|candidate| candidate.id == account.account_id)
        .ok_or("account_not_found")?;
    if account.revoking {
        return Err("account_revoking");
    }
    family_of(member, registry).ok_or("browser_share_family_unknown")
}

/// The family an identity has in the static table, whatever its state (a
/// revoked account still has one). `None` outside the table.
pub fn family_of(member: &Member, registry: &Registry) -> Option<String> {
    match member {
        Member::Account(member) => registry
            .accounts
            .iter()
            .find(|account| account.id == member.account_id)
            .and_then(|account| super::browser_families::provider(&account.provider_id)),
    }
    .map(str::to_owned)
}

/// Why `member` (of `family`) may not use share `share_id`'s profile
/// because of its family bindings, if it may not: a binding whose recorded
/// family no longer matches the table (only a reviewed code change can do
/// that) blocks everyone until a confirmed reset, and a binding of another
/// identity whose family is `family` blocks this member.
pub fn binding_refusal(
    shares: &Shares,
    share_id: &str,
    (member, family): (&Member, &str),
    current: impl Fn(&Member) -> Option<String>,
) -> Option<&'static str> {
    for row in shares
        .bindings
        .iter()
        .filter(|row| row.share_id == share_id)
    {
        let now = current(&row.member);
        if now.as_deref() != Some(row.family.as_str()) {
            return Some("browser_share_family_drift");
        }
        if &row.member != member && (row.family == family || now.as_deref() == Some(family)) {
            return Some("browser_share_family_bound");
        }
    }
    None
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
    shares.settled(share_id)?;
    if let Some(row) = shares.membership(member) {
        shares.settled(&row.share_id)?;
    }
    let family = member_family(member, ctx.registry)?;
    if shares
        .membership(member)
        .is_some_and(|row| row.share_id == share_id && row.family == family)
    {
        return Ok(());
    }
    // An unreadable binding of this family may be any other identity's.
    if shares
        .unreadable_bindings
        .iter()
        .any(|row| row.share_id == share_id && row.family == family)
    {
        return Err("browser_share_family_bound");
    }
    if shares
        .members_of(share_id)
        .iter()
        .any(|row| &row.member != member && row.family == family)
    {
        return Err("browser_share_family_conflict");
    }
    // The profile may still hold an earlier member's cookies of this family.
    if let Some(code) = binding_refusal(shares, share_id, (member, &family), |other| {
        family_of(other, ctx.registry)
    }) {
        return Err(code);
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
                pending_reset: None,
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
            shares
                .unreadable_bindings
                .retain(|row| &row.share_id != share_id);
            // Deletion revokes the profile anyway; a pending reset is moot.
            if let Some(share) = shares
                .shares
                .iter_mut()
                .find(|share| &share.share_id == share_id)
            {
                share.pending_reset = None;
            }
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
            set_share(shares, member, share_id.as_deref(), ctx)
        }
        _ => Err("invalid_change"),
    }
}

/// Put `member` into share `share_id`, or take it out (`None`).
fn set_share(
    shares: &mut Shares,
    member: &Member,
    share_id: Option<&str>,
    ctx: &ShareContext<'_>,
) -> Result<Applied, &'static str> {
    if !member.valid() {
        return Err("invalid_argument");
    }
    // Only the owner's own identities, even to leave (a no-op too).
    let Member::Account(account) = member;
    if !ctx
        .registry
        .accounts
        .iter()
        .any(|entry| entry.id == account.account_id)
    {
        return Err("account_not_found");
    }
    if let Some(share_id) = share_id {
        join(shares, member, share_id, ctx)?;
    } else {
        if let Some(row) = shares.membership(member) {
            shares.settled(&row.share_id)?;
        }
        leave(shares, member);
    }
    Ok(Applied {
        subject: member.own_key(ctx.owner),
        share_id: share_id.map(str::to_owned),
    })
}

type ShareRow = (
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
);

fn decode(
    owner: &str,
    rows: Vec<ShareRow>,
    members: Vec<(String, String, String)>,
    bindings: Vec<(String, String, String)>,
) -> Shares {
    let (mut readable, mut unreadable_bindings) = (Vec::new(), Vec::new());
    for (share_id, family, member) in bindings {
        match Member::decode(&member) {
            Some(member) => readable.push(Binding {
                share_id,
                family,
                member,
            }),
            None => unreadable_bindings.push(UnreadableBinding {
                share_id,
                family,
                member,
            }),
        }
    }
    let bindings = readable;
    Shares {
        shares: rows
            .into_iter()
            .filter(|(share_id, ..)| share_id_valid(share_id))
            .map(
                |(share_id, _, label, created_ms, revision, deleting_ms, pending)| Share {
                    // Derived again: the stored key is a lookup index only.
                    browser_key: share_key(owner, &share_id),
                    share_id,
                    label,
                    created_ms,
                    revision,
                    deleting_ms,
                    // An unreadable fence stays a fence.
                    pending_reset: pending.map(|value| {
                        serde_json::from_str(&value).unwrap_or(PendingReset {
                            operation_id: String::new(),
                            machine_id: String::new(),
                            expected_profile_revision: 0,
                            started_ms: 0,
                            released: Vec::new(),
                        })
                    }),
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
        bindings,
        unreadable_bindings,
    }
}

macro_rules! load_shares {
    ($exec:expr, $owner:expr) => {{
        let rows: Vec<ShareRow> = sqlx::query_as("SELECT share_id,browser_key,label,created_ms,revision,deleting_ms,pending_reset FROM provider_browser_shares WHERE owner_user_id=$1 ORDER BY created_ms,share_id")
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
        // Kept verbatim: a rewrite never forgets an identity it cannot read.
        for row in &$shares.unreadable_bindings {
            sqlx::query("INSERT INTO provider_browser_share_bindings (owner_user_id,share_id,family,member) VALUES ($1,$2,$3,$4)")
                .bind($owner).bind(&row.share_id).bind(&row.family).bind(&row.member).execute(&mut *$tx).await?;
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
    /// receipt, exactly like every other account change. Families come from
    /// the static table (`browser_families`), never the Provider catalog.
    pub async fn mutate_browser_shares(
        &self,
        owner: &str,
        operation: &str,
        input: &Mutation,
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
            let context = ShareContext { owner, registry: &registry, placed: &placed,
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
                sqlx::query("INSERT INTO provider_browser_shares (owner_user_id,share_id,browser_key,label,created_ms,revision,deleting_ms,pending_reset) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                    .bind(owner).bind(&share.share_id).bind(share.browser_key.as_str()).bind(&share.label)
                    .bind(share.created_ms).bind(share.revision).bind(share.deleting_ms)
                    .bind(share.pending_reset.as_ref().map(serde_json::to_string).transpose()?).execute(&mut *tx).await?;
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
        connection_is_current: impl FnOnce() -> bool + Send,
    ) -> Result<bool> {
        let mut check = Some(connection_is_current);
        let mut run = || check.take().expect("connection_is_current invoked once")();
        self.record_browser_share_revocation_inner(owner, share_id, machine_id, &mut run)
            .await
    }

    async fn record_browser_share_revocation_inner(
        &self,
        owner: &str,
        share_id: &str,
        machine_id: &str,
        connection_is_current: &mut (dyn FnMut() -> bool + Send),
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
    /// Every share being deleted, across owners: the deletion reconciler's
    /// work after a Controller restart.
    /// Whether `encoded_member` (`account:<id>`) still holds a row in
    /// `provider_browser_share_members` for `owner`.
    pub async fn is_browser_share_member(&self, owner: &str, encoded_member: &str) -> Result<bool> {
        macro_rules! check { ($db:expr) => {{
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM provider_browser_share_members WHERE owner_user_id=$1 AND member=$2",
            )
            .bind(owner)
            .bind(encoded_member)
            .fetch_one(&$db.pool)
            .await?;
            count > 0
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => check!(db),
            StorageBackend::Sqlite(db) => check!(db),
        })
    }

    /// Revoking named accounts whose share membership removal was deferred while
    /// an identity-bound navigation was unsettled: `(owner, account_id, share_browser_key)`.
    pub async fn deferred_revoked_share_members(
        &self,
        owner: Option<&str>,
    ) -> Result<Vec<(String, ProviderAccountId, ProviderAccountId)>> {
        macro_rules! list { ($db:expr) => {{
            if let Some(owner) = owner {
                sqlx::query_as::<_, (String, String, String)>(
                    "SELECT m.owner_user_id, m.member, s.browser_key                      FROM provider_browser_share_members m                      JOIN provider_browser_shares s                        ON s.owner_user_id=m.owner_user_id AND s.share_id=m.share_id                      WHERE m.owner_user_id=$1                      ORDER BY m.owner_user_id, m.member",
                )
                .bind(owner)
                .fetch_all(&$db.pool)
                .await?
            } else {
                sqlx::query_as::<_, (String, String, String)>(
                    "SELECT m.owner_user_id, m.member, s.browser_key                      FROM provider_browser_share_members m                      JOIN provider_browser_shares s                        ON s.owner_user_id=m.owner_user_id AND s.share_id=m.share_id                      ORDER BY m.owner_user_id, m.member",
                )
                .fetch_all(&$db.pool)
                .await?
            }
        }}; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => list!(db),
            StorageBackend::Sqlite(db) => list!(db),
        };
        let mut registries: std::collections::HashMap<String, Registry> =
            std::collections::HashMap::new();
        let mut out = Vec::new();
        for (row_owner, encoded_member, key_str) in rows {
            let Some(Member::Account(account_member)) = Member::decode(&encoded_member) else {
                continue;
            };
            let Ok(browser_key) = ProviderAccountId::try_from(key_str) else {
                continue;
            };
            if !registries.contains_key(&row_owner) {
                registries.insert(row_owner.clone(), self.provider_accounts(&row_owner).await?);
            }
            if registries.get(&row_owner).is_some_and(|registry| {
                registry
                    .accounts
                    .iter()
                    .any(|account| account.id == account_member.account_id && account.revoking)
            }) {
                out.push((row_owner, account_member.account_id, browser_key));
            }
        }
        Ok(out)
    }

    /// Complete a deferred share membership removal for a revoking account once
    /// no navigation is pending on that share. Bumps the share's membership
    /// revision and the owner's registry revision, returning the new registry
    /// revision when a membership row was removed.
    pub async fn finish_revoked_share_leave(
        &self,
        owner: &str,
        account_id: &ProviderAccountId,
    ) -> Result<Option<i64>> {
        let encoded = Member::account(account_id.clone()).encode();
        macro_rules! finish { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query(
                "UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1",
            )
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            if locked.rows_affected() != 1 {
                return Ok(None);
            }
            let value: String = sqlx::query_scalar(
                "SELECT value FROM provider_account_registries WHERE owner_user_id=$1",
            )
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
            let mut registry: Registry = serde_json::from_str(&value)?;
            if !registry
                .accounts
                .iter()
                .any(|account| account.id == *account_id && account.revoking)
            {
                return Ok(None);
            }
            sqlx::query(
                "UPDATE provider_browser_shares SET revision=revision+1                  WHERE owner_user_id=$1 AND share_id IN (                   SELECT share_id FROM provider_browser_share_members                    WHERE owner_user_id=$1 AND member=$2                 )",
            )
            .bind(owner)
            .bind(&encoded)
            .execute(&mut *tx)
            .await?;
            let deleted = sqlx::query(
                "DELETE FROM provider_browser_share_members WHERE owner_user_id=$1 AND member=$2",
            )
            .bind(owner)
            .bind(&encoded)
            .execute(&mut *tx)
            .await?;
            let ops: Vec<(String, String)> = sqlx::query_as(
                "SELECT operation_id, result FROM provider_account_operations WHERE owner_user_id=$1",
            )
            .bind(owner)
            .fetch_all(&mut *tx)
            .await?;
            let mut cleared_flag = false;
            for (op_id, raw) in ops {
                let Ok(mut receipt) = serde_json::from_str::<Receipt>(&raw) else {
                    continue;
                };
                if receipt.account_id == *account_id
                    && let Some(plan) = receipt.revocation.as_mut()
                    && plan.browser_share_leave_pending
                {
                    plan.browser_share_leave_pending = false;
                    sqlx::query(
                        "UPDATE provider_account_operations SET result=$3                          WHERE owner_user_id=$1 AND operation_id=$2",
                    )
                    .bind(owner)
                    .bind(&op_id)
                    .bind(serde_json::to_string(&receipt)?)
                    .execute(&mut *tx)
                    .await?;
                    cleared_flag = true;
                }
            }
            if deleted.rows_affected() == 0 {
                if cleared_flag {
                    tx.commit().await?;
                }
                return Ok(None);
            }
            registry.revision = registry
                .revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("revision_exhausted"))?;
            sqlx::query(
                "UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3",
            )
            .bind(registry.revision)
            .bind(serde_json::to_string(&registry)?)
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Some(registry.revision)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => finish!(db),
            StorageBackend::Sqlite(db) => finish!(db),
        })
    }

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

#[path = "browser_share_resets.rs"]
mod resets;

#[cfg(test)]
#[path = "browser_shares_tests.rs"]
mod tests;
