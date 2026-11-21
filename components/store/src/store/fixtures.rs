//! Fixtures shared by the store's tests and the base crate's Hub tests.

use sqlx::postgres::PgPool;

use super::{StorageBackend, Store};

use crate::agent_model::Status;
use crate::model::{SessionMeta, SessionOrigin};

/// A Web session on Machine `hawk` in workspace `cowboy`.
#[must_use]
pub fn session(id: &str) -> SessionMeta {
    SessionMeta {
        continued_from: None,
        handoff_resumed: false,
        forked_from: None,
        id: id.to_owned(),
        provider: "codex".to_owned(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".to_owned(),
        workspace_id: Some("cowboy".to_owned()),
        workspace_name: Some("Cowboy".to_owned()),
        workspace_source_path: Some("/tmp/cowboy".to_owned()),
        execution_binding: None,
        cwd: "/tmp/cowboy-worktree".to_owned(),
        title: "Storage contract".to_owned(),
        status: Status::Starting,
        origin: SessionOrigin::Web,
        agent_session_id: None,
        paused: false,
        closing: false,
        system: false,
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
        owner_user_id: None,
        owner_username: None,
        trashed_at_ms: None,
    }
}

/// The connection pool behind a store, for crates whose tests seed or inspect
/// rows directly (the base crate's Hub contracts).
pub enum TestPool<'a> {
    Postgres(&'a PgPool),
    Sqlite(&'a sqlx::SqlitePool),
}

impl Store {
    #[must_use]
    pub fn test_pool(&self) -> TestPool<'_> {
        match &self.backend {
            StorageBackend::Postgres(db) => TestPool::Postgres(&db.pool),
            StorageBackend::Sqlite(db) => TestPool::Sqlite(&db.pool),
        }
    }
}
