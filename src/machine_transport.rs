//! Connection-local framing for slow Machine links. Negotiated in the HTTPS
//! upgrade, independently of the retained worker protocol and durable codecs.
//! Only heartbeats may overtake bulk data; application frames retain FIFO order.

use std::collections::VecDeque;
use std::io::Write;
use std::time::Duration;

use anyhow::{Result, ensure};
use futures::{Sink, SinkExt};
use tokio::sync::mpsc;

pub(crate) const HEADER: &str = "x-cowboy-machine-transport";
pub(crate) const CHUNKED: &str = "chunks-v1";
pub(crate) const COMPRESSION_HEADER: &str = "x-cowboy-machine-compression";
pub(crate) const DEFLATE: &str = "zlib-v1";
const CHUNK_BYTES: usize = 16 * 1024;
const MAX_BYTES: usize = crate::runtime_wire::MAX_FRAME_BYTES + 1024;
const COMPRESSED: u32 = 1 << 31;

#[derive(Clone, Copy)]
pub(crate) struct Features {
    pub chunked: bool,
    pub compressed: bool,
}

impl From<bool> for Features {
    fn from(chunked: bool) -> Self {
        Self {
            chunked,
            compressed: false,
        }
    }
}

impl From<(bool, bool)> for Features {
    fn from((chunked, compressed): (bool, bool)) -> Self {
        Self {
            chunked,
            compressed: chunked && compressed,
        }
    }
}

fn compress(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = flate2::Decompress::new(true);
    let mut output = Vec::new();
    let mut input = bytes;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let status = decoder.decompress(input, &mut buffer, flate2::FlushDecompress::None)?;
        let consumed = (decoder.total_in() - before.0) as usize;
        let produced = (decoder.total_out() - before.1) as usize;
        ensure!(
            output.len() + produced <= MAX_BYTES,
            "inflated Machine frame exceeds limit"
        );
        output.extend_from_slice(&buffer[..produced]);
        input = &input[consumed..];
        if status == flate2::Status::StreamEnd {
            ensure!(input.is_empty(), "trailing compressed Machine data");
            return Ok(output);
        }
        ensure!(
            consumed > 0 || produced > 0,
            "incomplete compressed Machine frame"
        );
    }
}

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
    features: impl Into<Features>,
) -> Result<()>
where
    S: Sink<M> + Unpin,
    M: Message,
{
    let features = features.into();
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
            .filter(|text| features.chunked && text.len() > CHUNK_BYTES);
        let Some(text) = text else {
            // Small-frame replays also build a backlog. Heartbeats arriving
            // while that backlog drains must overtake it, just as they do
            // between chunks of a large frame. Application data stays FIFO.
            for _ in 0..1024 {
                let Ok(next) = incoming.try_recv() else { break };
                if next.urgent() {
                    send(&mut sink, next).await?;
                } else {
                    queued.push_back(next);
                }
            }
            send(&mut sink, message).await?;
            // A ready sink and the private queue need not yield on their own.
            // Let heartbeat producers run before admitting the next frame.
            tokio::task::yield_now().await;
            continue;
        };
        ensure!(
            text.len() <= MAX_BYTES,
            "Machine transport frame exceeds limit"
        );
        // Per-message dictionaries avoid cross-message state. Compression is
        // off the async executor, bounded by the existing plaintext frame cap.
        // A slow link otherwise holds every small tool reply behind this data.
        let encoded = if features.compressed {
            let input = text.as_bytes().to_vec();
            let compressed = tokio::task::spawn_blocking(move || compress(&input)).await??;
            (compressed.len() < text.len() * 9 / 10).then_some(compressed)
        } else {
            None
        };
        let payload = encoded.as_deref().unwrap_or(text.as_bytes());
        let total = u32::try_from(payload.len())? | if encoded.is_some() { COMPRESSED } else { 0 };
        for (index, bytes) in payload.chunks(CHUNK_BYTES).enumerate() {
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
    compressed: bool,
}

impl Decoder {
    pub(crate) fn text(&self, text: &str) -> Result<()> {
        ensure!(
            self.bytes.is_empty() || heartbeat(text),
            "interleaved Machine data frame"
        );
        Ok(())
    }

    pub(crate) async fn chunk(
        &mut self,
        features: impl Into<Features>,
        bytes: &[u8],
    ) -> Result<Option<String>> {
        let features = features.into();
        ensure!(
            features.chunked,
            "Machine chunk transport was not negotiated"
        );
        ensure!(
            (9..=CHUNK_BYTES + 8).contains(&bytes.len()),
            "invalid Machine chunk size"
        );
        let header = u32::from_be_bytes(bytes[..4].try_into()?);
        let compressed = header & COMPRESSED != 0;
        ensure!(
            !compressed || features.compressed,
            "Machine compression was not negotiated"
        );
        let total = (header & !COMPRESSED) as usize;
        let offset = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
        ensure!(
            total <= MAX_BYTES && (total > CHUNK_BYTES || (compressed && total > 0)),
            "invalid Machine frame size"
        );
        ensure!(offset == self.bytes.len(), "out-of-order Machine chunk");
        if offset == 0 {
            self.total = total;
            self.compressed = compressed;
        }
        ensure!(
            total == self.total
                && compressed == self.compressed
                && offset + bytes.len() - 8 <= total,
            "invalid Machine chunk boundary"
        );
        self.bytes.extend_from_slice(&bytes[8..]);
        if self.bytes.len() != total {
            return Ok(None);
        }
        self.total = 0;
        let bytes = std::mem::take(&mut self.bytes);
        Ok(Some(String::from_utf8(if compressed {
            tokio::task::spawn_blocking(move || decompress(&bytes)).await??
        } else {
            bytes
        })?))
    }
}

#[cfg(test)]
mod tests;
