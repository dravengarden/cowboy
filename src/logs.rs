//! Host-local OTel evidence and bounded, read-only diagnostic queries.
//!
//! Storage and retrieval are ports. Neither admission nor a network connection
//! is a durability receipt. Business events, credentials and agent output are
//! never captured by the tracing bridge.

mod analysis;
mod capture;
pub mod cli;
mod data;
mod source;
pub(crate) mod storage;
#[cfg(test)]
mod tests;

#[cfg(feature = "machine-host")]
pub(crate) use capture::provider_usage;
#[cfg(any(feature = "full", feature = "machine-host"))]
pub(crate) use capture::runtime_spans;
pub use capture::{Context, Guard, directory, init};
#[cfg(any(feature = "full", feature = "machine-host"))]
pub(crate) use capture::{forward_runtime, init_stderr};
pub(crate) use data::now_ms;
#[cfg(feature = "full")]
pub(crate) use storage::SqliteStore;

/// The existing JSONL writer and the indexed OTel store accept the same already
/// sanitized telemetry records. Remote delivery retains its independent queue.
#[cfg(feature = "full")]
pub(crate) trait EvidenceSink: Send {
    fn write(&mut self, records: &str, now_ms: i64) -> anyhow::Result<()>;
    fn maintain(&mut self, now_ms: i64) -> anyhow::Result<()>;
}

#[cfg(feature = "full")]
impl<T: EvidenceSink + ?Sized> EvidenceSink for Box<T> {
    fn write(&mut self, records: &str, now: i64) -> anyhow::Result<()> {
        (**self).write(records, now)
    }
    fn maintain(&mut self, now: i64) -> anyhow::Result<()> {
        (**self).maintain(now)
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum LocalBackend {
    Sqlite,
    Jsonl,
    Both,
}

#[cfg(feature = "full")]
pub(crate) struct Fanout(pub Vec<Box<dyn EvidenceSink>>);
#[cfg(feature = "full")]
impl EvidenceSink for Fanout {
    fn write(&mut self, records: &str, now: i64) -> anyhow::Result<()> {
        let mut failure = None;
        for sink in &mut self.0 {
            if let Err(e) = sink.write(records, now) {
                failure = Some(e);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    fn maintain(&mut self, now: i64) -> anyhow::Result<()> {
        let mut failure = None;
        for sink in &mut self.0 {
            if let Err(e) = sink.maintain(now) {
                failure = Some(e);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
