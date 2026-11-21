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

impl PostgresStorage {
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
