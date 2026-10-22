//! Prove the Controller can answer, even when a local TCP proxy still ACKs
//! writes after its upstream connection has failed. Pacing-credit Pongs must
//! not satisfy this probe; its payload has a separate namespace and sequence.

use std::time::Duration;
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct Probe {
    sequence: u64,
    pending: Option<(Vec<u8>, Instant)>,
}

impl Probe {
    pub(super) fn ping(&mut self, now: Instant, timeout: Duration) -> Option<Vec<u8>> {
        if self.pending.is_some() {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let payload = format!("cowboy-liveness-v1:{}", self.sequence).into_bytes();
        self.pending = Some((payload.clone(), now + timeout));
        Some(payload)
    }

    pub(super) fn pong(&mut self, payload: &[u8]) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(expected, _)| expected == payload)
        {
            self.pending = None;
        }
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, deadline)| *deadline)
    }
}

pub(super) async fn wait(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt as _, StreamExt as _};
    use tokio::io::AsyncReadExt as _;
    use tokio_tungstenite::{
        WebSocketStream,
        tungstenite::{Message, protocol::Role},
    };

    #[tokio::test]
    async fn proxy_accepting_writes_without_forwarding_replies_expires() {
        let (local, mut proxy) = tokio::io::duplex(4096);
        let sink = tokio::spawn(async move {
            let mut bytes = Vec::new();
            proxy.read_to_end(&mut bytes).await.unwrap();
            bytes
        });
        let mut socket = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let mut probe = Probe::default();
        let payload = probe
            .ping(Instant::now(), Duration::from_millis(100))
            .unwrap();
        socket.send(Message::Ping(payload.into())).await.unwrap();
        // TCP write success is exactly the false evidence in the incident.
        // The proxy accepts another frame, but never supplies a Controller Pong.
        socket
            .send(Message::Text("heartbeat".into()))
            .await
            .unwrap();
        tokio::select! {
            _ = wait(probe.deadline()) => {},
            message = socket.next() => panic!("proxy unexpectedly replied: {message:?}"),
        }
        drop(socket);
        assert!(!sink.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn standard_websocket_peer_answers_without_protocol_changes() {
        let (local, remote) = tokio::io::duplex(4096);
        let peer = tokio::spawn(async move {
            let mut socket = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
            assert!(matches!(socket.next().await, Some(Ok(Message::Ping(_)))));
            socket.flush().await.unwrap();
        });
        let mut socket = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let mut probe = Probe::default();
        let payload = probe.ping(Instant::now(), Duration::from_secs(1)).unwrap();
        socket.send(Message::Ping(payload.into())).await.unwrap();
        tokio::select! {
            _ = wait(probe.deadline()) => panic!("healthy peer timed out"),
            message = socket.next() => {
                let Some(Ok(Message::Pong(payload))) = message else { panic!("unexpected reply {message:?}") };
                probe.pong(&payload);
            }
        }
        assert!(probe.deadline().is_none());
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn successful_writes_cannot_extend_an_unanswered_probe() {
        let mut probe = Probe::default();
        let start = Instant::now();
        let timeout = Duration::from_millis(10);
        let payload = probe.ping(start, timeout).unwrap();
        for _ in 0..10 {
            assert!(probe.ping(Instant::now(), timeout).is_none());
            probe.pong(b"unrelated pacing credit");
        }
        assert_eq!(probe.deadline(), Some(start + timeout));
        wait(probe.deadline()).await;
        assert!(Instant::now() >= start + timeout);
        probe.pong(&payload);
        assert_eq!(probe.deadline(), None);
        let next = probe.ping(Instant::now(), timeout).unwrap();
        assert_ne!(next, payload);
        probe.pong(&payload);
        assert!(
            probe.deadline().is_some(),
            "stale Pong must not acknowledge a new probe"
        );
        probe.pong(&next);
        assert_eq!(probe.deadline(), None);
    }
}
