//! Durable local outbox for provider usage observed by Machine-local gateways.

#![warn(clippy::pedantic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixListener;

use crate::machine_protocol::{MachineEvent, ProviderUsageEvent};

const MAX_EVENT_BYTES: usize = 16 * 1024;
const MAX_BATCH: usize = 100;

#[derive(Debug, Deserialize)]
struct GatewayUsage {
    event_id: String,
    producer_id: String,
    occurred_at_ms: i64,
    account_fingerprint: String,
    provider: String,
    agent: String,
    model: String,
    status: u16,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    cache_hit_tokens: Option<u64>,
    cache_miss_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct ProviderUsageSpool {
    connection: Arc<parking_lot::Mutex<Connection>>,
}

impl ProviderUsageSpool {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating usage spool directory {}", parent.display()))?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("opening provider usage spool {}", path.display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS producers (
               producer_id TEXT PRIMARY KEY, next_sequence INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
               producer_id TEXT NOT NULL, sequence INTEGER NOT NULL,
               event_id TEXT NOT NULL UNIQUE, payload TEXT NOT NULL,
               PRIMARY KEY (producer_id, sequence)
             );",
        )?;
        Ok(Self {
            connection: Arc::new(parking_lot::Mutex::new(connection)),
        })
    }

    fn ingest(&self, input: &[u8]) -> Result<()> {
        let mut event: GatewayUsage =
            serde_json::from_slice(input).context("decode usage event")?;
        validate(&event)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        if transaction
            .query_row(
                "SELECT 1 FROM events WHERE event_id = ?1",
                [&event.event_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            transaction.commit()?;
            return Ok(());
        }
        let sequence: i64 = transaction.query_row(
            "INSERT INTO producers (producer_id, next_sequence) VALUES (?1, 2)
                 ON CONFLICT (producer_id) DO UPDATE SET next_sequence = next_sequence + 1
                 RETURNING next_sequence - 1",
            [&event.producer_id],
            |row| row.get(0),
        )?;
        let protocol = ProviderUsageEvent {
            producer_id: event.producer_id.clone(),
            sequence: u64::try_from(sequence).context("negative usage sequence")?,
            occurred_at_ms: event.occurred_at_ms,
            account_fingerprint: std::mem::take(&mut event.account_fingerprint),
            provider: std::mem::take(&mut event.provider),
            agent: std::mem::take(&mut event.agent),
            model: std::mem::take(&mut event.model),
            status: event.status,
            input_tokens: event.input_tokens,
            output_tokens: event.output_tokens,
            reasoning_tokens: event.reasoning_tokens,
            cache_hit_tokens: event.cache_hit_tokens,
            cache_miss_tokens: event.cache_miss_tokens,
        };
        transaction.execute(
            "INSERT INTO events (producer_id, sequence, event_id, payload) VALUES (?1, ?2, ?3, ?4)",
            params![
                protocol.producer_id,
                sequence,
                event.event_id,
                serde_json::to_string(&protocol)?
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn pending_batch(&self) -> Result<Option<MachineEvent>> {
        let connection = self.connection.lock();
        let producer: Option<String> = connection
            .query_row(
                "SELECT producer_id FROM events ORDER BY sequence LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(producer_id) = producer else {
            return Ok(None);
        };
        let mut statement = connection.prepare(
            "SELECT payload FROM events WHERE producer_id = ?1 ORDER BY sequence LIMIT ?2",
        )?;
        let events = statement
            .query_map(params![producer_id, MAX_BATCH], |row| {
                row.get::<_, String>(0)
            })?
            .map(|row| serde_json::from_str::<ProviderUsageEvent>(&row?).map_err(Into::into))
            .collect::<Result<Vec<_>>>()?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        Ok(Some(MachineEvent::ProviderUsageBatch {
            producer_id: producer_id.clone(),
            first_sequence: first.sequence,
            last_sequence: events.last().map_or(first.sequence, |event| event.sequence),
            events,
        }))
    }

    pub fn acknowledge(&self, producer_id: &str, sequence: u64) -> Result<()> {
        self.connection.lock().execute(
            "DELETE FROM events WHERE producer_id = ?1 AND sequence <= ?2",
            params![producer_id, i64::try_from(sequence).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }
}

fn validate(event: &GatewayUsage) -> Result<()> {
    if event.event_id.is_empty()
        || event.event_id.len() > 128
        || event.producer_id.is_empty()
        || event.producer_id.len() > 128
        || event.provider != "deepseek"
        || !matches!(event.agent.as_str(), "codex" | "claude")
        || event.account_fingerprint.len() != 16
        || event.model.len() > 128
        || !(100..=599).contains(&event.status)
    {
        anyhow::bail!("invalid gateway usage event");
    }
    Ok(())
}

pub async fn serve(socket: PathBuf, spool: ProviderUsageSpool) -> Result<()> {
    if let Some(parent) = socket.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if socket.exists() {
        tokio::fs::remove_file(&socket).await?;
    }
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("binding provider usage socket {}", socket.display()))?;
    loop {
        let (mut stream, _) = listener.accept().await?;
        let spool = spool.clone();
        tokio::spawn(async move {
            let mut input = Vec::new();
            let accepted = match (&mut stream)
                .take(u64::try_from(MAX_EVENT_BYTES + 1).unwrap_or(u64::MAX))
                .read_to_end(&mut input)
                .await
            {
                Ok(_) if input.len() <= MAX_EVENT_BYTES => spool.ingest(&input).is_ok(),
                _ => false,
            };
            let _ = stream
                .write_all(if accepted { b"ok\n" } else { b"error\n" })
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spool() -> (ProviderUsageSpool, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "cowboy-provider-usage-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        (ProviderUsageSpool::open(&path).expect("open spool"), path)
    }

    fn event(id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": id,
            "producer_id": "codex-deepseek",
            "occurred_at_ms": 1_786_000_000_000_i64,
            "account_fingerprint": "0123456789abcdef",
            "provider": "deepseek",
            "agent": "codex",
            "model": "deepseek-chat",
            "status": 200,
            "input_tokens": 10,
            "output_tokens": 4,
            "reasoning_tokens": 1,
            "cache_hit_tokens": 7,
            "cache_miss_tokens": 3
        }))
        .expect("serialize event")
    }

    #[test]
    fn duplicate_event_is_stored_once_and_acknowledged_after_delivery() {
        let (spool, path) = spool();
        spool.ingest(&event("event-1")).expect("first ingest");
        spool.ingest(&event("event-1")).expect("duplicate ingest");
        let MachineEvent::ProviderUsageBatch {
            producer_id,
            first_sequence,
            last_sequence,
            events,
        } = spool
            .pending_batch()
            .expect("read batch")
            .expect("batch exists")
        else {
            panic!("unexpected event")
        };
        assert_eq!(producer_id, "codex-deepseek");
        assert_eq!((first_sequence, last_sequence, events.len()), (1, 1, 1));
        drop(spool);

        let reopened = ProviderUsageSpool::open(&path).expect("reopen spool");
        assert!(reopened.pending_batch().expect("replay batch").is_some());
        reopened
            .acknowledge("codex-deepseek", 1)
            .expect("acknowledge batch");
        assert!(reopened.pending_batch().expect("empty batch").is_none());
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_unknown_provider_and_invalid_status() {
        let (spool, path) = spool();
        let mut value: serde_json::Value =
            serde_json::from_slice(&event("event-2")).expect("parse event");
        value["provider"] = "not-deepseek".into();
        value["status"] = 99.into();
        assert!(spool.ingest(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(spool.pending_batch().expect("empty batch").is_none());
        drop(spool);
        let _ = std::fs::remove_file(path);
    }
}
