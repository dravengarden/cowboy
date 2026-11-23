//! Reset retry history after a sustained authenticated connection, regardless
//! of whether its read, liveness, broker or writer path reports the disconnect.
//!
//! Losing a sustained connection is usually a Controller restart (a component
//! activation takes about 20 s). Until the Controller answers again, failed
//! dials retry every second for a bounded window instead of backing off
//! exponentially: the backoff alone used to add up to 16 s to a restart, so
//! held execution requests outlived the target executor's 30 s retention of
//! closed processes (docs/uncertain-effect-recovery.md, L3). Any failure after
//! the WebSocket opened (an authentication refusal included) ends the window.

use std::time::Duration;
use tokio::time::Instant;

use crate::machine_protocol::CONTROLLER_RECONNECT_MAX_BACKOFF_SECONDS;

/// How long after a sustained connection ended failed dials retry quickly.
const RAPID_WINDOW: Duration = Duration::from_secs(60);
const RAPID_DELAY: Duration = Duration::from_secs(1);
/// Bound on one dial and WebSocket handshake inside the rapid window. Outside
/// it a dial is unbounded as before, so a slow fallback address (a stalled
/// first DNS answer) still connects once the window ends.
const DIAL_BOUND: Duration = Duration::from_secs(10);

/// The Controller could not be reached: no WebSocket was established.
#[derive(Debug)]
pub(super) struct Unreachable;

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("connecting Machine WebSocket")
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Open the Controller WebSocket, within `bound` when one is given; a failure
/// is [`Unreachable`].
pub(super) async fn dial(
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
    bound: Option<Duration>,
) -> anyhow::Result<(
    Socket,
    tokio_tungstenite::tungstenite::handshake::client::Response,
)> {
    let connect = tokio_tungstenite::connect_async(request);
    let connected = match bound {
        Some(bound) => match tokio::time::timeout(bound, connect).await {
            Ok(connected) => connected,
            Err(elapsed) => return Err(anyhow::Error::new(elapsed).context(Unreachable)),
        },
        None => connect.await,
    };
    connected.map_err(|error| anyhow::Error::new(error).context(Unreachable))
}

pub(super) async fn run(config: super::ControllerConfig) -> anyhow::Result<()> {
    let mut retry = Backoff::default();
    loop {
        let delay = match super::controller_connection(&config, &mut retry).await {
            Err(error) => {
                tracing::warn!(reason = crate::logs::failure_reason(&error), %error, "Machine controller disconnected");
                if error.downcast_ref::<Unreachable>().is_some() {
                    retry.unreachable(Instant::now())
                } else {
                    retry.disconnected(Instant::now())
                }
            }
            Ok(()) => retry.disconnected(Instant::now()),
        };
        tokio::time::sleep(delay).await;
    }
}

pub(super) struct Backoff {
    next: Duration,
    connected_at: Option<Instant>,
    /// Until when failed dials retry every [`RAPID_DELAY`].
    rapid_until: Option<Instant>,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            next: Duration::from_secs(1),
            connected_at: None,
            rapid_until: None,
        }
    }
}

impl Backoff {
    pub(super) fn connected(&mut self, now: Instant) {
        self.connected_at = Some(now);
    }

    /// A connection ended, or failed after its WebSocket opened.
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
            self.rapid_until = Some(now + RAPID_WINDOW);
        } else {
            // The Controller answered and still failed: back off.
            self.rapid_until = None;
        }
        let delay = self.next;
        self.next = (delay * 2).min(cap);
        delay
    }

    /// The bound for the next dial: [`DIAL_BOUND`] inside the rapid window.
    pub(super) fn dial_bound(&self) -> Option<Duration> {
        self.rapid_until
            .is_some_and(|until| Instant::now() < until)
            .then_some(DIAL_BOUND)
    }

    /// A dial failed before any WebSocket opened.
    pub(super) fn unreachable(&mut self, now: Instant) -> Duration {
        // A rapid retry never starts past the window's end.
        if self
            .rapid_until
            .is_some_and(|until| now + RAPID_DELAY < until)
        {
            return RAPID_DELAY;
        }
        self.rapid_until = None;
        self.disconnected(now)
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
        // Without a sustained connection, failed dials back off too.
        assert_eq!(retry.unreachable(now), Duration::from_secs(30));
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
            // A subsequent failure after the socket opened does not reuse the
            // old health evidence.
            assert_eq!(retry.disconnected(now), Duration::from_secs(2));
        }
    }

    #[test]
    fn a_restarting_controller_is_redialled_every_second_within_the_window() {
        let mut retry = Backoff::default();
        let mut now = Instant::now();
        retry.connected(now);
        now += Duration::from_secs(600);
        assert_eq!(retry.disconnected(now), Duration::from_secs(1));
        let lost = now;
        // The Controller restarts: every failed dial in the window waits 1 s.
        while now + RAPID_DELAY < lost + RAPID_WINDOW {
            now += RAPID_DELAY;
            assert_eq!(retry.unreachable(now), RAPID_DELAY);
        }
        // A failure in the window's last second does not retry past its end,
        // and the window does not extend: the backoff resumes.
        now = lost + RAPID_WINDOW - Duration::from_millis(500);
        assert_eq!(retry.unreachable(now), Duration::from_secs(2));
        assert_eq!(retry.unreachable(now), Duration::from_secs(4));
    }

    #[test]
    fn a_refusal_after_the_socket_opened_ends_the_rapid_window() {
        let mut retry = Backoff::default();
        let mut now = Instant::now();
        retry.connected(now);
        now += Duration::from_secs(600);
        assert_eq!(retry.disconnected(now), Duration::from_secs(1));
        now += Duration::from_secs(1);
        assert_eq!(retry.unreachable(now), RAPID_DELAY);
        // The Controller answered and refused (no `connected`): back off.
        assert_eq!(retry.disconnected(now), Duration::from_secs(2));
        assert_eq!(retry.unreachable(now), Duration::from_secs(4));
    }

    #[test]
    fn a_dial_failure_is_classified_unreachable_through_its_context() {
        let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            .context(Unreachable);
        assert!(error.downcast_ref::<Unreachable>().is_some());
        assert_eq!(crate::logs::failure_reason(&error), "connection_refused");
        assert_eq!(format!("{error}"), "connecting Machine WebSocket");
    }

    fn request(port: u16) -> tokio_tungstenite::tungstenite::handshake::client::Request {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
        format!("ws://127.0.0.1:{port}/api/machine/connect")
            .into_client_request()
            .expect("request")
    }

    #[tokio::test]
    async fn a_proxy_error_while_the_controller_is_down_is_a_failed_dial() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 1024];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n")
                .await;
        });
        let error = dial(request(port), Some(DIAL_BOUND))
            .await
            .expect_err("refused");
        assert!(error.downcast_ref::<Unreachable>().is_some());
        assert_eq!(crate::logs::failure_reason(&error), "websocket_handshake");
    }

    #[tokio::test]
    async fn a_stalled_upgrade_is_cut_by_the_bound_as_a_failed_dial() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accepts and never answers the upgrade.
        let held = tokio::spawn(async move { listener.accept().await });
        let error = dial(request(port), Some(Duration::from_millis(200)))
            .await
            .expect_err("cut");
        assert!(error.downcast_ref::<Unreachable>().is_some());
        assert_eq!(crate::logs::failure_reason(&error), "timeout");
        held.abort();
    }

    #[test]
    fn dials_are_bounded_only_inside_the_rapid_window() {
        let mut retry = Backoff::default();
        assert_eq!(retry.dial_bound(), None);
        retry.connected(Instant::now() - Duration::from_secs(600));
        retry.disconnected(Instant::now());
        assert_eq!(retry.dial_bound(), Some(DIAL_BOUND));
    }
}
