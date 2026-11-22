//! A Machine that was offline during an adoption may have rotated the moved
//! root generation meanwhile. No root wipe of that Provider reaches it before
//! the rotation is reported: the wipe deletes the writable projection, and a
//! later credential scan skips a wiped generation.
//!
//! The hold is enforced where every Provider credential is sent to a Machine
//! (`sync_provider_account_auth_to_machine`), keyed by the recipient's
//! connection, the Provider and the moved generation, so no other Machine's
//! reconciliation or a retry can bypass it. It is released only when that
//! generation is settled on that connection: a credential observation whose
//! inventory reports no pending refresh of it (or another generation), or the
//! promotion of the Machine's refresh candidate of it. A Machine that never
//! reports (an older one) is released after a bounded wait. A held send fails
//! and is retried like any other failed delivery.
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::machine_control::ConnectionToken;
use crate::machine_protocol::{PluginInventory, ProviderMaterializationState};

/// A Machine that never publishes an observation (compatibility).
pub(in crate::server) const COMPATIBILITY_WAIT: Duration = Duration::from_secs(300);

/// The refusal of a held wipe (a failed delivery, retried).
pub(in crate::server) const HELD: &str =
    "root wipe held until the Machine reports its credential state";

struct Hold {
    connection: ConnectionToken,
    generation: u64,
    until: Instant,
}

static HOLDS: LazyLock<Mutex<HashMap<(String, String), Hold>>> = LazyLock::new(Mutex::default);

/// Whether a Machine reporting `reported` still holds the moved root
/// generation `moved_root` and may carry an unreported rotation of it.
pub(in crate::server) fn holds_moved_generation(reported: Option<u64>, moved_root: u64) -> bool {
    reported == Some(moved_root)
}

/// Hold every root wipe of `provider` to `machine` on `connection` until
/// `generation` is settled there.
pub(in crate::server) fn hold(
    machine: &str,
    provider: &str,
    connection: &ConnectionToken,
    generation: u64,
) {
    hold_until(
        machine,
        provider,
        connection,
        generation,
        Instant::now() + COMPATIBILITY_WAIT,
    );
}

fn hold_until(
    machine: &str,
    provider: &str,
    connection: &ConnectionToken,
    generation: u64,
    until: Instant,
) {
    HOLDS.lock().insert(
        (machine.to_owned(), provider.to_owned()),
        Hold {
            connection: connection.clone(),
            generation,
            until,
        },
    );
}

/// Whether a root wipe of `provider` to `machine` on `connection` is held.
/// A hold of an earlier connection or past its bound is dropped.
pub(in crate::server) fn held(machine: &str, provider: &str, connection: &ConnectionToken) -> bool {
    held_at(machine, provider, connection, Instant::now())
}

fn held_at(machine: &str, provider: &str, connection: &ConnectionToken, now: Instant) -> bool {
    let mut holds = HOLDS.lock();
    let key = (machine.to_owned(), provider.to_owned());
    match holds.get(&key) {
        Some(hold) if hold.connection.same(connection) && now < hold.until => true,
        Some(_) => {
            holds.remove(&key);
            false
        }
        None => false,
    }
}

/// `machine` published a credential observation on `connection`: release
/// each held Provider whose moved generation it reports settled (another
/// generation, or no refresh of it pending).
pub(in crate::server) fn observed(
    machine: &str,
    connection: &ConnectionToken,
    plugins: &[PluginInventory],
) {
    HOLDS.lock().retain(|(held_machine, provider), hold| {
        if held_machine != machine || !hold.connection.same(connection) {
            return true;
        }
        plugins
            .iter()
            .find(|plugin| &plugin.plugin_id == provider)
            .is_some_and(|plugin| {
                plugin.auth_generation == Some(hold.generation)
                    && plugin.materialization_state == ProviderMaterializationState::Applying
            })
    });
}

/// The Machine's refresh candidate of `generation` was promoted.
pub(in crate::server) fn settled(machine: &str, provider: &str, generation: u64) {
    let mut holds = HOLDS.lock();
    let key = (machine.to_owned(), provider.to_owned());
    if holds
        .get(&key)
        .is_some_and(|hold| hold.generation == generation)
    {
        holds.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory(
        provider: &str,
        generation: u64,
        state: ProviderMaterializationState,
    ) -> PluginInventory {
        serde_json::from_value(serde_json::json!({
            "plugin_id": provider, "plugin_version": "1.0.0", "plugin_kind": "agent_provider",
            "generation_digest": "d", "contract_fingerprint": "f", "state": "active",
            "auth_generation": generation, "replica_state": "current",
            "materialization_state": state,
        }))
        .unwrap()
    }

    #[test]
    fn only_a_machine_still_on_the_moved_generation_is_held() {
        assert!(holds_moved_generation(Some(7), 7));
        assert!(!holds_moved_generation(Some(8), 7));
        assert!(!holds_moved_generation(None, 7));
    }

    #[test]
    fn a_hold_is_per_connection_and_released_only_when_its_generation_settles() {
        let connection = ConnectionToken::for_test("hold-a", "1");
        let other = ConnectionToken::for_test("hold-a", "2");
        hold("hold-a", "codex", &connection, 14);
        assert!(held("hold-a", "codex", &connection));
        assert!(!held("hold-b", "codex", &connection));
        assert!(!held("hold-a", "grok", &connection));
        // A pending refresh of the moved generation keeps it held.
        observed(
            "hold-a",
            &connection,
            &[inventory(
                "codex",
                14,
                ProviderMaterializationState::Applying,
            )],
        );
        assert!(held("hold-a", "codex", &connection));
        // Another generation's candidate settles nothing.
        settled("hold-a", "codex", 13);
        assert!(held("hold-a", "codex", &connection));
        settled("hold-a", "codex", 14);
        assert!(!held("hold-a", "codex", &connection));
        // An observation without a pending refresh settles it.
        hold("hold-a", "codex", &connection, 14);
        observed(
            "hold-a",
            &connection,
            &[inventory(
                "codex",
                14,
                ProviderMaterializationState::Current,
            )],
        );
        assert!(!held("hold-a", "codex", &connection));
        // A replaced connection, or the bound, drops it.
        hold("hold-a", "codex", &connection, 14);
        assert!(!held("hold-a", "codex", &other));
        hold_until("hold-a", "codex", &connection, 14, Instant::now());
        assert!(!held("hold-a", "codex", &connection));
    }
}
