//! Idle renewal of a named portable account's sign-in (Grok).
//!
//! Usage never redeems a refresh token, so an account whose sessions all
//! ended keeps an access token that expires a few hours later, and every
//! usage read then reports authentication required although the sign-in is
//! intact. When a read reports that, no worker of the account runs on any
//! Machine, and the placed Machine's exact collector release declares an idle
//! renewal, the Controller asks that Machine to renew: the collector starts the
//! Provider CLI in the account's own session home and the CLI refreshes there
//! exactly as a session's startup would. The new credential reaches the vault
//! only through the Machine's ordinary refresh observation and the Service
//! compare-and-swap, like any session's refresh, and is then distributed to
//! every Machine; the next usage read uses it.
//!
//! The renewal is admitted under the owner's account lock with the exact vault
//! generation, like a usage read. It never runs while a worker of the account
//! runs (that worker's CLI renews by itself), so it adds no concurrent
//! redeemer beyond what sessions on several Machines already are. Attempts are
//! spaced per account: a renewal that changed nothing or failed means the
//! sign-in itself needs the owner, so the card keeps "sign in again".
use super::*;
use crate::machine_protocol::portable_usage::{
    PORTABLE_RENEWAL_PROTOCOL_VERSION, PortableRenewalRequest,
};

/// Minimum spacing after a renewal that renewed: one sign-in lasts hours.
const AFTER_RENEWED_MS: i64 = 30 * 60_000;
/// Spacing after a renewal that renewed nothing or failed.
const AFTER_FAILED_MS: i64 = 6 * 3_600_000;
/// The Machine never starts a renewal later than this after dispatch.
const DISPATCH_WINDOW_MS: i64 = 30_000;
/// After the budget allows it, the follow-up usage read waits this much more
/// for the renewed credential's compare-and-swap and distribution.
pub const FOLLOW_UP_MARGIN: std::time::Duration = std::time::Duration::from_secs(15);

/// The earliest next renewal of each `(owner, account)`. In memory: a
/// Controller restart allows at most one extra attempt.
static NEXT_ALLOWED: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<(String, String), i64>>,
> = std::sync::LazyLock::new(Default::default);

/// Why an account is not renewed now, or that it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The Machine holds a credential the vault does not yet; the follow-up
    /// read waits for its compare-and-swap and distribution.
    Renewed,
    /// Not attempted (spacing, a live worker, no capable Machine or release).
    Skipped,
    /// Attempted, and nothing changed or it failed.
    Failed,
}

/// Whether the account may be renewed on `machine` with `installed` now.
fn eligible(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    machine: &str,
    installed: &crate::machine_protocol::PluginInventory,
) -> bool {
    let live = state
        .hub
        .session_list_filtered(|session_owner| session_owner == Some(owner))
        .iter()
        .any(|meta| {
            meta.provider_account_id == *id
                && !meta.closing
                && state.supervisor.has_live_worker(&meta.id)
        });
    !live
        && state
            .machine_control
            .machine_supports(machine, PORTABLE_RENEWAL_PROTOCOL_VERSION)
        && state.usage.idle_renewal(installed)
}

/// Reserve the account's next renewal slot, or refuse while it is spaced.
fn reserve(owner: &str, id: &ProviderAccountId, now: i64) -> bool {
    let mut next = NEXT_ALLOWED.lock();
    let key = (owner.to_owned(), id.as_str().to_owned());
    if next.get(&key).is_some_and(|allowed| *allowed > now) {
        return false;
    }
    next.insert(key, now.saturating_add(AFTER_FAILED_MS));
    true
}

fn space(owner: &str, id: &ProviderAccountId, after_ms: i64) {
    NEXT_ALLOWED.lock().insert(
        (owner.to_owned(), id.as_str().to_owned()),
        now_ms().saturating_add(after_ms),
    );
}

/// Renew the account's sign-in on the placed Machine if it is eligible. The
/// caller holds the account's usage collection lock.
pub async fn renew(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    (machine, installed, _): &Placement,
) -> Outcome {
    if !eligible(state, owner, id, machine, installed) || !reserve(owner, id, now_ms()) {
        return Outcome::Skipped;
    }
    let request = PortableRenewalRequest {
        account_id: id.clone(),
        plugin_id: installed.plugin_id.clone(),
        plugin_version: installed.plugin_version.clone(),
        generation_digest: installed.generation_digest.clone(),
        auth_generation: target.auth_generation,
        expires_at_ms: now_ms().saturating_add(DISPATCH_WINDOW_MS),
    };
    let (pending, code) = under_account_lock(state, owner, id, target, || {
        // A worker that started since the check renews by itself.
        if !eligible(state, owner, id, machine, installed) {
            return Err("portable_renewal_skipped");
        }
        crate::usage::portable::begin_renewal(&state.machine_control, machine, installed, request)
    })
    .await;
    let result = match pending {
        Some(pending) => pending.await,
        None => Err(code),
    };
    tracing::info!(account_id = id.as_str(), %machine, outcome = ?result, "portable account renewal settled");
    match result {
        Ok("renewed") => {
            space(owner, id, AFTER_RENEWED_MS);
            Outcome::Renewed
        }
        // Nothing was sent: the next read may try again.
        Err("portable_renewal_skipped") => {
            NEXT_ALLOWED
                .lock()
                .remove(&(owner.to_owned(), id.as_str().to_owned()));
            Outcome::Skipped
        }
        _ => Outcome::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewals_of_one_account_are_spaced() {
        let id: ProviderAccountId = "11111111-1111-4111-8111-111111111111"
            .to_owned()
            .try_into()
            .unwrap();
        let owner = "renewal-spacing-owner";
        assert!(reserve(owner, &id, 1_000));
        assert!(!reserve(owner, &id, 2_000), "an attempt in flight is spaced");
        assert!(
            !reserve(owner, &id, 1_000 + AFTER_FAILED_MS - 1),
            "a failed renewal waits for the owner"
        );
        assert!(reserve(owner, &id, 1_000 + AFTER_FAILED_MS));
        assert!(reserve("another-owner", &id, 2_000), "per owner and account");
    }
}
