use super::*;

impl SqliteSessionRow {
    pub(super) fn into_meta(
        self,
        account_id: crate::provider_account::ProviderAccountId,
    ) -> SessionMeta {
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
            context_used: 0,
            context_size: 0,
            usage: None,
            attention: crate::model::SessionAttention::default(),
            background_tasks: 0,
            supports_current_turn: false,
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
