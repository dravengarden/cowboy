use super::*;

impl SessionRow {
    pub(super) fn into_meta(
        self,
        account_id: crate::provider_account::ProviderAccountId,
    ) -> SessionMeta {
        let last_usage = decode_last_usage(self.last_usage, &self.id);
        SessionMeta {
            continued_from: None,
            handoff_resumed: false,
            forked_from: None,
            id: self.id,
            provider: self.provider,
            provider_version: self.provider_version,
            provider_generation_digest: self.provider_generation_digest,
            provider_auth_generation: self
                .provider_auth_generation
                .and_then(|value| u64::try_from(value).ok()),
            provider_account_id: account_id,
            provider_behavior: self
                .provider_behavior
                .and_then(|value| serde_json::from_value(value).ok()),
            machine_id: self.machine_id,
            workspace_id: self.workspace_id,
            workspace_name: self.workspace_name,
            workspace_source_path: self.workspace_source_path,
            execution_binding: self
                .execution_binding
                .map(crate::execution_environment::ExecutionBinding::from_record),
            cwd: self.cwd,
            title: self.title,
            status: status_from_str(&self.status),
            origin: origin_from_str(&self.origin),
            agent_session_id: self.agent_session_id,
            // Restored from the durable baseline — a system session stays
            // view-only across a daemon restart.
            system: self.system,
            // Transient — the manual pause is in-memory only (not persisted), so a
            // restored session always comes back un-paused.
            paused: false,
            closing: false,
            // The last usage the agent reported survives a Controller restart:
            // a dormant session has no worker to re-report it, so the composer
            // keeps the recorded reading until a live worker reports again.
            context_used: last_usage.as_ref().map_or(0, |usage| usage.used),
            context_size: last_usage.as_ref().map_or(0, |usage| usage.size),
            usage: last_usage,
            attention: crate::model::SessionAttention::default(),
            background_tasks: 0,
            supports_current_turn: false,
            wrap_up_steering: false,
            provider_update: None,
            provider_update_available: None,
            resource_wait: None,
            // Derived from restored drafts in `session_list`, not stored here.
            next_schedule_ms: None,
            owner_user_id: self.owner_user_id,
            owner_username: self.owner_username,
            trashed_at_ms: self.trashed_at_ms,
        }
    }
}

/// Decode a session's persisted `last_usage`, degrading a malformed or legacy
/// value to "not reported" so one bad row never blocks the restore.
pub fn decode_last_usage(
    value: Option<serde_json::Value>,
    session_id: &str,
) -> Option<crate::agent_model::SessionUsage> {
    let value = value.filter(|value| !value.is_null())?;
    match serde_json::from_value(value) {
        Ok(usage) => Some(usage),
        Err(error) => {
            tracing::warn!(%error, session = %session_id, "ignoring undecodable last_usage");
            None
        }
    }
}

impl Store {
    /// Persist the session's last agent-reported context usage (`None` clears
    /// it). A display snapshot only: it is not activity and never touches the
    /// session's `updated_at`.
    pub async fn update_session_usage(
        &self,
        session_id: &str,
        usage: Option<&crate::agent_model::SessionUsage>,
    ) -> Result<()> {
        let value = usage.map(serde_json::to_value).transpose()?;
        match &self.backend {
            StorageBackend::Postgres(backend) => {
                backend
                    .update_session_usage(session_id, value.as_ref())
                    .await
            }
            StorageBackend::Sqlite(backend) => {
                backend
                    .update_session_usage(session_id, value.as_ref())
                    .await
            }
        }
    }
}

/// Clear conversation's restart fence: the discarded native ids under their
/// registered setting key. Written with the native id's removal.
pub type ClearFence<'a> = (&'a str, &'a serde_json::Value);

/// Advance the durable context epoch for a session's uncertain-effect ack
/// record inside Clear's atomic transaction so concurrent stale-context CAS
/// writes fail atomically and the fresh conversation starts without old holds.
pub(in crate::store) fn advance_uncertain_effect_context_record(
    existing: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut record = match existing {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    let next_epoch = record
        .get("context_epoch")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    record.insert("context_epoch".into(), serde_json::json!(next_epoch));
    record.remove("observed");
    record.remove("method");
    record.remove("observed_at_ms");
    serde_json::Value::Object(record)
}

fn checked_fence(fence: Option<ClearFence<'_>>) -> Result<Option<(&str, serde_json::Value)>> {
    fence
        .map(|(key, value)| {
            anyhow::ensure!(
                crate::model::settings_keys::is_persisted_setting_key(key),
                "unsupported Hub setting"
            );
            let mut value = value.clone();
            strip_nul(&mut value);
            Ok((key, value))
        })
        .transpose()
}

impl Store {
    /// Forget the session's native id and record its fence in one transaction,
    /// so neither persists without the other (context renewal, fork reset).
    pub async fn clear_agent_session_id(
        &self,
        session_id: &str,
        fence_key: &str,
        fence: &serde_json::Value,
    ) -> Result<()> {
        let fence = checked_fence(Some((fence_key, fence)))?;
        let fence = fence.as_ref().map(|(key, value)| (*key, value));
        match &self.backend {
            StorageBackend::Postgres(backend) => {
                backend
                    .clear_conversation_inner(session_id, None, fence, None)
                    .await
            }
            StorageBackend::Sqlite(backend) => {
                backend
                    .clear_conversation_inner(session_id, None, fence, None)
                    .await
            }
        }
    }

    /// Clear conversation as one transaction: delete the transcript and its
    /// projections, archive old Calls, with `fence` forget the native id and
    /// record the fence, and with `turn_lost` record that the call budget's
    /// owner turn went with the transcript. A failure keeps the transcript and
    /// the context.
    pub async fn clear_conversation(
        &self,
        session_id: &str,
        calls_before_ms: i64,
        fence: Option<ClearFence<'_>>,
        turn_lost: Option<ClearFence<'_>>,
    ) -> Result<()> {
        let fence = checked_fence(fence)?;
        let fence = fence.as_ref().map(|(key, value)| (*key, value));
        let turn_lost = checked_fence(turn_lost)?;
        let turn_lost = turn_lost.as_ref().map(|(key, value)| (*key, value));
        match &self.backend {
            StorageBackend::Postgres(backend) => {
                backend
                    .clear_conversation_inner(session_id, Some(calls_before_ms), fence, turn_lost)
                    .await
            }
            StorageBackend::Sqlite(backend) => {
                backend
                    .clear_conversation_inner(session_id, Some(calls_before_ms), fence, turn_lost)
                    .await
            }
        }
    }
}

impl PostgresStorage {
    pub(super) async fn clear_conversation(
        &self,
        session_id: &str,
        calls_before_ms: i64,
        fence: Option<ClearFence<'_>>,
    ) -> Result<()> {
        self.clear_conversation_inner(session_id, Some(calls_before_ms), fence, None)
            .await
    }

    /// `transcript`: delete it, archiving Calls created before this time.
    async fn clear_conversation_inner(
        &self,
        session_id: &str,
        transcript: Option<i64>,
        fence: Option<ClearFence<'_>>,
        turn_lost: Option<ClearFence<'_>>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        if let Some(calls_before_ms) = transcript {
            session_lineage::seal_lineage_on_clear_pg(&mut tx, session_id).await?;
            turn_cache_facts::project_missing_for_session_pg(&mut tx, session_id).await?;
            sqlx::query("DELETE FROM events WHERE session_id = $1")
                .bind(session_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("DELETE events for {session_id}"))?;
            session_widgets::clear_transcript_projections_pg(&mut tx, session_id).await?;
            // Archive old Calls while keeping unconfirmed opted-in notices visible.
            managed_calls::clear_calls_on_transcript_clear_pg(&mut tx, session_id, calls_before_ms)
                .await?;
        }
        for (key, value) in fence.into_iter().chain(turn_lost) {
            sqlx::query(
                "INSERT INTO settings(key, value, updated_at) VALUES ($1, $2, now()) \
                 ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
            )
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPSERT Hub setting {key}"))?;
        }
        if fence.is_some() {
            sqlx::query(
                "UPDATE sessions SET agent_session_id = NULL, updated_at = now() WHERE id = $1",
            )
            .bind(session_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPDATE session agent_session_id {session_id}"))?;
            let ack_key = format!(
                "{}{session_id}",
                crate::model::settings_keys::SESSION_UNCERTAIN_EFFECT_ACK_PREFIX
            );
            let existing: Option<(serde_json::Value,)> =
                sqlx::query_as("SELECT value FROM settings WHERE key = $1 FOR UPDATE")
                    .bind(&ack_key)
                    .fetch_optional(&mut *tx)
                    .await
                    .with_context(|| format!("SELECT Hub setting {ack_key}"))?;
            let bumped = advance_uncertain_effect_context_record(existing.map(|(v,)| v));
            sqlx::query(
                "INSERT INTO settings(key, value, updated_at) VALUES ($1, $2, now()) \
                 ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
            )
            .bind(&ack_key)
            .bind(&bumped)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPSERT Hub setting {ack_key}"))?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_session_usage(
        &self,
        session_id: &str,
        usage: Option<&serde_json::Value>,
    ) -> Result<()> {
        let mut usage = usage.cloned();
        if let Some(value) = usage.as_mut() {
            strip_nul(value);
        }
        sqlx::query("UPDATE sessions SET last_usage = $1 WHERE id = $2")
            .bind(usage)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session last_usage {session_id}"))?;
        Ok(())
    }

    /// Persist the latest agent-advertised config options. This is a display
    /// snapshot, not the source of the user's selected values.
    pub async fn update_config_options(
        &self,
        session_id: &str,
        options: &serde_json::Value,
    ) -> Result<()> {
        let mut options = options.clone();
        strip_nul(&mut options);
        sqlx::query("UPDATE sessions SET config_options = $1, updated_at = now() WHERE id = $2")
            .bind(options)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE session config_options {session_id}"))?;
        Ok(())
    }
}
