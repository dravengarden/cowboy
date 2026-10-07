//! Connection-local framing for slow Machine links. Negotiated in the HTTPS
//! upgrade, independently of the retained worker protocol and durable codecs.
//! Only heartbeats may overtake bulk data; application frames retain FIFO order.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{Result, ensure};
use futures::{Sink, SinkExt};
use tokio::sync::mpsc;

pub(crate) const HEADER: &str = "x-cowboy-machine-transport";
pub(crate) const CHUNKED: &str = "chunks-v1";
const CHUNK_BYTES: usize = 16 * 1024;
const MAX_BYTES: usize = crate::runtime_wire::MAX_FRAME_BYTES + 1024;

pub(crate) fn heartbeat(text: &str) -> bool {
    text.starts_with("{\"type\":\"heartbeat\",")
        || text == "{\"type\":\"runtime\",\"frame\":{\"type\":\"heartbeat\"}}"
}

pub(crate) trait Message: Send {
    fn text(&self) -> Option<&str>;
    fn binary(bytes: Vec<u8>) -> Self;
    fn urgent(&self) -> bool;
}

macro_rules! message {
    ($message:ty) => {
        impl Message for $message {
            fn text(&self) -> Option<&str> {
                match self {
                    Self::Text(text) => Some(text.as_str()),
                    _ => None,
                }
            }
            fn binary(bytes: Vec<u8>) -> Self {
                Self::Binary(bytes.into())
            }
            fn urgent(&self) -> bool {
                matches!(self, Self::Ping(_) | Self::Pong(_) | Self::Close(_))
                    || self.text().is_some_and(heartbeat)
            }
        }
    };
}
message!(tokio_tungstenite::tungstenite::Message);
#[cfg(feature = "full")]
message!(axum::extract::ws::Message);

async fn send<S, M>(sink: &mut S, message: M) -> Result<()>
where
    S: Sink<M> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(15), sink.send(message))
        .await
        .map_err(|_| anyhow::anyhow!("Machine transport chunk send timed out"))?
        .map_err(|_| anyhow::anyhow!("Machine transport send failed"))
}

pub(crate) async fn write<S, M>(
    mut sink: S,
    mut incoming: mpsc::UnboundedReceiver<M>,
    chunked: bool,
) -> Result<()>
where
    S: Sink<M> + Unpin,
    M: Message,
{
    let mut queued = VecDeque::new();
    loop {
        let message = match queued.pop_front() {
            Some(message) => message,
            None => match incoming.recv().await {
                Some(message) => message,
                None => return Ok(()),
            },
        };
        let text = message
            .text()
            .filter(|text| chunked && text.len() > CHUNK_BYTES);
        let Some(text) = text else {
            send(&mut sink, message).await?;
            continue;
        };
        ensure!(
            text.len() <= MAX_BYTES,
            "Machine transport frame exceeds limit"
        );
        let total = u32::try_from(text.len())?;
        for (index, bytes) in text.as_bytes().chunks(CHUNK_BYTES).enumerate() {
            // Heartbeats/control frames cannot wait behind a multi-MiB replay.
            // Bound each drain so a busy producer cannot starve this transfer.
            for _ in 0..1024 {
                let Ok(next) = incoming.try_recv() else { break };
                if next.urgent() {
                    send(&mut sink, next).await?;
                } else {
                    queued.push_back(next);
                }
            }
            let offset = u32::try_from(index * CHUNK_BYTES)?;
            let mut chunk = Vec::with_capacity(8 + bytes.len());
            chunk.extend_from_slice(&total.to_be_bytes());
            chunk.extend_from_slice(&offset.to_be_bytes());
            chunk.extend_from_slice(bytes);
            send(&mut sink, M::binary(chunk)).await?;
            tokio::task::yield_now().await;
        }
    }
}

#[derive(Default)]
pub(crate) struct Decoder {
    bytes: Vec<u8>,
    total: usize,
}

impl Decoder {
    pub(crate) fn text(&self, text: &str) -> Result<()> {
        ensure!(
            self.bytes.is_empty() || heartbeat(text),
            "interleaved Machine data frame"
        );
        Ok(())
    }

    pub(crate) fn chunk(&mut self, chunked: bool, bytes: &[u8]) -> Result<Option<String>> {
        ensure!(chunked, "Machine chunk transport was not negotiated");
        ensure!(
            (9..=CHUNK_BYTES + 8).contains(&bytes.len()),
            "invalid Machine chunk size"
        );
        let total = u32::from_be_bytes(bytes[..4].try_into()?) as usize;
        let offset = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
        ensure!(
            total <= MAX_BYTES && total > CHUNK_BYTES,
            "invalid Machine frame size"
        );
        ensure!(offset == self.bytes.len(), "out-of-order Machine chunk");
        if offset == 0 {
            self.total = total;
        }
        ensure!(
            total == self.total && offset + bytes.len() - 8 <= total,
            "invalid Machine chunk boundary"
        );
        self.bytes.extend_from_slice(&bytes[8..]);
        if self.bytes.len() != total {
            return Ok(None);
        }
        self.total = 0;
        Ok(Some(String::from_utf8(std::mem::take(&mut self.bytes))?))
    }
}

#[cfg(test)]
mod tests;
