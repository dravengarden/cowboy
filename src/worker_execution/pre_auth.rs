//! Pre-authentication HTTP request-line classification and bounded handshake
//! framing for the execution Unix/TCP endpoint.

use std::sync::Arc;

use anyhow::{Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};

use super::DIAGNOSTICS_PATH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestLineRoute {
    Mutation,
    Diagnostics,
    Invalid,
}

pub(super) fn classify_request_line(prefix: &[u8]) -> RequestLineRoute {
    let Ok(line) = std::str::from_utf8(prefix) else {
        return RequestLineRoute::Invalid;
    };
    let Some(line) = line.strip_suffix("\r\n") else {
        return RequestLineRoute::Invalid;
    };
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return RequestLineRoute::Invalid;
    };
    if method != "GET" || target.is_empty() || !version.starts_with("HTTP/1.") {
        return RequestLineRoute::Invalid;
    }
    let Ok(uri) = target.parse::<tokio_tungstenite::tungstenite::http::Uri>() else {
        return RequestLineRoute::Invalid;
    };
    if uri.path() == DIAGNOSTICS_PATH {
        RequestLineRoute::Diagnostics
    } else {
        RequestLineRoute::Mutation
    }
}

pub(super) async fn peek_request_line<S>(socket: &mut S) -> Result<(Vec<u8>, RequestLineRoute)>
where
    S: AsyncRead + Unpin,
{
    let mut prefix = Vec::with_capacity(128);
    let mut byte = [0_u8; 1];
    while prefix.len() < 256 {
        let n = socket.read(&mut byte).await?;
        if n == 0 {
            bail!("execution socket closed before HTTP request line");
        }
        prefix.push(byte[0]);
        if prefix.ends_with(b"\r\n") {
            let route = classify_request_line(&prefix);
            return Ok((prefix, route));
        }
    }
    bail!("execution HTTP request line exceeded 256 bytes");
}

/// Limits pre-upgrade HTTP header reads to `MAX_PRE_AUTH_HEADER_BYTES` and
/// replays the peeked request-line prefix so `/diagnostics` gets its tight
/// 64 KiB `WebSocketConfig` at the tungstenite parser boundary.
pub(super) struct PrefixedHandshakeStream<S> {
    pub(super) inner: S,
    pub(super) prefix: Vec<u8>,
    pub(super) prefix_pos: usize,
    pub(super) pre_auth_remaining: usize,
    pub(super) init_remaining: usize,
    pub(super) upgraded: Arc<std::sync::atomic::AtomicBool>,
    pub(super) initialized: Arc<std::sync::atomic::AtomicBool>,
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedHandshakeStream<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.prefix_pos < this.prefix.len() {
            let take = (this.prefix.len() - this.prefix_pos).min(buf.remaining());
            buf.put_slice(&this.prefix[this.prefix_pos..this.prefix_pos + take]);
            this.prefix_pos += take;
            return std::task::Poll::Ready(Ok(()));
        }
        if this.initialized.load(std::sync::atomic::Ordering::Acquire) {
            return std::pin::Pin::new(&mut this.inner).poll_read(cx, buf);
        }
        let upgraded = this.upgraded.load(std::sync::atomic::Ordering::Acquire);
        let (remaining, err_msg) = if upgraded {
            (
                &mut this.init_remaining,
                "WebSocket initialization frame exceeded 4 KiB limit",
            )
        } else {
            (
                &mut this.pre_auth_remaining,
                "pre-auth HTTP handshake exceeded 4 KiB limit",
            )
        };
        if *remaining == 0 {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err_msg,
            )));
        }
        let max = buf.remaining().min(*remaining);
        let read_res = {
            let mut sub = buf.take(max);
            let before = sub.filled().len();
            match std::pin::Pin::new(&mut this.inner).poll_read(cx, &mut sub) {
                std::task::Poll::Ready(Ok(())) => {
                    std::task::Poll::Ready(Ok(sub.filled().len() - before))
                }
                std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        };
        match read_res {
            std::task::Poll::Ready(Ok(n)) => {
                buf.advance(n);
                *remaining = remaining.saturating_sub(n);
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedHandshakeStream<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
