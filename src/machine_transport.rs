//! Connection-local framing for slow Machine links. Negotiated in the HTTPS
//! upgrade, independently of the retained worker protocol and durable codecs.
//! Only heartbeats may overtake bulk data; application frames retain FIFO order.
//! With pacing, small execution requests and replies (independent RPCs) also
//! overtake a chunked frame, and the receiver's credit bounds the chunk bytes
//! buffered below this writer: a single-flow overlay link otherwise holds
//! every session's small reply behind megabytes already in socket buffers.
//! With streaming compression, small frames share one deflate context per
//! direction, so the binding and envelope every RPC repeats cost a few bytes.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, ensure};
use futures::{Sink, SinkExt};
use tokio::sync::{Notify, mpsc};

pub(crate) const HEADER: &str = "x-cowboy-machine-transport";
pub(crate) const CHUNKED: &str = "chunks-v1";
pub(crate) const COMPRESSION_HEADER: &str = "x-cowboy-machine-compression";
pub(crate) const DEFLATE: &str = "zlib-v1";
pub(crate) const PACING_HEADER: &str = "x-cowboy-machine-pacing";
pub(crate) const CREDIT: &str = "credit-v1";
pub(crate) const STREAM_HEADER: &str = "x-cowboy-machine-stream";
pub(crate) const STREAM: &str = "deflate-stream-v1";
const CHUNK_BYTES: usize = 16 * 1024;
// Chunk bytes sent beyond the receiver's credit: at the measured ~100 KiB/s
// overlay a small frame waits about a second behind bulk, while a 0.2-1 s
// round trip still allows 128-640 KiB/s of bulk.
const CREDIT_WINDOW: u64 = 128 * 1024;
// No credit progress for this long while bulk waits means a dead link.
const CREDIT_STALL: Duration = Duration::from_secs(60);
const CREDIT_PREFIX: &[u8] = b"cowboy-credit-v1:";
const MAX_BYTES: usize = crate::runtime_wire::MAX_FRAME_BYTES + 1024;
const COMPRESSED: u32 = 1 << 31;
/// A small frame compressed in the connection's shared deflate stream: one
/// binary message, decoded in arrival order, never part of a chunked frame.
const STREAMED: u32 = 1 << 30;

#[derive(Clone, Copy)]
pub(crate) struct Features {
    pub chunked: bool,
    pub compressed: bool,
    pub paced: bool,
    pub stream: bool,
}

impl From<bool> for Features {
    fn from(chunked: bool) -> Self {
        Self {
            chunked,
            compressed: false,
            paced: false,
            stream: false,
        }
    }
}

impl From<(bool, bool)> for Features {
    fn from((chunked, compressed): (bool, bool)) -> Self {
        Self {
            chunked,
            compressed: chunked && compressed,
            paced: false,
            stream: false,
        }
    }
}

impl From<(bool, bool, bool)> for Features {
    fn from((chunked, compressed, paced): (bool, bool, bool)) -> Self {
        Self::from((chunked, compressed, paced, false))
    }
}

impl From<(bool, bool, bool, bool)> for Features {
    fn from((chunked, compressed, paced, stream): (bool, bool, bool, bool)) -> Self {
        Self {
            chunked,
            compressed: chunked && compressed,
            paced: chunked && paced,
            stream: chunked && stream,
        }
    }
}

/// The sending half of a streaming-compressed connection.
struct StreamEncoder(flate2::Compress);

impl StreamEncoder {
    fn new() -> Self {
        Self(flate2::Compress::new(flate2::Compression::new(6), false))
    }

    /// Compress one frame with a sync flush, so the peer can decode it on
    /// arrival using everything sent before it.
    fn frame(&mut self, text: &str) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(8 + text.len() / 2 + 64);
        out.extend_from_slice(&[0; 8]);
        let mut input = text.as_bytes();
        loop {
            if out.capacity() - out.len() < 64 {
                out.reserve(out.capacity());
            }
            let before = self.0.total_in();
            self.0
                .compress_vec(input, &mut out, flate2::FlushCompress::Sync)?;
            let consumed = usize::try_from(self.0.total_in() - before)?;
            input = &input[consumed..];
            // A sync flush is complete once all input is taken and the
            // output buffer was not filled to the brim.
            if input.is_empty() && out.len() < out.capacity() {
                break;
            }
        }
        let total = u32::try_from(out.len() - 8)?;
        ensure!(
            total & (COMPRESSED | STREAMED) == 0,
            "streamed frame too large"
        );
        out[..4].copy_from_slice(&(total | STREAMED).to_be_bytes());
        Ok(out)
    }
}

/// The receiving half: inflate a streamed frame, bounded to a small frame.
struct StreamDecoder(flate2::Decompress);

impl StreamDecoder {
    fn frame(&mut self, bytes: &[u8]) -> Result<String> {
        let mut out = Vec::with_capacity(bytes.len() * 4 + 64);
        let mut input = bytes;
        loop {
            if out.capacity() - out.len() < 64 {
                ensure!(
                    out.len() <= CHUNK_BYTES,
                    "streamed Machine frame exceeds limit"
                );
                out.reserve(out.capacity());
            }
            let (before_in, before_out) = (self.0.total_in(), self.0.total_out());
            let status = self
                .0
                .decompress_vec(input, &mut out, flate2::FlushDecompress::Sync)?;
            ensure!(
                status != flate2::Status::StreamEnd,
                "streamed Machine compression ended"
            );
            let consumed = usize::try_from(self.0.total_in() - before_in)?;
            let produced = self.0.total_out() - before_out;
            input = &input[consumed..];
            if input.is_empty() && out.len() < out.capacity() {
                break;
            }
            ensure!(
                consumed > 0 || produced > 0,
                "truncated streamed Machine frame"
            );
        }
        ensure!(
            out.len() <= CHUNK_BYTES,
            "streamed Machine frame exceeds limit"
        );
        Ok(String::from_utf8(out)?)
    }
}

/// Chunk bytes the peer has decoded, from its credit messages. The reader of
/// a paced connection records them; its writer waits on them.
#[derive(Default)]
pub(crate) struct Credit {
    acknowledged: AtomicU64,
    changed: Notify,
}

impl Credit {
    /// Record a credit message; false when `payload` is not one.
    pub(crate) fn observe(&self, payload: &[u8]) -> bool {
        let Some(total) = payload
            .strip_prefix(CREDIT_PREFIX)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| digits.parse::<u64>().ok())
        else {
            return false;
        };
        self.acknowledged.fetch_max(total, Ordering::AcqRel);
        self.changed.notify_waiters();
        true
    }

    fn acknowledged(&self) -> u64 {
        self.acknowledged.load(Ordering::Acquire)
    }
}

/// Small, mutually independent RPC frames: each names its own request and the
/// receiver forwards them concurrently, so their order relative to a large
/// frame carries no meaning. Worker events and acks keep their order.
fn overtakes(text: &str) -> bool {
    text.len() <= CHUNK_BYTES
        && (text.starts_with("{\"type\":\"runtime\",\"frame\":{\"type\":\"execution_request\"")
            || text.starts_with("{\"type\":\"runtime\",\"frame\":{\"type\":\"execution_reply\""))
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
    fn pong(bytes: Vec<u8>) -> Self;
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
            fn pong(bytes: Vec<u8>) -> Self {
                Self::Pong(bytes.into())
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

/// Write `incoming` to `sink`, pacing chunked frames by `credit` when the
/// connection negotiated it (both `features.paced` and a credit are required).
pub(crate) async fn write<S, M>(
    mut sink: S,
    mut incoming: mpsc::UnboundedReceiver<M>,
    features: impl Into<Features>,
    credit: Option<std::sync::Arc<Credit>>,
) -> Result<()>
where
    S: Sink<M> + Unpin,
    M: Message,
{
    let features = features.into();
    let credit = credit.filter(|_| features.paced);
    let first = |message: &M| {
        message.urgent() || (credit.is_some() && message.text().is_some_and(overtakes))
    };
    let mut sent_chunks = 0_u64;
    let mut open = true;
    let mut queued = VecDeque::new();
    let mut stream = features.stream.then(StreamEncoder::new);
    // Every frame that is not a chunk leaves through here, in wire order,
    // which is also the order the peer's stream decoder consumes them.
    macro_rules! emit {
        ($message:expr) => {{
            let message = $message;
            match (&mut stream, message.text()) {
                (Some(encoder), Some(text)) if !message.urgent() && text.len() <= CHUNK_BYTES => {
                    let frame = encoder.frame(text)?;
                    send(&mut sink, M::binary(frame)).await
                }
                _ => send(&mut sink, message).await,
            }
        }};
    }
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
                    emit!(next)?;
                } else {
                    queued.push_back(next);
                }
            }
            emit!(message)?;
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
                if first(&next) {
                    emit!(next)?;
                } else {
                    queued.push_back(next);
                }
            }
            // Paced: hold this chunk until the receiver has decoded all but a
            // window of earlier ones, still sending frames allowed first.
            if let Some(credit) = credit.as_deref() {
                let mut progress = (credit.acknowledged(), tokio::time::Instant::now());
                loop {
                    let changed = credit.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let acknowledged = credit.acknowledged();
                    if sent_chunks.saturating_sub(acknowledged) < CREDIT_WINDOW {
                        break;
                    }
                    if acknowledged != progress.0 {
                        progress = (acknowledged, tokio::time::Instant::now());
                    }
                    tokio::select! {
                        () = &mut changed => {}
                        next = incoming.recv(), if open => match next {
                            Some(next) if first(&next) => emit!(next)?,
                            Some(next) => queued.push_back(next),
                            None => open = false,
                        },
                        () = tokio::time::sleep_until(progress.1 + CREDIT_STALL) => {
                            anyhow::bail!("Machine transport credit stalled");
                        }
                    }
                }
            }
            let offset = u32::try_from(index * CHUNK_BYTES)?;
            let mut chunk = Vec::with_capacity(8 + bytes.len());
            chunk.extend_from_slice(&total.to_be_bytes());
            chunk.extend_from_slice(&offset.to_be_bytes());
            chunk.extend_from_slice(bytes);
            send(&mut sink, M::binary(chunk)).await?;
            sent_chunks += bytes.len() as u64;
            tokio::task::yield_now().await;
        }
    }
}

#[derive(Default)]
pub(crate) struct Decoder {
    bytes: Vec<u8>,
    total: usize,
    compressed: bool,
    paced: bool,
    received: u64,
    /// Chunk bytes arrived since the last credit was returned.
    credit_due: bool,
    stream: Option<StreamDecoder>,
}

impl Decoder {
    /// A decoder for a connection's negotiated pacing and stream compression.
    pub(crate) fn new(features: impl Into<Features>) -> Self {
        let features = features.into();
        Self {
            paced: features.paced,
            stream: features
                .stream
                .then(|| StreamDecoder(flate2::Decompress::new(false))),
            ..Self::default()
        }
    }

    pub(crate) fn text(&self, text: &str) -> Result<()> {
        ensure!(
            self.bytes.is_empty() || heartbeat(text) || (self.paced && overtakes(text)),
            "interleaved Machine data frame"
        );
        Ok(())
    }

    /// The credit to return for the chunks decoded so far, when paced. Send it
    /// as an urgent message after each chunk.
    pub(crate) fn credit<M: Message>(&mut self) -> Option<M> {
        // Streamed small frames are not chunk bytes and earn no credit.
        if !std::mem::take(&mut self.credit_due) {
            return None;
        }
        self.paced
            .then(|| M::pong([CREDIT_PREFIX, self.received.to_string().as_bytes()].concat()))
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
        ensure!(bytes.len() >= 9, "invalid Machine chunk size");
        let header = u32::from_be_bytes(bytes[..4].try_into()?);
        if header & STREAMED != 0 {
            let decoder = self
                .stream
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Machine stream compression was not negotiated"))?;
            ensure!(
                header & COMPRESSED == 0
                    && (header & !STREAMED) as usize == bytes.len() - 8
                    && bytes[4..8] == [0; 4],
                "invalid streamed Machine frame"
            );
            let text = decoder.frame(&bytes[8..])?;
            self.text(&text)?;
            return Ok(Some(text));
        }
        ensure!(bytes.len() <= CHUNK_BYTES + 8, "invalid Machine chunk size");
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
        self.received += (bytes.len() - 8) as u64;
        self.credit_due = true;
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
