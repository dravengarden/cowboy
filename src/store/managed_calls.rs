//! Parent-scoped immutable admission and compare-and-set lifecycle persistence.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::managed_calls::Request;
use crate::managed_calls::lifecycle::{Record, State};
use anyhow::{Result, ensure};
use sha2::{Digest as _, Sha256};

#[derive(sqlx::FromRow)]
struct Row {
    call_id: String,
    parent_session_id: String,
    request_id: String,
    request_document: String,
    child_session_id: String,
    state: String,
    revision: i64,
    document: String,
    document_sha256: String,
}

fn checksum(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

impl Row {
    fn decode(self) -> Result<(Record, Request)> {
        ensure!(
            self.document.len() <= 2 * 1024 * 1024
                && self.document_sha256 == checksum(&self.document),
            "invalid managed call checksum or size"
        );
        let record: Record = serde_json::from_str(&self.document)?;
        let request = Request::parse(self.request_document.as_bytes())?;
        ensure!(
            record.validate()
                && record.call_id == self.call_id
                && record.placement.parent_session_id == self.parent_session_id
                && record.request_id == self.request_id
                && record.child_session_id == self.child_session_id
                && record.state.code() == self.state
                && matches_request(&record, &request)
                && i64::try_from(record.revision)? == self.revision,
            "invalid managed call record identity"
        );
        Ok((record, request))
    }
}

fn matches_request(record: &Record, request: &Request) -> bool {
    request.validate().is_ok()
        && record.request_id == request.request_id
        && record.request_digest == request.digest()
        && record.purpose == request.purpose
        && record.access == request.access
        && record.labels == request.labels
        && match &request.conversation {
            crate::managed_calls::Conversation::Fresh {} => true,
            crate::managed_calls::Conversation::Continue { child_session_id } => {
                child_session_id == &record.child_session_id
            }
        }
}

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

impl Store {
    pub(crate) async fn managed_child(&self, parent: &str, child: &str) -> Result<Option<Record>> {
        dispatch!(self, managed_child(parent, child))
    }
    pub async fn managed_call(&self, parent: &str, id: &str) -> Result<Option<Record>> {
        dispatch!(self, managed_call(parent, id))
    }

    pub async fn managed_call_request(
        &self,
        parent: &str,
        request: &str,
    ) -> Result<Option<Record>> {
        dispatch!(self, managed_call_request(parent, request))
    }

    pub async fn managed_calls(&self, parent: &str, before: Option<&str>) -> Result<Vec<Record>> {
        dispatch!(self, managed_calls(parent, before))
    }

    /// Every non-terminal call, oldest first, for Controller recovery.
    pub async fn active_managed_calls(&self) -> Result<Vec<Record>> {
        dispatch!(self, active_managed_calls())
    }

    /// Returns the original record after duplicate admission. A caller must
    /// claim Queued -> Starting using CAS; inserting/observing grants no launch.
    pub async fn admit_managed_call(&self, record: &Record, request: &Request) -> Result<Record> {
        ensure!(
            record.validate()
                && record.state == State::Queued
                && record.revision == 1
                && matches_request(record, request),
            "invalid managed call admission"
        );
        dispatch!(self, admit_managed_call(record, request))
    }

    /// Private dispatch input. Product projections deliberately return only Record.
    pub async fn managed_call_input(&self, parent: &str, id: &str) -> Result<Option<Request>> {
        dispatch!(self, managed_call_input(parent, id))
    }

    pub async fn advance_managed_call(&self, previous: &Record, next: &Record) -> Result<bool> {
        ensure!(
            previous.accepts(next) || previous.accepts_cancel_request(next),
            "invalid managed call transition"
        );
        dispatch!(self, advance_managed_call(previous, next))
    }
}

macro_rules! implementation {
    ($backend:ty) => {
        impl $backend {
            async fn managed_child(&self, parent: &str, child: &str) -> Result<Option<Record>> {
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE parent_session_id = $1 AND child_session_id = $2 ORDER BY admission_order DESC LIMIT 1")
                    .bind(parent).bind(child).fetch_optional(&self.pool).await?
                    .map(|row| row.decode().map(|(record, _)| record)).transpose()
            }
            async fn managed_call(&self, parent: &str, id: &str) -> Result<Option<Record>> {
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE parent_session_id = $1 AND call_id = $2")
                    .bind(parent).bind(id).fetch_optional(&self.pool).await?
                    .map(|row| row.decode().map(|(record, _)| record)).transpose()
            }

            async fn managed_call_input(&self, parent: &str, id: &str) -> Result<Option<Request>> {
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE parent_session_id = $1 AND call_id = $2")
                    .bind(parent).bind(id).fetch_optional(&self.pool).await?
                    .map(|row| row.decode().map(|(_, request)| request)).transpose()
            }

            async fn managed_call_request(&self, parent: &str, request: &str) -> Result<Option<Record>> {
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE parent_session_id = $1 AND request_id = $2")
                    .bind(parent).bind(request).fetch_optional(&self.pool).await?
                    .map(|row| row.decode().map(|(record, _)| record)).transpose()
            }

            async fn managed_calls(&self, parent: &str, before: Option<&str>) -> Result<Vec<Record>> {
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE parent_session_id = $1 AND ($2 IS NULL OR admission_order < (SELECT admission_order FROM managed_agent_calls WHERE parent_session_id = $1 AND call_id = $2)) ORDER BY admission_order DESC LIMIT 100")
                    .bind(parent).bind(before).fetch_all(&self.pool).await?
                    .into_iter().map(|row| row.decode().map(|(record, _)| record)).collect()
            }

            async fn active_managed_calls(&self) -> Result<Vec<Record>> {
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE state IN ('queued', 'starting', 'running', 'waiting_input', 'stopping') ORDER BY admission_order LIMIT 1000")
                    .fetch_all(&self.pool).await?
                    .into_iter().map(|row| row.decode().map(|(record, _)| record)).collect()
            }

            async fn admit_managed_call(&self, record: &Record, request: &Request) -> Result<Record> {
                let document = serde_json::to_string(record)?;
                let request_document = serde_json::to_string(request)?;
                Request::parse(request_document.as_bytes())?;
                // A duplicate continuation also hits the active-child index.
                // Observe by the original parent/request after any uniqueness
                // conflict; another request owning that child yields no match.
                sqlx::query("INSERT INTO managed_agent_calls (call_id, parent_session_id, request_id, revision, document, document_sha256, request_document, child_session_id, state) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT DO NOTHING")
                    .bind(&record.call_id).bind(&record.placement.parent_session_id).bind(&record.request_id)
                    .bind(i64::try_from(record.revision)?).bind(&document).bind(checksum(&document))
                    .bind(&request_document).bind(&record.child_session_id).bind(record.state.code())
                    .execute(&self.pool).await?;
                let existing = self.managed_call_request(&record.placement.parent_session_id, &record.request_id)
                    .await?.ok_or_else(|| anyhow::anyhow!("managed call admission missing"))?;
                ensure!(existing.request_digest == record.request_digest && existing.provider == record.provider
                    && existing.placement == record.placement, "managed call request conflict");
                Ok(existing)
            }

            async fn advance_managed_call(&self, previous: &Record, next: &Record) -> Result<bool> {
                let old_document = serde_json::to_string(previous)?;
                let document = serde_json::to_string(next)?;
                let result = sqlx::query("UPDATE managed_agent_calls SET revision = $1, document = $2, document_sha256 = $3, state = $8 WHERE call_id = $4 AND parent_session_id = $5 AND revision = $6 AND document_sha256 = $7")
                    .bind(i64::try_from(next.revision)?).bind(&document).bind(checksum(&document))
                    .bind(&previous.call_id).bind(&previous.placement.parent_session_id)
                    .bind(i64::try_from(previous.revision)?).bind(checksum(&old_document))
                    .bind(next.state.code())
                    .execute(&self.pool).await?;
                Ok(result.rows_affected() == 1)
            }
        }
    };
}

implementation!(PostgresStorage);
implementation!(SqliteStorage);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_calls::lifecycle::Placement;

    #[tokio::test]
    async fn duplicate_admission_and_cancellation_race_have_one_owner() {
        conformance("sqlite::memory:").await;
    }

    #[tokio::test]
    #[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
    async fn postgres_managed_calls_have_the_same_admission_and_retention_contract() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
        conformance(&url).await;
    }

    async fn conformance(url: &str) {
        let root = std::env::temp_dir().join(format!(
            "cowboy-call-store-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let store = Store::connect(url, root.join("artifacts")).await.unwrap();
        store.migrate().await.unwrap();
        for id in ["parent-1", "parent-2"] {
            let parent = serde_json::from_value(serde_json::json!({"id":id,"provider":"codex","machine_id":"hawk","cwd":"/workspace","title":"Parent","origin":"api","status":"running"})).unwrap();
            store.insert_session(&parent).await.unwrap();
        }
        let request = Request::parse(br#"{"schema":1,"request_id":"review-1","purpose":"review","instruction":"private review instructions","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#).unwrap();
        let first = Record {
            schema: 1,
            call_id: "call-1".into(),
            request_id: "review-1".into(),
            request_digest: request.digest(),
            provider: "codex".into(),
            purpose: request.purpose,
            access: request.access,
            labels: request.labels.clone(),
            placement: Placement {
                service_id: "svc-1".into(),
                parent_session_id: "parent-1".into(),
                parent_revision: "r1".into(),
                machine_id: "hawk".into(),
                workspace_id: "project".into(),
                cwd: "/workspace".into(),
            },
            child_session_id: "child-1".into(),
            state: State::Queued,
            revision: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
            input_revision: None,
            result: None,
            runtime_machine_id: None,
            provider_version: None,
            provider_generation_digest: None,
            child_cursor: None,
            cancel_requested_at_ms: None,
            error: None,
        };
        let (overlapping, overlapping_record) = assert_admission(&store, &first, &request).await;
        assert_lifecycle(&store, &first, &overlapping, &overlapping_record).await;
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    async fn assert_admission(
        store: &Store,
        first: &Record,
        request: &Request,
    ) -> (Request, Record) {
        store.admit_managed_call(first, request).await.unwrap();
        assert_eq!(
            store
                .managed_call_input("parent-1", "call-1")
                .await
                .unwrap()
                .unwrap()
                .instruction,
            request.instruction
        );
        assert!(
            store
                .managed_call_input("parent-2", "call-1")
                .await
                .unwrap()
                .is_none()
        );
        let mut overlapping = request.clone();
        overlapping.request_id = "review-2".into();
        overlapping.conversation = crate::managed_calls::Conversation::Continue {
            child_session_id: "child-1".into(),
        };
        let mut overlapping_record = first.clone();
        overlapping_record.call_id = "call-0".into();
        overlapping_record.request_id = overlapping.request_id.clone();
        overlapping_record.request_digest = overlapping.digest();
        assert!(
            store
                .admit_managed_call(&overlapping_record, &overlapping)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .managed_call_request("parent-1", "review-1")
                .await
                .unwrap()
                .unwrap()
                .call_id,
            first.call_id
        );
        assert!(
            store
                .managed_call_request("parent-2", "review-1")
                .await
                .unwrap()
                .is_none()
        );
        let mut duplicate = first.clone();
        duplicate.call_id = "call-2".into();
        duplicate.child_session_id = "child-2".into();
        assert_eq!(
            store
                .admit_managed_call(&duplicate, request)
                .await
                .unwrap()
                .call_id,
            "call-1"
        );
        duplicate.provider = "claude-code".into();
        assert!(store.admit_managed_call(&duplicate, request).await.is_err());
        assert!(
            store
                .managed_call("parent-2", "call-1")
                .await
                .unwrap()
                .is_none()
        );
        (overlapping, overlapping_record)
    }

    async fn assert_lifecycle(
        store: &Store,
        first: &Record,
        overlapping: &Request,
        overlapping_record: &Record,
    ) {
        let mut starting = first.clone();
        starting.state = State::Starting;
        starting.revision = 2;
        let mut cancelled = first.clone();
        cancelled.state = State::Cancelled;
        cancelled.revision = 2;
        let (a, b) = tokio::join!(
            store.advance_managed_call(first, &starting),
            store.advance_managed_call(first, &cancelled)
        );
        assert_ne!(a.unwrap(), b.unwrap());
        assert_eq!(
            store.managed_calls("parent-1", None).await.unwrap().len(),
            1
        );
        assert!(
            store
                .managed_calls("parent-1", Some("call-1"))
                .await
                .unwrap()
                .is_empty()
        );
        let current = store
            .managed_call("parent-1", "call-1")
            .await
            .unwrap()
            .unwrap();
        if !current.state.terminal() {
            let mut failed = current.clone();
            failed.state = State::Failed;
            failed.revision += 1;
            assert!(store.advance_managed_call(&current, &failed).await.unwrap());
        }
        // Once the earlier turn is terminal, the exact conversation can accept
        // another turn. Its active lease prevents retention GC of the parent.
        store
            .admit_managed_call(overlapping_record, overlapping)
            .await
            .unwrap();
        assert_eq!(
            store
                .managed_child("parent-1", "child-1")
                .await
                .unwrap()
                .unwrap()
                .call_id,
            "call-0"
        );
        assert_eq!(
            store.managed_calls("parent-1", None).await.unwrap()[0].call_id,
            "call-0"
        );
        assert_eq!(
            store
                .managed_calls("parent-1", Some("call-0"))
                .await
                .unwrap()[0]
                .call_id,
            "call-1"
        );
        let mut repeated = overlapping_record.clone();
        repeated.call_id = "call-overlap-duplicate".into();
        assert_eq!(
            store
                .admit_managed_call(&repeated, overlapping)
                .await
                .unwrap()
                .call_id,
            overlapping_record.call_id
        );
        for parent in ["parent-1", "parent-2"] {
            store.delete_session(parent).await.unwrap();
        }
        assert_eq!(store.purge_deleted(0).await.unwrap(), 1);
        assert!(
            store
                .managed_call("parent-1", "call-1")
                .await
                .unwrap()
                .is_some()
        );
        let mut cancelled = overlapping_record.clone();
        cancelled.state = State::Cancelled;
        cancelled.revision += 1;
        assert!(
            store
                .advance_managed_call(overlapping_record, &cancelled)
                .await
                .unwrap()
        );
        assert_eq!(store.purge_deleted(0).await.unwrap(), 1);
        assert!(
            store
                .managed_call("parent-1", "call-1")
                .await
                .unwrap()
                .is_none()
        );
    }
}
