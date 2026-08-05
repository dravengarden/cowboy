//! Durable local outbox for provider usage observed by Machine-local gateways.

#![warn(clippy::pedantic)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixListener;

use crate::machine_protocol::{
    MachineEvent, PROVIDER_USAGE_MAX_DURATION_MS, PROVIDER_USAGE_MAX_REQUEST_BYTES,
    PROVIDER_USAGE_MAX_SHAPE_COUNT, PROVIDER_USAGE_MAX_TOKENS, ProviderUsageEvent,
};

const MAX_EVENT_BYTES: usize = 16 * 1024;
const MAX_BATCH: usize = 100;

#[derive(Debug, Deserialize)]
struct GatewayUsage {
    #[serde(default = "default_schema_version")]
    schema_version: u16,
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
    #[serde(default = "legacy_dimension")]
    operation: String,
    #[serde(default = "legacy_dimension")]
    protocol: String,
    #[serde(default = "legacy_dimension")]
    cache_observation: String,
    #[serde(default)]
    usage_observed: Option<bool>,
    #[serde(default)]
    completed: Option<bool>,
    #[serde(default)]
    streaming: Option<bool>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    request_bytes: Option<u64>,
    #[serde(default)]
    input_item_count: Option<u64>,
    #[serde(default)]
    tool_count: Option<u64>,
    #[serde(default)]
    system_block_count: Option<u64>,
    #[serde(default)]
    has_previous_response_id: Option<bool>,
    #[serde(default)]
    compatibility_fixes: Option<u64>,
}

const fn default_schema_version() -> u16 {
    1
}

fn legacy_dimension() -> String {
    "legacy".to_owned()
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
            schema_version: event.schema_version,
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
            operation: std::mem::take(&mut event.operation),
            protocol: std::mem::take(&mut event.protocol),
            cache_observation: std::mem::take(&mut event.cache_observation),
            usage_observed: event.usage_observed,
            completed: event.completed,
            streaming: event.streaming,
            duration_ms: event.duration_ms,
            request_bytes: event.request_bytes,
            input_item_count: event.input_item_count,
            tool_count: event.tool_count,
            system_block_count: event.system_block_count,
            has_previous_response_id: event.has_previous_response_id,
            compatibility_fixes: event.compatibility_fixes,
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

fn metrics_within_bounds(event: &GatewayUsage) -> bool {
    ![
        event.input_tokens,
        event.output_tokens,
        event.reasoning_tokens,
        event.cache_hit_tokens,
        event.cache_miss_tokens,
    ]
    .into_iter()
    .flatten()
    .any(|value| value > PROVIDER_USAGE_MAX_TOKENS)
        && event
            .duration_ms
            .is_none_or(|value| value <= PROVIDER_USAGE_MAX_DURATION_MS)
        && event
            .request_bytes
            .is_none_or(|value| value <= PROVIDER_USAGE_MAX_REQUEST_BYTES)
        && ![
            event.input_item_count,
            event.tool_count,
            event.system_block_count,
            event.compatibility_fixes,
        ]
        .into_iter()
        .flatten()
        .any(|value| value > PROVIDER_USAGE_MAX_SHAPE_COUNT)
}

fn validate(event: &GatewayUsage) -> Result<()> {
    if event.event_id.is_empty()
        || event.event_id.len() > 128
        || event.producer_id.is_empty()
        || event.producer_id.len() > 128
        || event.provider != "deepseek"
        || !matches!(event.agent.as_str(), "codex" | "claude")
        || event.account_fingerprint.len() != 16
        || !event
            .account_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || event.model.len() > 128
        || !(100..=599).contains(&event.status)
        || !matches!(event.schema_version, 1 | 2)
        || !matches!(
            event.operation.as_str(),
            "legacy" | "responses" | "compact" | "messages"
        )
        || !matches!(
            event.protocol.as_str(),
            "legacy" | "responses" | "chat_completions" | "anthropic_messages"
        )
        || !matches!(
            event.cache_observation.as_str(),
            "legacy" | "absent" | "derived" | "explicit"
        )
        || !matches!(
            (event.producer_id.as_str(), event.agent.as_str()),
            ("codex-deepseek", "codex") | ("claude-deepseek", "claude")
        )
        || !metrics_within_bounds(event)
    {
        anyhow::bail!("invalid gateway usage event");
    }
    if event.schema_version == 2
        && (event.operation == "legacy"
            || event.protocol == "legacy"
            || event.cache_observation == "legacy"
            || event.usage_observed.is_none()
            || event.completed.is_none()
            || event.streaming.is_none()
            || event.duration_ms.is_none()
            || event.request_bytes.is_none()
            || event.input_item_count.is_none()
            || event.tool_count.is_none()
            || event.system_block_count.is_none()
            || event.has_previous_response_id.is_none()
            || event.compatibility_fixes.is_none())
    {
        anyhow::bail!("incomplete version two gateway usage event");
    }
    if event.schema_version == 2
        && !matches!(
            (
                event.agent.as_str(),
                event.operation.as_str(),
                event.protocol.as_str()
            ),
            (
                "codex",
                "responses" | "compact",
                "responses" | "chat_completions"
            ) | ("claude", "messages", "anthropic_messages")
        )
    {
        anyhow::bail!("inconsistent gateway usage dimensions");
    }
    match event.cache_observation.as_str() {
        "absent" if event.cache_hit_tokens.is_some() || event.cache_miss_tokens.is_some() => {
            anyhow::bail!("cache counters must be absent without an observation");
        }
        "derived" | "explicit"
            if event.usage_observed != Some(true)
                || event.cache_hit_tokens.is_none()
                || event.cache_miss_tokens.is_none() =>
        {
            anyhow::bail!("measured cache observations require complete counters");
        }
        _ => {}
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
    tokio::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))
        .await
        .with_context(|| format!("setting provider usage socket mode {}", socket.display()))?;
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
            "schema_version": 2,
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
            "cache_miss_tokens": 3,
            "operation": "responses",
            "protocol": "responses",
            "cache_observation": "derived",
            "usage_observed": true,
            "completed": true,
            "streaming": true,
            "duration_ms": 42,
            "request_bytes": 123,
            "input_item_count": 2,
            "tool_count": 1,
            "system_block_count": 1,
            "has_previous_response_id": true,
            "compatibility_fixes": 0
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
        assert_eq!(events[0].operation, "responses");
        assert_eq!(events[0].cache_observation, "derived");
        assert_eq!(events[0].request_bytes, Some(123));
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

    #[test]
    fn rejects_unknown_producer_and_lane_mismatch() {
        let (spool, path) = spool();
        for (producer, agent) in [("custom-deepseek", "codex"), ("codex-deepseek", "claude")] {
            let mut value: serde_json::Value =
                serde_json::from_slice(&event(producer)).expect("parse event");
            value["producer_id"] = producer.into();
            value["agent"] = agent.into();
            assert!(spool.ingest(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(spool.pending_batch().expect("empty batch").is_none());
        drop(spool);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_metrics_above_supported_bounds() {
        let (spool, path) = spool();
        let mut value: serde_json::Value =
            serde_json::from_slice(&event("oversized-event")).expect("parse event");
        value["cache_hit_tokens"] = (PROVIDER_USAGE_MAX_TOKENS + 1).into();
        assert!(spool.ingest(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(spool.pending_batch().expect("empty batch").is_none());
        drop(spool);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn accepts_legacy_events_without_version_two_dimensions() {
        let (spool, path) = spool();
        let mut value: serde_json::Value =
            serde_json::from_slice(&event("legacy-event")).expect("parse event");
        for key in [
            "schema_version",
            "operation",
            "protocol",
            "cache_observation",
            "usage_observed",
            "completed",
            "streaming",
            "duration_ms",
            "request_bytes",
            "input_item_count",
            "tool_count",
            "system_block_count",
            "has_previous_response_id",
            "compatibility_fixes",
        ] {
            value.as_object_mut().expect("event object").remove(key);
        }
        spool
            .ingest(&serde_json::to_vec(&value).expect("legacy JSON"))
            .expect("legacy event accepted");
        let MachineEvent::ProviderUsageBatch { events, .. } = spool
            .pending_batch()
            .expect("read batch")
            .expect("batch exists")
        else {
            panic!("unexpected event")
        };
        assert_eq!(events[0].schema_version, 1);
        assert_eq!(events[0].operation, "legacy");
        assert_eq!(events[0].completed, None);
        drop(spool);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_cache_counters_without_observation() {
        let (spool, path) = spool();
        let mut value: serde_json::Value =
            serde_json::from_slice(&event("invalid-cache")).expect("parse event");
        value["cache_observation"] = "absent".into();
        assert!(spool.ingest(&serde_json::to_vec(&value).unwrap()).is_err());
        drop(spool);
        let _ = std::fs::remove_file(path);
    }
}
