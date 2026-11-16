//! The opening prompt of a session's current native context, read from the
//! durable transcript when the Hub's held log tail no longer reaches it.
//!
//! A context renewal digest quotes the request that started the context it
//! replaces (see [`crate::context_rollover`]). Long sessions trim their held
//! tail, so the Hub alone loses exactly that request.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::agent_model::Event;
use crate::model::Envelope;
use anyhow::{Context as _, Result};

/// User prompt rows read after the context boundary: enough to step over
/// generated reminders and machine-sent prompts before the first human one.
const OPENING_ROWS: usize = 64;

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

impl Store {
    /// The first user prompt rows, oldest first, of the native context that
    /// is current at `before_seq`: after the newest Clear or renewal boundary
    /// before it, or from the session's start.
    pub async fn context_opening_prompts(
        &self,
        session_id: &str,
        before_seq: u64,
    ) -> Result<Vec<Envelope>> {
        let before = i64::try_from(before_seq).context("context start cursor overflow")?;
        let rows = dispatch!(self, context_opening_prompt_rows(session_id, before))?;
        Ok(rows
            .into_iter()
            .filter_map(|(seq, mut payload)| {
                let cmid = crate::persistence::take_persisted_cmid(&mut payload);
                let event = serde_json::from_value::<Event>(payload).ok()?;
                Some(Envelope {
                    session_id: session_id.to_owned(),
                    seq: u64::try_from(seq).ok()?,
                    event,
                    cmid,
                })
            })
            .collect())
    }
}

macro_rules! implementation {
    ($backend:ty, $update_kind:literal) => {
        impl $backend {
            async fn context_opening_prompt_rows(
                &self,
                session_id: &str,
                before: i64,
            ) -> Result<Vec<(i64, serde_json::Value)>> {
                let boundary: Option<i64> = sqlx::query_scalar(concat!(
                    "SELECT MAX(seq) FROM events WHERE session_id = $1 AND seq < $2 AND ",
                    $update_kind,
                    " IN ('context_cleared', 'context_rolled_over')"
                ))
                .bind(session_id)
                .bind(before)
                .fetch_one(&self.pool)
                .await
                .context("SELECT newest context boundary")?;
                sqlx::query_as(concat!(
                    "SELECT seq, payload FROM events WHERE session_id = $1 AND seq > $2 \
                     AND seq < $3 AND ",
                    $update_kind,
                    " = 'user_message_chunk' ORDER BY seq LIMIT $4"
                ))
                .bind(session_id)
                .bind(boundary.unwrap_or(-1))
                .bind(before)
                .bind(i64::try_from(OPENING_ROWS).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await
                .context("SELECT context opening prompts")
            }
        }
    };
}

implementation!(PostgresStorage, "payload->'update'->>'sessionUpdate'");
implementation!(SqliteStorage, "json_extract(payload, '$.update.sessionUpdate')");
