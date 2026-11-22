//! A Machine that was offline during an adoption may have rotated the moved
//! root generation meanwhile. Its reconnect must not deliver the root wipe
//! before the Machine has reported that rotation: the wipe deletes the
//! writable projection, and a later credential scan skips a wiped
//! generation. The wipe of such a Provider waits for the Machine's first
//! credential observation on the new connection (its scan publishes the
//! Plugin inventory first, then any refresh candidates, which the
//! connection's read loop routes into the account in order), plus a short
//! settle. A Machine that never reports (an older one) gets it after a
//! bounded wait instead.
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::watch;

/// After the observation: its refresh candidates follow it on the connection.
pub(in crate::server) const SETTLE: Duration = Duration::from_secs(15);
/// A Machine that never publishes an observation (compatibility).
pub(in crate::server) const COMPATIBILITY_WAIT: Duration = Duration::from_secs(300);

static AWAITING: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(Mutex::default);

/// Whether a Machine reporting `reported` still holds the moved root
/// generation `moved_root` and may carry an unreported rotation of it.
pub(in crate::server) fn holds_moved_generation(reported: Option<u64>, moved_root: u64) -> bool {
    reported == Some(moved_root)
}

/// Register `machine` as awaiting its first credential observation on its
/// new connection (shared by every deferred Provider of that Machine).
pub(in crate::server) fn defer(machine: &str) -> watch::Receiver<bool> {
    AWAITING
        .lock()
        .entry(machine.to_owned())
        .or_insert_with(|| watch::channel(false).0)
        .subscribe()
}

/// `machine` published a credential observation (a Plugin inventory event).
pub(in crate::server) fn observed(machine: &str) {
    if let Some(sender) = AWAITING.lock().remove(machine) {
        sender.send_replace(true);
    }
}

/// Wait until it is safe to deliver a deferred wipe.
pub(in crate::server) async fn released(
    mut observation: watch::Receiver<bool>,
    settle: Duration,
    wait: Duration,
) {
    if tokio::time::timeout(wait, observation.wait_for(|seen| *seen))
        .await
        .is_ok_and(|seen| seen.is_ok())
    {
        tokio::time::sleep(settle).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_machine_still_on_the_moved_generation_is_deferred() {
        assert!(holds_moved_generation(Some(7), 7));
        assert!(!holds_moved_generation(Some(8), 7));
        assert!(!holds_moved_generation(None, 7));
    }

    #[tokio::test(start_paused = true)]
    async fn a_deferred_wipe_waits_for_the_observation_or_the_compatibility_bound() {
        let started = tokio::time::Instant::now();
        let first = defer("observing-fixture");
        let second = defer("observing-fixture");
        let release = tokio::spawn(released(first, SETTLE, COMPATIBILITY_WAIT));
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(!release.is_finished());
        observed("observing-fixture");
        release.await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(10) + SETTLE);
        // Every deferred Provider of the Machine is released, also late.
        released(second, SETTLE, COMPATIBILITY_WAIT).await;
        assert_eq!(started.elapsed(), Duration::from_secs(10) + SETTLE * 2);
        // An older Machine that never reports still gets it.
        let started = tokio::time::Instant::now();
        released(defer("silent-fixture"), SETTLE, COMPATIBILITY_WAIT).await;
        assert_eq!(started.elapsed(), COMPATIBILITY_WAIT);
    }
}
