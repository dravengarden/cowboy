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
use crate::store::provider_accounts::{Change, Mutation, MutationResult, Registry};
use crate::store::provider_accounts::{browser_families, legacy_cards};
use serde_json::{Value, json};

const REVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The share a member uses, as the browser routes report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareView {
    pub share_id: String,
    pub label: String,
    /// Membership revision: part of the browser identity a reset names.
    pub revision: i64,
    /// Why no member may open or attach to the share's browser until a
    /// confirmed reset (or deletion): a family conflict, drift or binding.
    pub blocked: Option<&'static str>,
    pub members: Vec<browser_shares::ShareMember>,
}

impl ShareView {
    pub fn value(&self, owner: &str) -> Value {
        json!({"share_id": self.share_id, "label": self.label, "revision": self.revision, "blocked": self.blocked,
            "members": self.members.iter().map(|row| member_value(owner, row)).collect::<Vec<_>>()})
    }
}

fn member_value(owner: &str, row: &browser_shares::ShareMember) -> Value {
    json!({"member": row.member, "family": row.family, "account_id": row.member.own_key(owner)})
}

/// The member identity of a browser account; a retired card's retained
/// browser has none and keeps its own key.
pub fn member_of(account_id: &ProviderAccountId, account: &BrowserAccount) -> Option<Member> {
    match account {
        BrowserAccount::Named(_) => Some(Member::account(account_id.clone())),
        BrowserAccount::RetiredCard => None,
    }
}

/// The family of an identity in the static table (`browser_families`),
/// also for an account that left or was revoked. Never the catalog.
fn family_now(member: &Member, registry: &Registry) -> Option<String> {
    browser_shares::family_of(member, registry)
}

/// The browser key `member` uses and the share behind it. A member in no
/// share keeps `own`. Before a share key is used, every member's family and
/// every family binding is checked again against the current catalog. A
/// share that would hold two members of one family, a member or binding
/// whose family drifted or is no longer known, or a family of this member
/// bound to another identity marks the share `blocked`: its browser is not
/// opened or attached (rather than shared) until a confirmed reset renews
/// it or the share is deleted, while stopping and resetting stay possible.
pub fn resolve_key(
    member: Option<&Member>,
    own: &ProviderAccountId,
    shares: &Shares,
    current_family: impl Fn(&Member) -> Option<String>,
) -> Result<(ProviderAccountId, Option<ShareView>), &'static str> {
    let Some((member, row)) =
        member.and_then(|member| shares.membership(member).map(|row| (member, row)))
    else {
        return Ok((own.clone(), None));
    };
    let share = shares
        .share(&row.share_id)
        .filter(|share| share.deleting_ms.is_none())
        .ok_or("browser_share_changed")?;
    let members = shares.members_of(&share.share_id);
    let mut blocked = (!families_distinct(&members)).then_some("browser_share_family_conflict");
    for other in &members {
        if blocked.is_some() {
            break;
        }
        blocked = match current_family(&other.member) {
            Some(family) if family == other.family => None,
            Some(_) => Some("browser_share_family_drift"),
            None => Some("browser_share_family_unknown"),
        };
    }
    if blocked.is_none() {
        blocked = current_family(member).map_or(Some("browser_share_family_unknown"), |family| {
            browser_shares::binding_refusal(
                shares,
                &share.share_id,
                (member, &family),
                &current_family,
            )
        });
    }
    Ok((
        share.browser_key.clone(),
        Some(ShareView {
            share_id: share.share_id.clone(),
            label: share.label.clone(),
            revision: share.revision,
            members: members.into_iter().cloned().collect(),
            blocked,
        }),
    ))
}

/// `resolve_key` against the owner's stored shares and current catalog.
pub async fn resolve(
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
    resolve_key(
        member_of(account_id, account).as_ref(),
        account_id,
        &shares,
        |member| family_now(member, registry),
    )
}

/// Whether `id` is one of the owner's share keys. Share keys are never
/// addressed directly: a member's card names its own identity.
pub async fn is_share_key(
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
pub async fn list_value(state: &AppState, owner: &str) -> Result<(Value, Value), &'static str> {
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
                    .map(|row| member_value(owner, row)).collect::<Vec<_>>(),
                // Families the profile belongs to (kept after a member left).
                "bound": shares.bindings.iter().filter(|row| row.share_id == share.share_id)
                    .map(|row| json!({"family": row.family, "account_id": row.member.own_key(owner)}))
                    // A retired card's binding keeps blocking its family until
                    // a confirmed reset; named by the card's browser key.
                    .chain(shares.unreadable_bindings.iter()
                        .filter(|row| row.share_id == share.share_id)
                        .filter_map(|row| row.retired_card().map(|card| json!({"family": row.family,
                            "account_id": legacy_cards::browser_key(owner, card), "retired_card": card}))))
                    .collect::<Vec<_>>()})
        })
        .collect();
    // The static family table: Provider ids with a family.
    let providers: BTreeMap<&str, &str> = browser_families::PROVIDERS.iter().copied().collect();
    Ok((
        Value::Array(listed),
        // `cards` stays, empty, for Web clients that predate the retirement.
        json!({"version": browser_families::VERSION, "providers": providers, "cards": []}),
    ))
}

/// What an applied account change does to running browsers and viewers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
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
pub fn change_effects(owner: &str, change: &Change) -> Vec<Effect> {
    match change {
        Change::BeginRevocation { id } => vec![Effect::EndMemberViewers(id.clone())],
        Change::SetBrowserEnabled { id, enabled: false }
        | Change::SetEnabled { id, enabled: false } => vec![
            Effect::EndMemberViewers(id.clone()),
            Effect::HibernateOwn(id.clone(), "browser_disabled"),
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

/// Whether the state an applied change produced still holds: a deferred
/// effect acts only while it does, so a later change (re-enabling, leaving
/// again) is never undone by an earlier change's effect.
pub async fn change_holds(state: &AppState, owner: &str, change: &Change) -> bool {
    let Some(store) = state.store.as_ref() else {
        return false;
    };
    let (Ok(registry), Ok(shares)) = (
        store.provider_accounts(owner).await,
        store.browser_shares(owner).await,
    ) else {
        return false;
    };
    change_holds_in(change, &registry, &shares)
}

/// `change_holds` against one read of the owner's state.
pub fn change_holds_in(change: &Change, registry: &Registry, shares: &Shares) -> bool {
    let account =
        |id: &ProviderAccountId| registry.accounts.iter().find(|account| &account.id == id);
    match change {
        Change::SetBrowserShare { member, share_id } => {
            shares.membership(member).map(|row| &row.share_id) == share_id.as_ref()
        }
        Change::SetBrowserEnabled { id, enabled: false } => {
            account(id).is_some_and(|account| !account.browser_enabled)
        }
        Change::SetEnabled { id, enabled: false } => {
            account(id).is_some_and(|account| !account.enabled)
        }
        _ => true,
    }
}

/// Hibernate the member's own running browsers, one target at a time: for
/// each, under the owner's share fence (which every account change takes),
/// the change must still hold, and the fence is kept through that target's
/// dispatch so no later change lands between the check and the command.
async fn hibernate_while_holding(
    state: &Arc<AppState>,
    owner: &str,
    change: &Change,
    (account, cause): (&ProviderAccountId, &'static str),
) {
    for observed in super::current_observations(state, account) {
        if !observed.running() {
            continue;
        }
        let fence = state.browsers.share_lock(owner);
        let _fence = fence.lock().await;
        if !change_holds(state, owner, change).await {
            return;
        }
        if let Err(error) = super::send_hibernate(state, &observed, cause).await {
            tracing::info!(%error, "browser hibernate after an account change did not complete");
        }
    }
}

/// The effects to run for an applied change: none for a replayed receipt,
/// whose operation already committed (and ran its effects) earlier.
pub fn fresh_effects(owner: &str, change: &Change, replay: bool) -> Vec<Effect> {
    if replay {
        return Vec::new();
    }
    change_effects(owner, change)
}

/// Carry out the effects of a freshly applied change (never a replayed
/// receipt). Deferred effects first check that the change still holds.
pub fn run_effects(state: &Arc<AppState>, owner: &str, change: &Change, effects: Vec<Effect>) {
    for effect in effects {
        match effect {
            Effect::EndMemberViewers(member) => {
                state.browsers.cancel_member_viewers(owner, member.as_str());
            }
            Effect::EndKeyViewers(key) => state.browsers.cancel_account_viewers(key.as_str()),
            Effect::HibernateOwn(account, cause) => {
                let (state, owner, change) = (Arc::clone(state), owner.to_owned(), change.clone());
                tokio::spawn(async move {
                    hibernate_while_holding(&state, &owner, &change, (&account, cause)).await;
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

/// Identity binding for [`super::instance_command`]: explicit
/// `expected_browser`, server-recorded viewer attachment identity (for cached
/// pre-upgrade Web clients), or unbound.
#[derive(Clone, Copy, Debug)]
pub enum IdentityFence<'a> {
    None,
    Expected(&'a str),
    Attachment(&'a str),
}

/// An identity-bound request names the browser identity the member's card
/// uses now, or is refused.
pub fn expected_identity(
    expected: Option<&str>,
    resolved: &super::Resolved,
) -> Result<(), &'static str> {
    match expected {
        Some(expected) if expected != resolved.identity() => Err("browser_identity_changed"),
        _ => Ok(()),
    }
}

/// Enforce [`IdentityFence`] under the owner's share lock against `resolved`.
pub fn verify_identity_fence(
    state: &AppState,
    owner: &str,
    (account_id, machine_id): (&ProviderAccountId, &str),
    resolved: &super::Resolved,
    identity_fence: IdentityFence<'_>,
) -> Result<(), &'static str> {
    match identity_fence {
        IdentityFence::None => Ok(()),
        IdentityFence::Expected(expected) => expected_identity(Some(expected), resolved),
        IdentityFence::Attachment(attachment_id) => {
            let recorded = state
                .browsers
                .viewer_identity(
                    machine_id,
                    resolved.key.as_str(),
                    owner,
                    account_id.as_str(),
                    attachment_id,
                )
                .ok_or("browser_identity_changed")?;
            expected_identity(Some(&recorded), resolved)
        }
    }
}

/// Persist a pending identity-bound navigation fence before dispatch and
/// track it in memory so both live observations and Controller restarts keep
/// share membership changes fenced.
pub async fn begin_durable_navigation(
    state: &AppState,
    owner: &str,
    browser_key: &ProviderAccountId,
    observed: &super::Observed,
    request_id: &str,
) -> Result<(), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let record =
        crate::store::provider_accounts::browser_pending_navigations::PendingNavigationRecord {
            request_id: request_id.to_owned(),
            owner_user_id: owner.to_owned(),
            browser_key: browser_key.clone(),
            machine_id: observed.machine_id.clone(),
            instance_id: observed.event.instance.instance_id.clone(),
            incarnation: observed.event.instance.incarnation,
            observed_at_ms: observed.event.observed_at_ms,
            started_at_ms: super::now_ms(),
        };
    store
        .record_browser_pending_navigation(&record)
        .await
        .map_err(|_| "unavailable")?;
    state.browsers.begin_navigation(
        request_id.to_owned(),
        owner,
        browser_key.clone(),
        &observed.connection,
        observed.event.instance.clone(),
        observed.event.observed_at_ms,
    );
    Ok(())
}

/// Clear a tracked navigation fence from both memory and durable storage.
pub async fn clear_durable_navigation(state: &AppState, request_id: &str) {
    state.browsers.settle_navigation(request_id);
    if let Some(store) = state.store.as_ref() {
        let mut ids = state.browsers.take_settled_navigation_ids();
        if !ids.iter().any(|id| id == request_id) {
            ids.push(request_id.to_owned());
        }
        if store
            .delete_browser_pending_navigations(&ids)
            .await
            .is_err()
        {
            state.browsers.requeue_settled_navigation_ids(ids);
        }
    }
}

/// Flush any request ids settled by synchronous Machine observation callbacks
/// out of durable storage (`browser_pending_navigations`).
pub async fn drain_settled_navigations(state: &AppState) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let ids = state.browsers.take_settled_navigation_ids();
    if ids.is_empty() {
        return;
    }
    if let Err(error) = store.delete_browser_pending_navigations(&ids).await {
        tracing::info!(%error, "settled browser navigation rows not deleted; retrying");
        state.browsers.requeue_settled_navigation_ids(ids);
    }
}

/// Whether `(owner, browser_key)` has an unsettled identity-bound navigation
/// in either memory or durable storage (including immediately after a
/// Controller restart).
pub async fn has_pending_navigation(
    state: &AppState,
    owner: &str,
    browser_key: &ProviderAccountId,
) -> Result<bool, &'static str> {
    drain_settled_navigations(state).await;
    if state.browsers.has_pending_navigation(owner, browser_key) {
        return Ok(true);
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    store
        .has_pending_browser_navigation(owner, browser_key)
        .await
        .map_err(|_| "unavailable")
}

/// Whether `account_id` is currently a member of a shared browser whose
/// profile has an unsettled identity-bound navigation in flight.
pub async fn member_has_pending_navigation(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<bool, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let shares = store
        .browser_shares(owner)
        .await
        .map_err(|_| "unavailable")?;
    let member = Member::account(account_id.clone());
    let Some(row) = shares.membership(&member) else {
        return Ok(false);
    };
    let Some(share) = shares.share(&row.share_id) else {
        return Ok(false);
    };
    has_pending_navigation(state, owner, &share.browser_key).await
}

/// Commit any deferred share-membership removals for revoking accounts of
/// `owner` whose share no longer has a pending navigation. Caller MUST hold
/// `state.browsers.share_lock(owner)`.
pub async fn flush_revoked_share_leaves_locked(state: &AppState, owner: &str) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(deferred) = store.deferred_revoked_share_members(Some(owner)).await else {
        tracing::info!("deferred revoked share members not read; retrying");
        return;
    };
    for (_, account_id, browser_key) in deferred {
        if has_pending_navigation(state, owner, &browser_key)
            .await
            .unwrap_or(true)
        {
            continue;
        }
        match store.finish_revoked_share_leave(owner, &account_id).await {
            Ok(Some(revision)) => {
                state
                    .hub
                    .announce_preferences(owner, "provider_accounts", revision);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, account = %account_id.as_str(), "deferred revoked share leave failed; retrying");
            }
        }
    }
}

/// Settle durable or in-memory identity-bound navigations whose dispatching
/// connection dropped or that survived a Controller restart once their Machine
/// connects: query first (settling immediately if the instance exited or was
/// replaced), and if that exact instance is still running on the Machine,
/// hibernate it so confirmed unit exit proves the old navigation can never land.
pub async fn reconcile_orphaned_navigations(state: &Arc<AppState>, machine_id: Option<&str>) {
    drain_settled_navigations(state).await;
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(rows) = store.pending_browser_navigations(machine_id).await else {
        tracing::info!("pending browser navigations not read; retrying");
        return;
    };
    for row in rows {
        if state
            .browsers
            .is_navigation_live_on_current(&row.request_id)
        {
            continue;
        }
        let instance = crate::machine_protocol::machine_browser::InstanceRef {
            account_id: row.browser_key.clone(),
            instance_id: row.instance_id.clone(),
            incarnation: row.incarnation,
        };
        state
            .browsers
            .seed_durable_navigation(super::coordinator::PendingNavigation {
                request_id: row.request_id.clone(),
                owner: row.owner_user_id.clone(),
                browser_key: row.browser_key.clone(),
                machine_id: row.machine_id.clone(),
                connection: None,
                instance: instance.clone(),
                observed_at_ms: row.observed_at_ms,
            });
        let Ok(connection) = super::connection_for(state, &row.machine_id) else {
            continue;
        };
        let Some(event) = super::query_machine(state, &row.machine_id, &row.browser_key).await
        else {
            continue;
        };
        drain_settled_navigations(state).await;
        if !state.browsers.is_navigation_pending(&row.request_id) {
            let _ = store
                .delete_browser_pending_navigation(&row.request_id)
                .await;
            continue;
        }
        if matches!(
            event.state,
            BrowserState::Starting | BrowserState::Detached | BrowserState::Attached
        ) && event.instance == instance
        {
            let request = super::request_id("browser-hibernate");
            super::hibernate_sent(&row.machine_id, &instance, &request, "navigation_reconcile");
            match state
                .machine_control
                .browser_command(
                    &connection,
                    request,
                    Command::Hibernate {
                        instance: instance.clone(),
                    },
                    super::HIBERNATE_TIMEOUT,
                )
                .await
            {
                Ok(hib_event) => {
                    if hib_event.state == BrowserState::Hibernated
                        && !matches!(hib_event.reason, Some(Reason::Busy | Reason::Failed))
                    {
                        state.browsers.record_authoritative(&connection, &hib_event);
                    }
                    drain_settled_navigations(state).await;
                }
                Err(error) => {
                    tracing::info!(
                        %error,
                        machine = %row.machine_id,
                        "browser hibernate to settle orphaned navigation did not complete"
                    );
                }
            }
        }
    }
    drain_settled_navigations(state).await;
}

/// Reconcile deferred share-membership removals for revoking accounts once
/// their share's pending navigation has settled.
pub async fn reconcile_revoked_share_leaves(state: &AppState, owner: Option<&str>) {
    drain_settled_navigations(state).await;
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(deferred) = store.deferred_revoked_share_members(owner).await else {
        tracing::info!("deferred revoked share members not read; retrying");
        return;
    };
    let mut owners = std::collections::BTreeSet::new();
    for (row_owner, _, browser_key) in deferred {
        if !has_pending_navigation(state, &row_owner, &browser_key)
            .await
            .unwrap_or(true)
        {
            owners.insert(row_owner);
        }
    }
    for row_owner in owners {
        let fence = state.browsers.share_lock(&row_owner);
        let _fence = fence.lock().await;
        flush_revoked_share_leaves_locked(state, &row_owner).await;
    }
}

/// Apply a share change through the registry writer, under the owner's
/// share fence (held by the caller), and report whether it was a no-op (the
/// member already in exactly that share), whose browser effects are none.
/// Joining a legacy card needs a card the Service collects now, and joining
/// a share whose browser already lives on a Machine needs that Machine to
/// be able to host the member (`network::join_refusal`). An operation
/// already recorded (its replay, or a reuse the writer refuses) and a stale
/// `expected_revision` (the writer's `revision_conflict`) are resolved by
/// the writer first, so they never wait on, or read as, a Machine check; a
/// no-op is answered without these checks too.
pub async fn update(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    input: &Mutation,
) -> Result<(MutationResult, bool), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    if store
        .provider_account_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?
        .is_none()
    {
        flush_revoked_share_leaves_locked(state, owner).await;
    }
    let mut noop = false;
    match &input.change {
        Change::SetBrowserShare { member, share_id }
            if fresh_join(store, owner, operation, input.expected_revision).await? =>
        {
            let shares = store
                .browser_shares(owner)
                .await
                .map_err(|_| "unavailable")?;
            let current = shares.membership(member).map(|row| row.share_id.as_str());
            noop = current == share_id.as_deref();
            if !noop {
                let mut keys = vec![member.own_key(owner)];
                if let Some(id) = current {
                    keys.push(browser_shares::share_key(owner, id));
                }
                if let Some(id) = share_id.as_deref() {
                    keys.push(browser_shares::share_key(owner, id));
                }
                for key in keys {
                    if has_pending_navigation(state, owner, &key).await? {
                        return Err("browser_navigation_pending");
                    }
                }
                if let Some(share_id) = share_id {
                    super::network::join_refusal(state, owner, (member, share_id)).await?;
                }
            }
        }
        Change::DeleteBrowserShare { share_id }
            if fresh_join(store, owner, operation, input.expected_revision).await?
                && has_pending_navigation(
                    state,
                    owner,
                    &browser_shares::share_key(owner, share_id),
                )
                .await? =>
        {
            return Err("browser_navigation_pending");
        }
        _ => {}
    }
    let result = store
        .mutate_browser_shares(owner, operation, input)
        .await
        .map_err(|_| "outcome_unknown")?;
    Ok((result, noop))
}

/// Whether a membership change is fresh and at the registry's revision, so
/// its own checks run before the writer: no receipt is recorded under
/// `operation`, and `expected_revision` is current. Revisions only advance,
/// so a stale one stays the writer's conflict; the caller's owner fence
/// serializes every share change, so a receipt cannot appear meanwhile.
pub async fn fresh_join(
    store: &crate::store::Store,
    owner: &str,
    operation: &str,
    expected_revision: i64,
) -> Result<bool, &'static str> {
    if store
        .provider_account_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?
        .is_some()
    {
        return Ok(false);
    }
    let revision = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?
        .revision;
    Ok(revision == expected_revision)
}

/// Revoke a deleted share's profile on every Machine that holds it: the
/// share key's own placements only. Each Machine's confirmation removes its
/// placement; the share is forgotten after the last one. Unreachable
/// Machines stay pending and are retried when they reconnect.
pub async fn revoke_deleted(state: &Arc<AppState>, owner: &str, share_id: &str) -> Vec<Value> {
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

/// Retry backoff of a pending share deletion while its Machine stays
/// connected; the reconciler also resumes every pending deletion after a
/// Controller restart (the deleting share is durable).
const DELETION_RETRY_INITIAL: std::time::Duration = std::time::Duration::from_secs(5);
const DELETION_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(300);
const DELETION_TICK: std::time::Duration = std::time::Duration::from_secs(5);

/// The next attempt of each pending deletion, doubling its delay after each
/// unconfirmed attempt. Deletions no longer pending are forgotten.
#[derive(Default)]
pub struct DeletionRetries {
    due: BTreeMap<(String, String), (std::time::Instant, std::time::Duration)>,
}

impl DeletionRetries {
    /// The pending deletions due at `now`, keeping only `pending` ones.
    pub fn due(
        &mut self,
        pending: &[(String, String)],
        now: std::time::Instant,
    ) -> Vec<(String, String)> {
        self.due.retain(|key, _| pending.contains(key));
        pending
            .iter()
            .filter(|key| {
                self.due
                    .entry((*key).clone())
                    .or_insert((now, DELETION_RETRY_INITIAL))
                    .0
                    <= now
            })
            .cloned()
            .collect()
    }

    /// An attempt did not confirm every placement: retry later.
    pub fn failed(&mut self, key: &(String, String), now: std::time::Instant) {
        let entry = self
            .due
            .entry(key.clone())
            .or_insert((now, DELETION_RETRY_INITIAL));
        *entry = (now + entry.1, (entry.1 * 2).min(DELETION_RETRY_MAX));
    }
}

/// Retry every pending share deletion, with the same Machine operation id,
/// until each is confirmed: started with the Controller, so a restart or a
/// timed-out revoke (an open holding the lifecycle lock) converges without
/// a human step.
pub fn start_deletion_reconciler(state: &Arc<AppState>) {
    // Dedicated notification task so frequent navigation settlements flush
    // durable fences and deferred revocation leaves promptly without ever
    // postponing the periodic share-recovery interval below.
    let weak_notify = Arc::downgrade(state);
    let nav_settled = state.browsers.navigation_settled_notify();
    tokio::spawn(async move {
        loop {
            let woken = tokio::select! {
                () = nav_settled.notified() => true,
                () = tokio::time::sleep(DELETION_TICK) => false,
            };
            let Some(state) = weak_notify.upgrade() else {
                return;
            };
            if woken {
                reconcile_revoked_share_leaves(&state, None).await;
            }
        }
    });
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut retries = DeletionRetries::default();
        let mut tick = tokio::time::interval(DELETION_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else { return };
            reconcile_orphaned_navigations(&state, None).await;
            reconcile_revoked_share_leaves(&state, None).await;
            let Some(store) = state.store.as_ref() else {
                return;
            };
            let (Ok(deletions), Ok(resets)) = (
                store.deleting_browser_shares().await,
                store.pending_share_resets().await,
            ) else {
                tracing::info!("pending browser share work not read; retrying");
                continue;
            };
            let mut pending = deletions;
            pending.extend(
                resets
                    .iter()
                    .map(|(owner, share, _)| (owner.clone(), format!("reset:{share}"))),
            );
            for key in retries.due(&pending, std::time::Instant::now()) {
                let settled = match key.1.strip_prefix("reset:") {
                    Some(share) => {
                        let found = resets
                            .iter()
                            .find(|(owner, id, _)| *owner == key.0 && id == share);
                        match found {
                            Some((_, _, reset)) => {
                                reconcile_reset(&state, &key.0, share, reset).await
                            }
                            None => true,
                        }
                    }
                    None => revoke_deleted(&state, &key.0, &key.1)
                        .await
                        .iter()
                        .all(|result| result["state"] == "revoked"),
                };
                if !settled {
                    retries.failed(&key, std::time::Instant::now());
                }
            }
        }
    });
}

/// A pending reset younger than this is still awaited by its own request
/// (whose timeout is shorter), so the reconciler leaves it alone.
const RESET_GRACE_MS: i64 = 90_000;

/// Whether the reconciler may settle a pending reset `age_ms` old.
pub fn reset_due(age_ms: i64) -> bool {
    age_ms >= RESET_GRACE_MS
}

/// What a Machine's answer to a reset proves about that reset operation.
/// The Machine keys a reset by its request id and keeps a durable fence for
/// it: `owner_request` (carried out now) and a plain `hibernated` (that
/// operation already completed) prove it applied; `superseded` (no fence was
/// raised: the profile revision moved, possibly by this very reset whose
/// remembered completion a later operation replaced) proves it runs no
/// further, not that it never ran; `failed`
/// (the fence is retained until the unit's exit is confirmed), `busy`
/// (another operation holds the fence) and anything else prove neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetOutcome {
    Applied,
    RunsNoFurther,
    Unsettled,
}

pub fn reset_outcome(event: &crate::machine_protocol::machine_browser::StateEvent) -> ResetOutcome {
    if event.state != BrowserState::Hibernated {
        return ResetOutcome::Unsettled;
    }
    match event.reason {
        Some(Reason::OwnerRequest) | None => ResetOutcome::Applied,
        Some(Reason::Superseded) => ResetOutcome::RunsNoFurther,
        _ => ResetOutcome::Unsettled,
    }
}

/// Settle a share reset whose own request did not learn a terminal
/// outcome: the same operation (same request id, key and expected profile
/// revision) is sent again, so the Machine continues or reports its own
/// fenced operation rather than starting another. Only an answer proving
/// it applied or runs no further clears the fence, and never renews the
/// bindings: only a reset's own first confirmation proves the profile
/// fresh since it began. Returns whether the reset is settled.
async fn reconcile_reset(
    state: &Arc<AppState>,
    owner: &str,
    share_id: &str,
    pending: &browser_shares::PendingReset,
) -> bool {
    if !reset_due(super::now_ms().saturating_sub(pending.started_ms)) {
        return false;
    }
    let Some(store) = state.store.as_ref() else {
        return false;
    };
    let Ok(connection) = super::connection_for(state, &pending.machine_id) else {
        return false;
    };
    let answer = state
        .machine_control
        .browser_command(
            &connection,
            pending.operation_id.clone(),
            Command::Reset {
                account_id: browser_shares::share_key(owner, share_id),
                expected_profile_revision: pending.expected_profile_revision,
            },
            super::RESET_TIMEOUT,
        )
        .await;
    let outcome = match answer {
        Ok(event) => reset_outcome(&event),
        Err(error) => {
            tracing::info!(%error, operation = %pending.operation_id, "browser share reset unanswered");
            ResetOutcome::Unsettled
        }
    };
    if outcome == ResetOutcome::Unsettled {
        return false;
    }
    store
        .finish_share_reset(owner, (share_id, &pending.operation_id), false)
        .await
        .is_ok()
}

/// Retry every pending share deletion of `owner` (after a Machine
/// reconnects, so reboot converges without a human step).
pub async fn resume_deletions(state: &Arc<AppState>, owner: &str) {
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
