//! The Machine side of one display stream: a dedicated WebSocket to the
//! Controller, opened with the one-time stream token, relayed byte for byte
//! to and from the instance's RFB socket in bounded binary frames. Payloads
//! are never logged, persisted or inspected beyond framing; counters record
//! bytes and frames.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use futures::{SinkExt as _, StreamExt as _};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use crate::machine_protocol::machine_browser::{
    MAX_FRAME_BYTES, PAIRING_TIMEOUT_MS, STREAM_PATH, STREAM_TOKEN_HEADER,
};

/// A frame that cannot be delivered within this window closes the stream:
/// a slow consumer stalls its producer instead of growing a buffer.
const CREDIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a stream ended. Reported to the instance so it can move to
/// `detached`; never carries payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEnd {
    /// The Controller or viewer closed the WebSocket.
    ViewerClosed,
    /// The RFB side closed (display server gone).
    DisplayClosed,
    /// Pairing was refused or timed out before any byte flowed.
    PairingFailed(String),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub frames_to_viewer: u64,
    pub bytes_to_viewer: u64,
    pub frames_to_display: u64,
    pub bytes_to_display: u64,
}

/// A running relay. Dropping it cancels the stream.
pub struct Stream {
    cancel: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Stream {
    /// Open the stream to `endpoint` (the Controller's Machine WebSocket base
    /// URL) and relay it with the RFB socket at `rfb_socket`; `on_end` runs
    /// exactly once with the outcome.
    pub fn start(
        endpoint: reqwest::Url,
        token: String,
        rfb_socket: PathBuf,
        on_end: Box<dyn FnOnce(StreamEnd, Counters) + Send>,
    ) -> Self {
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let (end, counters) = relay(endpoint, token, rfb_socket, cancelled).await;
            tracing::debug!(
                ?end,
                frames_to_viewer = counters.frames_to_viewer,
                bytes_to_viewer = counters.bytes_to_viewer,
                frames_to_display = counters.frames_to_display,
                bytes_to_display = counters.bytes_to_display,
                "browser display stream ended"
            );
            on_end(end, counters);
        });
        Self { cancel, task }
    }

    /// End the stream; `on_end` still runs (with `Cancelled`).
    pub fn cancel(&self) {
        let _ = self.cancel.send(true);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        // The task observes the cancellation itself; aborting it would skip
        // `on_end`. Detach it instead of waiting.
        let _ = &self.task;
    }
}

async fn relay(
    mut endpoint: reqwest::Url,
    token: String,
    rfb_socket: PathBuf,
    mut cancelled: tokio::sync::watch::Receiver<bool>,
) -> (StreamEnd, Counters) {
    let mut counters = Counters::default();
    endpoint.set_path(STREAM_PATH);
    endpoint.set_query(None);
    let pairing = async {
        let mut request = endpoint.as_str().into_client_request()?;
        request
            .headers_mut()
            .insert(STREAM_TOKEN_HEADER, token.parse()?);
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .context("browser stream upgrade refused")?;
        let display = UnixStream::connect(&rfb_socket)
            .await
            .with_context(|| format!("connecting {}", rfb_socket.display()))?;
        anyhow::Ok((socket, display))
    };
    let paired = tokio::select! {
        _ = cancelled.changed() => return (StreamEnd::Cancelled, counters),
        paired = tokio::time::timeout(
            Duration::from_millis(u64::try_from(PAIRING_TIMEOUT_MS).unwrap_or(15_000)),
            pairing,
        ) => paired,
    };
    let (socket, mut display) = match paired {
        Ok(Ok(paired)) => paired,
        Ok(Err(error)) => return (StreamEnd::PairingFailed(format!("{error:#}")), counters),
        Err(_) => {
            return (
                StreamEnd::PairingFailed("pairing timed out".into()),
                counters,
            );
        }
    };
    let (mut sink, mut source) = socket.split();
    let mut buffer = vec![0u8; MAX_FRAME_BYTES];
    let end = loop {
        tokio::select! {
            _ = cancelled.changed() => break StreamEnd::Cancelled,
            message = source.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if bytes.len() > MAX_FRAME_BYTES {
                            break StreamEnd::Failed("viewer frame exceeds the bound".into());
                        }
                        let written = tokio::time::timeout(CREDIT_TIMEOUT, display.write_all(&bytes)).await;
                        match written {
                            Ok(Ok(())) => {
                                counters.frames_to_display += 1;
                                counters.bytes_to_display += bytes.len() as u64;
                            }
                            Ok(Err(_)) => break StreamEnd::DisplayClosed,
                            Err(_) => break StreamEnd::Failed("display did not accept input in time".into()),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if tokio::time::timeout(CREDIT_TIMEOUT, sink.send(Message::Pong(payload))).await.is_err() {
                            break StreamEnd::Failed("viewer did not accept a pong in time".into());
                        }
                    }
                    Some(Ok(Message::Pong(_) | Message::Frame(_))) => {}
                    Some(Ok(Message::Text(_))) => break StreamEnd::Failed("viewer sent a text frame".into()),
                    Some(Ok(Message::Close(_))) | None => break StreamEnd::ViewerClosed,
                    Some(Err(error)) => break StreamEnd::Failed(error.to_string()),
                }
            }
            read = display.read(&mut buffer) => {
                match read {
                    Ok(0) => break StreamEnd::DisplayClosed,
                    Ok(count) => {
                        let frame = Message::Binary(buffer[..count].to_vec().into());
                        match tokio::time::timeout(CREDIT_TIMEOUT, sink.send(frame)).await {
                            Ok(Ok(())) => {
                                counters.frames_to_viewer += 1;
                                counters.bytes_to_viewer += count as u64;
                            }
                            Ok(Err(_)) => break StreamEnd::ViewerClosed,
                            Err(_) => break StreamEnd::Failed("viewer did not accept a frame in time".into()),
                        }
                    }
                    Err(error) => break StreamEnd::Failed(error.to_string()),
                }
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), sink.send(Message::Close(None))).await;
    let _ = display.shutdown().await;
    (end, counters)
}

/// Validate the Controller base URL a stream may be opened against.
///
/// # Errors
/// Returns when the URL is not an absolute `ws`/`wss`/`http`/`https` URL.
pub fn stream_endpoint(controller_url: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(controller_url).context("invalid Controller URL")?;
    match url.scheme() {
        "https" => url
            .set_scheme("wss")
            .map_err(|()| anyhow::anyhow!("scheme"))?,
        "http" => url
            .set_scheme("ws")
            .map_err(|()| anyhow::anyhow!("scheme"))?,
        "ws" | "wss" => {}
        other => bail!("unsupported Controller URL scheme {other}"),
    }
    url.set_path(STREAM_PATH);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_endpoint_switches_to_websocket_schemes_and_the_stream_path() {
        assert_eq!(
            stream_endpoint("https://cowboy.example/api/machine/ws?x=1")
                .unwrap()
                .as_str(),
            "wss://cowboy.example/api/machine/browser-stream"
        );
        assert_eq!(
            stream_endpoint("http://127.0.0.1:3000").unwrap().as_str(),
            "ws://127.0.0.1:3000/api/machine/browser-stream"
        );
        assert!(stream_endpoint("ftp://x").is_err());
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // tungstenite's header callback signature
    async fn frames_relay_both_ways_and_a_viewer_close_ends_the_stream() {
        use tokio::io::AsyncWriteExt as _;
        let temp = tempfile::tempdir().unwrap();
        let rfb_path = temp.path().join("rfb.sock");
        let rfb = tokio::net::UnixListener::bind(&rfb_path).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let controller = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut token = None;
            let socket = tokio_tungstenite::accept_hdr_async(
                tcp,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                 response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    token = request
                        .headers()
                        .get(STREAM_TOKEN_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .map(ToOwned::to_owned);
                    assert_eq!(request.uri().path(), STREAM_PATH);
                    Ok(response)
                },
            )
            .await
            .unwrap();
            assert_eq!(token.as_deref(), Some("token-1234567890abcdef"));
            let (mut sink, mut source) = socket.split();
            sink.send(Message::Binary(b"RFB 003.008\n".to_vec().into()))
                .await
                .unwrap();
            let Some(Ok(Message::Binary(reply))) = source.next().await else {
                panic!("binary reply expected");
            };
            assert_eq!(reply.as_ref(), b"RFB 003.008\n");
            sink.send(Message::Close(None)).await.unwrap();
        });
        let display = tokio::spawn(async move {
            let (mut stream, _) = rfb.accept().await.unwrap();
            let mut greeting = [0u8; 12];
            stream.read_exact(&mut greeting).await.unwrap();
            stream.write_all(&greeting).await.unwrap();
            let mut rest = Vec::new();
            let _ = stream.read_to_end(&mut rest).await;
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _stream = Stream::start(
            reqwest::Url::parse(&format!("ws://127.0.0.1:{port}/api/machine/ws")).unwrap(),
            "token-1234567890abcdef".into(),
            rfb_path,
            Box::new(move |end, counters| {
                let _ = tx.send((end, counters));
            }),
        );
        let (end, counters) = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(end, StreamEnd::ViewerClosed);
        assert_eq!(counters.frames_to_display, 1);
        assert_eq!(counters.bytes_to_display, 12);
        assert_eq!(counters.frames_to_viewer, 1);
        controller.await.unwrap();
        display.await.unwrap();
    }
}
