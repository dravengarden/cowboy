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
use crate::store::provider_accounts::browser_families;
use crate::store::provider_accounts::browser_shares::{self, Member, Shares, families_distinct};
use crate::store::provider_accounts::{Change, Mutation, MutationResult, Receipt, Registry};
use serde_json::{Value, json};

const REVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The share a member uses, as the browser routes report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) struct ShareView {
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
    pub(in crate::server) fn value(&self, owner: &str) -> Value {
        json!({"share_id": self.share_id, "label": self.label, "revision": self.revision, "blocked": self.blocked,
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
pub(in crate::server) fn resolve_key(
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
    resolve_key(
        member_of(account_id, account).as_ref(),
        account_id,
        &shares,
        |member| family_now(member, registry),
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
                    .map(|row| member_value(owner, row)).collect::<Vec<_>>(),
                // Families the profile belongs to (kept after a member left).
                "bound": shares.bindings.iter().filter(|row| row.share_id == share.share_id)
                    .map(|row| json!({"family": row.family, "account_id": row.member.own_key(owner)}))
                    .collect::<Vec<_>>()})
        })
        .collect();
    // The static family table: Provider ids and card ids with a family.
    let providers: BTreeMap<&str, &str> = browser_families::PROVIDERS.iter().copied().collect();
    Ok((
        Value::Array(listed),
        json!({"version": browser_families::VERSION, "providers": providers, "cards": browser_families::CARDS}),
    ))
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

/// Whether the state an applied change produced still holds: a deferred
/// effect acts only while it does, so a later change (re-enabling, leaving
/// again) is never undone by an earlier change's effect.
pub(in crate::server) async fn change_holds(
    state: &AppState,
    owner: &str,
    change: &Change,
) -> bool {
    let Some(store) = state.store.as_ref() else {
        return false;
    };
    let (Ok(registry), Ok(cards), Ok(shares)) = (
        store.provider_accounts(owner).await,
        store.legacy_browser_cards(owner).await,
        store.browser_shares(owner).await,
    ) else {
        return false;
    };
    change_holds_in(change, &registry, &cards, &shares)
}

/// `change_holds` against one read of the owner's state.
pub(in crate::server) fn change_holds_in(
    change: &Change,
    registry: &Registry,
    enabled_cards: &[String],
    shares: &Shares,
) -> bool {
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
        Change::SetLegacyBrowserEnabled {
            card,
            enabled: false,
        } => !enabled_cards.contains(card),
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
pub(in crate::server) fn fresh_effects(
    owner: &str,
    change: &Change,
    receipt: &Receipt,
    replay: bool,
) -> Vec<Effect> {
    if replay {
        return Vec::new();
    }
    change_effects(owner, change, receipt)
}

/// Carry out the effects of a freshly applied change (never a replayed
/// receipt). Deferred effects first check that the change still holds.
pub(in crate::server) fn run_effects(
    state: &Arc<AppState>,
    owner: &str,
    change: &Change,
    effects: Vec<Effect>,
) {
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
        .mutate_browser_shares(owner, operation, input)
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

/// Retry backoff of a pending share deletion while its Machine stays
/// connected; the reconciler also resumes every pending deletion after a
/// Controller restart (the deleting share is durable).
const DELETION_RETRY_INITIAL: std::time::Duration = std::time::Duration::from_secs(5);
const DELETION_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(300);
const DELETION_TICK: std::time::Duration = std::time::Duration::from_secs(5);

/// The next attempt of each pending deletion, doubling its delay after each
/// unconfirmed attempt. Deletions no longer pending are forgotten.
#[derive(Default)]
pub(in crate::server) struct DeletionRetries {
    due: BTreeMap<(String, String), (std::time::Instant, std::time::Duration)>,
}

impl DeletionRetries {
    /// The pending deletions due at `now`, keeping only `pending` ones.
    pub(in crate::server) fn due(
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
    pub(in crate::server) fn failed(&mut self, key: &(String, String), now: std::time::Instant) {
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
pub(in crate::server) fn start_deletion_reconciler(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut retries = DeletionRetries::default();
        loop {
            tokio::time::sleep(DELETION_TICK).await;
            let Some(state) = weak.upgrade() else { return };
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
pub(in crate::server) fn reset_due(age_ms: i64) -> bool {
    age_ms >= RESET_GRACE_MS
}

/// What a Machine's answer to a reset proves about that reset operation.
/// The Machine keys a reset by its request id and keeps a durable fence for
/// it: `owner_request` (carried out now) and a plain `hibernated` (that
/// operation already completed) prove it applied; `superseded` (no fence was
/// raised: the profile revision moved) proves it will never run; `failed`
/// (the fence is retained until the unit's exit is confirmed), `busy`
/// (another operation holds the fence) and anything else prove neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::server) enum ResetOutcome {
    Applied,
    NotExecuted,
    Unsettled,
}

pub(in crate::server) fn reset_outcome(
    event: &crate::machine_protocol::machine_browser::StateEvent,
) -> ResetOutcome {
    if event.state != BrowserState::Hibernated {
        return ResetOutcome::Unsettled;
    }
    match event.reason {
        Some(Reason::OwnerRequest) | None => ResetOutcome::Applied,
        Some(Reason::Superseded) => ResetOutcome::NotExecuted,
        _ => ResetOutcome::Unsettled,
    }
}

/// Settle a share reset whose own request did not learn a terminal
/// outcome: the same operation (same request id, key and expected profile
/// revision) is sent again, so the Machine continues or reports its own
/// fenced operation rather than starting another. Only an answer proving
/// it applied or never runs clears the fence, and never renews the
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
