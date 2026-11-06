//! Reset retry history after a sustained authenticated connection, regardless
//! of whether its read, liveness, broker or writer path reports the disconnect.

use std::time::Duration;
use tokio::time::Instant;

use crate::machine_protocol::CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS;

pub(super) async fn run(config: super::ControllerConfig) -> anyhow::Result<()> {
    let mut retry = Backoff::default();
    loop {
        if let Err(error) = super::controller_connection(&config, &mut retry).await {
            tracing::warn!(%error, "Machine controller disconnected");
        }
        let delay = retry.disconnected(Instant::now());
        tokio::time::sleep(delay).await;
    }
}

pub(super) struct Backoff {
    next: Duration,
    connected_at: Option<Instant>,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            next: Duration::from_secs(1),
            connected_at: None,
        }
    }
}

impl Backoff {
    pub(super) fn connected(&mut self, now: Instant) {
        self.connected_at = Some(now);
    }

    pub(super) fn disconnected(&mut self, now: Instant) -> Duration {
        let cap = Duration::from_secs(CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS);
        // Reuse the existing recovery horizon: a connection surviving the
        // whole maximum retry period clears old failures; brief flaps do not.
        if self
            .connected_at
            .take()
            .is_some_and(|start| now.duration_since(start) >= cap)
        {
            self.next = Duration::from_secs(1);
        }
        let delay = self.next;
        self.next = (delay * 2).min(cap);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_dials_and_brief_authenticated_flaps_remain_bounded() {
        let mut retry = Backoff::default();
        let mut now = Instant::now();
        for expected in [1, 2, 4, 8, 16, 30, 30, 30] {
            retry.connected(now);
            now += Duration::from_millis(100);
            assert_eq!(retry.disconnected(now), Duration::from_secs(expected));
            now += Duration::from_secs(expected);
        }
        // Authentication failure never calls connected and cannot reset it.
        assert_eq!(retry.disconnected(now), Duration::from_secs(30));
    }

    #[test]
    fn sustained_connections_reset_each_episode_independently_of_error_kind() {
        let mut retry = Backoff::default();
        let mut now = Instant::now();
        for _ in 0..3 {
            for _ in 0..6 {
                retry.disconnected(now);
            }
            retry.connected(now);
            now += Duration::from_secs(CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS);
            assert_eq!(retry.disconnected(now), Duration::from_secs(1));
            // A subsequent failed dial does not reuse the old health evidence.
            assert_eq!(retry.disconnected(now), Duration::from_secs(2));
        }
    }
}
