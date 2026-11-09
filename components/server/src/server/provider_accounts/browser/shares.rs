//! Owner-defined shared Machine browser profiles: which browser key a
//! member's card uses, the account-list projection, the effects of share
//! changes and the revocation of a deleted share's profile.
//!
//! Every browser path resolves the key here (`resolve_key`), so a member of
//! a share opens, reads, attaches to, resizes, navigates, hibernates and
//! resets the share's single browser, while a non-member keeps its own key.
//! Authorization stays per member: the share has no admission of its own.
//! See `docs/machine-browser.md`, "Shared browser profiles".

use super::super::super::*;
use super::BrowserAccount;
use crate::machine_protocol::MACHINE_BROWSER_PROTOCOL_VERSION;
use crate::machine_protocol::machine_browser::{Command, Reason, State as BrowserState};
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::browser_shares::{self, Member, Shares, families_distinct};
use crate::store::provider_accounts::{Change, Mutation, MutationResult, Receipt, Registry};
use serde_json::{Value, json};

const REVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The account-usage family of each catalogued Provider. A Provider whose
/// catalog entries disagree has none, so it never joins a share.
pub(in crate::server) fn families(state: &AppState) -> BTreeMap<String, String> {
    family_map(state.provider_catalog.entries().into_iter().map(|entry| {
        let family = entry
            .manifest
            .host
            .account_usage
            .as_ref()
            .map(|usage| usage.provider.clone());
        (entry.provider_id, family)
    }))
}

fn family_map(
    entries: impl IntoIterator<Item = (String, Option<String>)>,
) -> BTreeMap<String, String> {
    let mut seen: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (provider, family) in entries {
        let Some(family) = family else { continue };
        seen.entry(provider)
            .and_modify(|known| {
                if known.as_ref() != Some(&family) {
                    *known = None;
                }
            })
            .or_insert(Some(family));
    }
    seen.into_iter()
        .filter_map(|(provider, family)| family.map(|family| (provider, family)))
        .collect()
}

/// The share a member uses, as the browser routes report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) struct ShareView {
    pub share_id: String,
    pub label: String,
    pub members: Vec<browser_shares::ShareMember>,
}

impl ShareView {
    pub(in crate::server) fn value(&self, owner: &str) -> Value {
        json!({"share_id": self.share_id, "label": self.label,
            "members": self.members.iter().map(|row| member_value(owner, row)).collect::<Vec<_>>()})
    }
}

fn member_value(owner: &str, row: &browser_shares::ShareMember) -> Value {
    json!({"member": row.member, "family": row.family, "account_id": row.member.own_key(owner)})
}

/// The member identity of a browser account; a key known only from a
/// retained placement has none and keeps its own key.
pub(in crate::server) fn member_of(
    account_id: &ProviderAccountId,
    account: &BrowserAccount,
) -> Option<Member> {
    match account {
        BrowserAccount::Named(_) => Some(Member::account(account_id.clone())),
        BrowserAccount::Legacy(card) => card.card.as_deref().map(Member::card),
    }
}

/// The current family of a member: its card, or its Provider's family.
fn family_now(
    member: &Member,
    registry: &Registry,
    families: &BTreeMap<String, String>,
) -> Option<String> {
    browser_shares::member_family(member, registry, families).ok()
}

/// The browser key `member` uses and the share behind it. A member in no
/// share keeps `own`. Before a share key is used, every member's family is
/// checked again against the current catalog: a share that would hold two
/// members of one family, or a member whose family is no longer known, is
/// refused rather than shared.
pub(in crate::server) fn resolve_key(
    member: Option<&Member>,
    own: &ProviderAccountId,
    shares: &Shares,
    current_family: impl Fn(&Member) -> Option<String>,
) -> Result<(ProviderAccountId, Option<ShareView>), &'static str> {
    let Some(row) = member.and_then(|member| shares.membership(member)) else {
        return Ok((own.clone(), None));
    };
    let share = shares
        .share(&row.share_id)
        .filter(|share| share.deleting_ms.is_none())
        .ok_or("browser_share_changed")?;
    let members = shares.members_of(&share.share_id);
    if !families_distinct(&members) {
        return Err("browser_share_family_conflict");
    }
    for member in &members {
        match current_family(&member.member) {
            Some(family) if family == member.family => {}
            Some(_) => return Err("browser_share_family_conflict"),
            None => return Err("browser_share_family_unknown"),
        }
    }
    Ok((
        share.browser_key.clone(),
        Some(ShareView {
            share_id: share.share_id.clone(),
            label: share.label.clone(),
            members: members.into_iter().cloned().collect(),
        }),
    ))
}

/// `resolve_key` against the owner's stored shares and current catalog.
pub(in crate::server) async fn resolve(
    state: &AppState,
    owner: &str,
    (account_id, account): (&ProviderAccountId, &BrowserAccount),
    registry: &Registry,
) -> Result<(ProviderAccountId, Option<ShareView>), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let shares = store
        .browser_shares(owner)
        .await
        .map_err(|_| "unavailable")?;
    let families = families(state);
    resolve_key(
        member_of(account_id, account).as_ref(),
        account_id,
        &shares,
        |member| family_now(member, registry, &families),
    )
}

/// Whether `id` is one of the owner's share keys. Share keys are never
/// addressed directly: a member's card names its own identity.
pub(in crate::server) async fn is_share_key(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<bool, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    Ok(store
        .browser_shares(owner)
        .await
        .map_err(|_| "unavailable")?
        .by_key(id)
        .is_some())
}

/// The account list's share projection.
pub(in crate::server) async fn list_value(
    state: &AppState,
    owner: &str,
) -> Result<(Value, Value), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let shares = store
        .browser_shares(owner)
        .await
        .map_err(|_| "unavailable")?;
    let listed: Vec<Value> = shares
        .shares
        .iter()
        .map(|share| {
            json!({"share_id": share.share_id, "label": share.label, "created_ms": share.created_ms,
                "revision": share.revision, "deleting": share.deleting_ms.is_some(),
                "members": shares.members_of(&share.share_id).into_iter()
                    .map(|row| member_value(owner, row)).collect::<Vec<_>>()})
        })
        .collect();
    Ok((Value::Array(listed), json!(families(state))))
}

/// What an applied account change does to running browsers and viewers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) enum Effect {
    /// End every viewer the member's card attached, on its own browser or
    /// on a share's.
    EndMemberViewers(ProviderAccountId),
    /// Hibernate the member's own browser key; never a share's.
    HibernateOwn(ProviderAccountId, &'static str),
    /// End every viewer of a deleted share's browser.
    EndKeyViewers(ProviderAccountId),
    /// Revoke (delete) the deleted share's profile on every Machine.
    RevokeShare(String),
}

/// The effects of an applied change. A member leaving, disabling its
/// browser or being revoked never resets, revokes or hibernates the share's
/// browser: other members depend on it. Only deleting the share revokes it.
pub(in crate::server) fn change_effects(
    owner: &str,
    change: &Change,
    receipt: &Receipt,
) -> Vec<Effect> {
    match change {
        Change::BeginRevocation { id } => vec![Effect::EndMemberViewers(id.clone())],
        Change::SetBrowserEnabled { id, enabled: false }
        | Change::SetEnabled { id, enabled: false } => vec![
            Effect::EndMemberViewers(id.clone()),
            Effect::HibernateOwn(id.clone(), "browser_disabled"),
        ],
        Change::SetLegacyBrowserEnabled { enabled: false, .. } => vec![
            Effect::EndMemberViewers(receipt.account_id.clone()),
            Effect::HibernateOwn(receipt.account_id.clone(), "browser_disabled"),
        ],
        // Joining: the member's own browser is no longer reachable from its
        // card. Leaving: its viewer of the share ends.
        Change::SetBrowserShare { member, share_id } => {
            let own = member.own_key(owner);
            let mut effects = vec![Effect::EndMemberViewers(own.clone())];
            if share_id.is_some() {
                effects.push(Effect::HibernateOwn(own, "browser_share_changed"));
            }
            effects
        }
        Change::DeleteBrowserShare { share_id } => vec![
            Effect::EndKeyViewers(browser_shares::share_key(owner, share_id)),
            Effect::RevokeShare(share_id.clone()),
        ],
        _ => Vec::new(),
    }
}

/// Carry out the effects of an applied change.
pub(in crate::server) fn run_effects(state: &Arc<AppState>, owner: &str, effects: Vec<Effect>) {
    for effect in effects {
        match effect {
            Effect::EndMemberViewers(member) => {
                state.browsers.cancel_member_viewers(member.as_str())
            }
            Effect::EndKeyViewers(key) => state.browsers.cancel_account_viewers(key.as_str()),
            Effect::HibernateOwn(account, cause) => {
                let state = Arc::clone(state);
                tokio::spawn(async move {
                    if let Err(error) = super::hibernate_running(&state, &account, cause).await {
                        tracing::info!(%error, "browser hibernate after an account change did not complete");
                    }
                });
            }
            Effect::RevokeShare(share_id) => {
                let (state, owner) = (Arc::clone(state), owner.to_owned());
                tokio::spawn(async move {
                    revoke_deleted(&state, &owner, &share_id).await;
                });
            }
        }
    }
}

/// Apply a share change through the registry writer. Joining a legacy card
/// needs a card the Service collects now (a replay is always answered).
pub(in crate::server) async fn update(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    input: &Mutation,
) -> Result<MutationResult, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    if let Change::SetBrowserShare {
        member: Member::Card(card),
        share_id: Some(_),
    } = &input.change
        && !super::legacy::bound_cards(state).contains_key(&card.card)
        && store
            .provider_account_receipt(owner, operation)
            .await
            .map_err(|_| "unavailable")?
            .is_none()
    {
        return Err("account_unavailable");
    }
    store
        .mutate_browser_shares(owner, operation, input, &families(state))
        .await
        .map_err(|_| "outcome_unknown")
}

/// Revoke a deleted share's profile on every Machine that holds it: the
/// share key's own placements only. Each Machine's confirmation removes its
/// placement; the share is forgotten after the last one. Unreachable
/// Machines stay pending and are retried when they reconnect.
pub(in crate::server) async fn revoke_deleted(
    state: &Arc<AppState>,
    owner: &str,
    share_id: &str,
) -> Vec<Value> {
    let key = browser_shares::share_key(owner, share_id);
    let Some(store) = state.store.as_ref() else {
        return Vec::new();
    };
    let placements = match store.browser_placements(owner, &key).await {
        Ok(placements) => placements,
        Err(error) => {
            tracing::info!(%error, "deleted browser share placements not read");
            return Vec::new();
        }
    };
    let mut results = Vec::new();
    for placement in placements {
        let outcome = revoke_on(state, owner, share_id, &placement.machine_id).await;
        if let Err(error) = &outcome {
            tracing::info!(%error, machine = %placement.machine_id, "browser share revocation pending");
        }
        results.push(json!({"machine": placement.machine_id,
            "state": if outcome.is_ok() { "revoked" } else { "pending" }}));
    }
    results
}

/// The Machine-side operation id of a share deletion: stable, so a retry
/// replays the Machine's receipt.
fn machine_operation_id(owner: &str, share_id: &str) -> String {
    super::revocation::machine_operation_id(&format!("browser-share-delete:{owner}:{share_id}"))
}

async fn revoke_on(
    state: &Arc<AppState>,
    owner: &str,
    share_id: &str,
    machine_id: &str,
) -> anyhow::Result<bool> {
    let store = state.store.as_ref().context("store unavailable")?;
    let key = browser_shares::share_key(owner, share_id);
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(machine_id)
        .context("machine offline")?;
    anyhow::ensure!(
        protocol >= MACHINE_BROWSER_PROTOCOL_VERSION,
        "machine protocol predates browsers"
    );
    // An open committed before the deletion is answered first.
    let lifecycle = state.browsers.lifecycle_lock(key.as_str());
    let _dispatch = tokio::time::timeout(REVOKE_TIMEOUT, lifecycle.lock())
        .await
        .context("an open of this share is still in flight")?;
    let event = state
        .machine_control
        .browser_command(
            &connection,
            super::request_id("browser-share-revoke"),
            Command::Revoke {
                account_id: key,
                operation_id: machine_operation_id(owner, share_id),
            },
            REVOKE_TIMEOUT,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        event.state == BrowserState::Hibernated
            && !matches!(event.reason, Some(Reason::Busy | Reason::Failed)),
        "browser share revocation unconfirmed"
    );
    state.browsers.record_authoritative(&connection, &event);
    store
        .record_browser_share_revocation(owner, share_id, machine_id, || {
            state.machine_control.is_current(&connection)
        })
        .await
}

/// Retry every pending share deletion of `owner` (after a Machine
/// reconnects, so reboot converges without a human step).
pub(in crate::server) async fn resume_deletions(state: &Arc<AppState>, owner: &str) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(shares) = store.browser_shares(owner).await else {
        return;
    };
    for share in shares
        .shares
        .iter()
        .filter(|share| share.deleting_ms.is_some())
    {
        revoke_deleted(state, owner, &share.share_id).await;
    }
}

#[cfg(test)]
#[path = "shares_tests.rs"]
mod tests;
