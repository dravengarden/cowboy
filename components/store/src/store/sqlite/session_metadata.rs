use super::*;

impl SqliteSessionRow {
    pub(super) fn into_meta(
        self,
        account_id: crate::provider_account::ProviderAccountId,
    ) -> SessionMeta {
        let last_usage =
            crate::store::session_metadata::decode_last_usage(self.last_usage, &self.id);
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
            system: self.system,
            paused: false,
            closing: false,
            // Restored last-reported usage (see the PostgreSQL row).
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
            next_schedule_ms: None,
            owner_user_id: self.owner_user_id,
            owner_username: self.owner_username,
            trashed_at_ms: self.trashed_at_ms,
        }
    }
}

impl SqliteStorage {
    pub(in crate::store) async fn update_session_usage(
        &self,
        session_id: &str,
        usage: Option<&serde_json::Value>,
    ) -> Result<()> {
        let mut usage = usage.cloned();
        if let Some(value) = usage.as_mut() {
            strip_nul(value);
        }
        sqlx::query("UPDATE sessions SET last_usage = ?1 WHERE id = ?2")
            .bind(usage)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session last_usage {session_id}"))?;
        Ok(())
    }

    pub(in crate::store) async fn update_config_options(
        &self,
        session_id: &str,
        options: &serde_json::Value,
    ) -> Result<()> {
        let mut options = options.clone();
        strip_nul(&mut options);
        sqlx::query("UPDATE sessions SET config_options = ?1, updated_at_ms = ?2 WHERE id = ?3")
            .bind(options)
            .bind(now_ms())
            .bind(session_id)
            .execute(&self.pool)
            .await
            .with_context(|| format!("UPDATE SQLite session config_options {session_id}"))?;
        Ok(())
    }
}

impl SqliteStorage {
    pub(in crate::store) async fn clear_conversation(
        &self,
        session_id: &str,
        calls_before_ms: i64,
        fence: Option<crate::store::session_metadata::ClearFence<'_>>,
    ) -> Result<()> {
        self.clear_conversation_inner(session_id, Some(calls_before_ms), fence, None)
            .await
    }

    /// See `PostgresStorage::clear_conversation_inner`.
    pub(in crate::store) async fn clear_conversation_inner(
        &self,
        session_id: &str,
        transcript: Option<i64>,
        fence: Option<crate::store::session_metadata::ClearFence<'_>>,
        turn_lost: Option<crate::store::session_metadata::ClearFence<'_>>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let now = now_ms();
        if let Some(calls_before_ms) = transcript {
            sqlx::query("DELETE FROM events WHERE session_id = ?1")
                .bind(session_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("DELETE SQLite events for {session_id}"))?;
            crate::store::session_widgets::clear_transcript_projections_sqlite(&mut tx, session_id)
                .await?;
            crate::store::managed_calls::clear_calls_on_transcript_clear_sqlite(
                &mut tx,
                session_id,
                calls_before_ms,
            )
            .await?;
        }
        if fence.is_some() {
            sqlx::query(
                "UPDATE sessions SET agent_session_id = NULL, updated_at_ms = ?1 WHERE id = ?2",
            )
            .bind(now)
            .bind(session_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPDATE SQLite session agent_session_id {session_id}"))?;
        }
        for (key, value) in fence.into_iter().chain(turn_lost) {
            sqlx::query(
                "INSERT INTO settings(key, value, updated_at_ms) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value, \
                 updated_at_ms = excluded.updated_at_ms",
            )
            .bind(key)
            .bind(value)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPSERT SQLite Hub setting {key}"))?;
        }
        tx.commit().await?;
        Ok(())
    }
}
