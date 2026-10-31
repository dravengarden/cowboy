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
    /// Atomically choose one owner for equivalent captured review input. A new
    /// explicit attempt after an unsuccessful completion can claim the input;
    /// callers already waiting on that attempt must observe its actual failure.
    pub async fn reserve_review_input(&self, record: &Record, identity: &str) -> Result<Record> {
        dispatch!(self, reserve_review_input(record, identity))
    }

    /// Recover an immutable attempt selection without re-reading its inputs.
    pub async fn review_input_source(&self, parent: &str, call: &str) -> Result<Option<Record>> {
        dispatch!(self, review_input_source(parent, call))
    }

    /// Hide/restore settled rows without deleting idempotency receipts, child
    /// ownership or budget accounting. The terminal predicate wins cancel races.
    pub async fn dismiss_managed_call(
        &self,
        parent: &str,
        id: &str,
        dismissed: bool,
    ) -> Result<bool> {
        dispatch!(self, dismiss_managed_call(parent, id, dismissed))
    }
    pub async fn managed_child(&self, parent: &str, child: &str) -> Result<Option<Record>> {
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

    /// A parent's active and total call counts, for its session budget.
    pub async fn managed_call_counts(&self, parent: &str) -> Result<(u64, u64)> {
        dispatch!(self, managed_call_counts(parent))
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

    /// Whether a person dismissed this call from the session's list.
    pub async fn managed_call_dismissed(&self, parent: &str, id: &str) -> Result<bool> {
        dispatch!(self, managed_call_dismissed(parent, id))
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
            async fn review_input_source(&self, parent: &str, call: &str) -> Result<Option<Record>> {
                let owner: Option<(String,)> = sqlx::query_as("SELECT source_call_id FROM managed_review_sources WHERE parent_session_id = $1 AND call_id = $2")
                    .bind(parent).bind(call).fetch_optional(&self.pool).await?;
                match owner {
                    Some((owner,)) => Ok(Some(self.managed_call(parent, &owner).await?
                        .ok_or_else(|| anyhow::anyhow!("review source missing"))?)),
                    None => Ok(None),
                }
            }

            async fn reserve_review_input(&self, record: &Record, identity: &str) -> Result<Record> {
                let parent = &record.placement.parent_session_id;
                if let Some(original) = self.review_input_source(parent, &record.call_id).await? {
                    return Ok(original);
                }
                let mut identity = identity.to_owned();
                // Retain failed owners too. Replacing their reservation would
                // silently redirect existing waiters to a later retry.
                for _ in 0..128 {
                    sqlx::query("INSERT INTO managed_review_inputs (parent_session_id, input_identity, call_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                        .bind(parent).bind(&identity).bind(&record.call_id).execute(&self.pool).await?;
                    let (owner,): (String,) = sqlx::query_as("SELECT call_id FROM managed_review_inputs WHERE parent_session_id = $1 AND input_identity = $2")
                        .bind(parent).bind(&identity).fetch_one(&self.pool).await?;
                    let original = self.managed_call(parent, &owner).await?
                        .ok_or_else(|| anyhow::anyhow!("review input owner missing"))?;
                    if owner == record.call_id
                        || !original.state.terminal()
                        || crate::managed_calls::review::complete(&original)
                        || original.finished_at_ms.is_none_or(|finished| record.created_at_ms <= finished)
                    {
                        sqlx::query("INSERT INTO managed_review_sources (parent_session_id, call_id, source_call_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
                            .bind(parent).bind(&record.call_id).bind(&owner).execute(&self.pool).await?;
                        return self.review_input_source(parent, &record.call_id).await?
                            .ok_or_else(|| anyhow::anyhow!("review source missing"));
                    }
                    identity = crate::managed_calls::canonical_digest(&serde_json::json!({
                        "input": identity, "after": owner
                    }));
                }
                anyhow::bail!("review retry history limit")
            }

            async fn managed_call_dismissed(&self, parent: &str, id: &str) -> Result<bool> {
                let dismissed: Option<bool> = sqlx::query_scalar("SELECT dismissed FROM managed_agent_calls WHERE parent_session_id = $1 AND call_id = $2")
                    .bind(parent).bind(id).fetch_optional(&self.pool).await?;
                Ok(dismissed.unwrap_or(false))
            }

            async fn dismiss_managed_call(&self, parent: &str, id: &str, dismissed: bool) -> Result<bool> {
                let result = sqlx::query("UPDATE managed_agent_calls SET dismissed = $3 WHERE parent_session_id = $1 AND call_id = $2 AND state IN ('completed', 'failed', 'cancelled')")
                    .bind(parent).bind(id).bind(dismissed).execute(&self.pool).await?;
                Ok(result.rows_affected() == 1)
            }
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
                sqlx::query_as::<_, Row>("SELECT call_id, parent_session_id, request_id, request_document, child_session_id, state, revision, document, document_sha256 FROM managed_agent_calls WHERE parent_session_id = $1 AND dismissed = FALSE AND ($2 IS NULL OR admission_order < (SELECT admission_order FROM managed_agent_calls WHERE parent_session_id = $1 AND call_id = $2)) ORDER BY admission_order DESC LIMIT 100")
                    .bind(parent).bind(before).fetch_all(&self.pool).await?
                    .into_iter().map(|row| row.decode().map(|(record, _)| record)).collect()
            }

            async fn managed_call_counts(&self, parent: &str) -> Result<(u64, u64)> {
                let (active, total): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(CASE WHEN state IN ('queued', 'starting', 'running', 'waiting_input', 'stopping') THEN 1 ELSE 0 END), 0), COUNT(*) FROM managed_agent_calls WHERE parent_session_id = $1")
                    .bind(parent).fetch_one(&self.pool).await?;
                Ok((u64::try_from(active)?, u64::try_from(total)?))
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
            started_at_ms: None,
            finished_at_ms: None,
            configuration: None,
            provider_account_id: None,
            input_revision: None,
            result: None,
            runtime_machine_id: None,
            provider_version: None,
            provider_generation_digest: None,
            child_cursor: None,
            cancel_requested_at_ms: None,
            error: None,
            preset: None,
            selection: None,
        };
        let (overlapping, overlapping_record) = assert_admission(&store, &first, &request).await;
        assert_rejected_images_do_not_poison_event_batches(&store).await;
        assert_image_retention(&store, &first, &request).await;
        assert_lifecycle(&store, &first, &overlapping, &overlapping_record).await;
        assert_review_reservations(&store, &first, &request).await;
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[allow(clippy::too_many_lines)] // One durable timeline covers concurrent owners, failure and retry.
    async fn assert_review_reservations(store: &Store, template: &Record, input: &Request) {
        let parent = serde_json::from_value(serde_json::json!({
            "id":"review-reservations","provider":"codex","machine_id":"hawk","cwd":"/workspace",
            "title":"Review","origin":"api","status":"running"
        }))
        .unwrap();
        store.insert_session(&parent).await.unwrap();
        let mut records = Vec::new();
        for index in 0..6 {
            let mut request = input.clone();
            request.request_id = format!("reservation-{index}");
            let mut record = template.clone();
            record.placement.parent_session_id = "review-reservations".into();
            record.call_id = format!("reservation-{index}");
            record.child_session_id = format!("reservation-child-{index}");
            record.request_id.clone_from(&request.request_id);
            record.request_digest = request.digest();
            record.created_at_ms = if index == 5 { 225 } else { 100 + index * 100 };
            record.updated_at_ms = record.created_at_ms;
            store.admit_managed_call(&record, &request).await.unwrap();
            let mut starting = record.clone();
            starting.state = State::Starting;
            starting.revision += 1;
            assert!(
                store
                    .advance_managed_call(&record, &starting)
                    .await
                    .unwrap()
            );
            records.push(starting);
        }
        let (left, right) = tokio::join!(
            store.reserve_review_input(&records[0], "same-input"),
            store.reserve_review_input(&records[1], "same-input"),
        );
        let original = left.unwrap();
        assert_eq!(original.call_id, right.unwrap().call_id);
        let waiter = if original.call_id == records[0].call_id {
            &records[1]
        } else {
            &records[0]
        };
        let mut failed = original.clone();
        failed.state = State::Failed;
        failed.revision += 1;
        failed.updated_at_ms = 250;
        failed.finished_at_ms = Some(250);
        assert!(
            store
                .advance_managed_call(&original, &failed)
                .await
                .unwrap()
        );
        // A request queued before failure shares that attempt even when its
        // first lookup happens after failure. It must not implicitly retry.
        assert_eq!(
            store
                .reserve_review_input(&records[5], "same-input")
                .await
                .unwrap()
                .call_id,
            original.call_id
        );
        let recovered = store
            .review_input_source("review-reservations", &waiter.call_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.call_id, original.call_id);
        assert_eq!(recovered.state, State::Failed);
        assert!(
            store
                .review_input_source("another-parent", &waiter.call_id)
                .await
                .unwrap()
                .is_none()
        );
        let retry = store
            .reserve_review_input(&records[2], "same-input")
            .await
            .unwrap();
        assert_eq!(retry.call_id, records[2].call_id);
        // A retry must not steal the already-waiting request's original outcome.
        assert_eq!(
            store
                .reserve_review_input(waiter, "same-input")
                .await
                .unwrap()
                .call_id,
            original.call_id
        );
        let mut completed = retry.clone();
        completed.state = State::Completed;
        completed.revision += 1;
        completed.updated_at_ms = 350;
        completed.finished_at_ms = Some(350);
        completed.input_revision = Some(crate::managed_calls::canonical_digest(
            &serde_json::json!("snapshot"),
        ));
        completed.result = Some(serde_json::json!({
            "reused_from":{"call_id":"native-proof"},
            "structured":{"review_complete":true,"verdict":"needs-attention"}
        }));
        assert!(
            store
                .advance_managed_call(&retry, &completed)
                .await
                .unwrap()
        );
        assert_eq!(
            store
                .reserve_review_input(&records[3], "same-input")
                .await
                .unwrap()
                .call_id,
            retry.call_id
        );
        assert_eq!(
            store
                .reserve_review_input(&records[4], "changed-input")
                .await
                .unwrap()
                .call_id,
            records[4].call_id
        );
    }

    async fn assert_rejected_images_do_not_poison_event_batches(store: &Store) {
        use crate::core::{Envelope, Event};
        use crate::managed_calls::media;
        let image = |seq, data| Envelope {
            session_id: "parent-1".into(),
            seq,
            cmid: None,
            event: Event::Update {
                update: serde_json::json!({"sessionUpdate":"tool_call_update","status":"completed",
                    "rawOutput":{"cowboyGeneratedImage":true},"content":[{"type":"content","content":{
                        "type":"image","mimeType":"image/png","data":data}}]}),
            },
        };
        let healthy = Envelope {
            session_id: "parent-2".into(),
            seq: 0,
            cmid: None,
            event: Event::TurnEnd {
                usage: None,
                stop_reason: "end_turn".into(),
            },
        };
        let png = media::tests::png();
        let expected = media::inspect(&png).unwrap();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        store
            .upsert_event_batch(
                &[
                    image(0, "A".repeat(media::MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1)),
                    healthy.clone(),
                    image(1, "invalid base64!".into()),
                    image(2, encoded),
                ],
                &std::collections::HashMap::from([("parent-1".into(), 3), ("parent-2".into(), 1)]),
            )
            .await
            .unwrap();
        let sessions = store.load_all().await.unwrap();
        let other = sessions.iter().find(|s| s.meta.id == "parent-2").unwrap();
        assert_eq!(other.next_seq, 1);
        assert_eq!(
            serde_json::to_value(&other.events).unwrap(),
            serde_json::json!([healthy])
        );
        let rejected = sessions.iter().find(|s| s.meta.id == "parent-1").unwrap();
        assert_eq!(rejected.next_seq, 3);
        assert_eq!(rejected.events.len(), 3);
        for (event, code) in rejected
            .events
            .iter()
            .zip(["artifact_too_large", "artifact_invalid"])
        {
            let Event::Update { update } = &event.event else {
                panic!("expected rejected image event");
            };
            assert!(serde_json::to_vec(update).unwrap().len() < 512);
            assert_eq!(
                media::collect(update, &store.artifacts()).unwrap_err(),
                code
            );
            let mut replay = update.clone();
            store.artifacts().externalize_images(&mut replay).unwrap();
            assert_eq!(&replay, update);
        }
        let Event::Update { update } = &rejected.events[2].event else {
            panic!("expected restored generated image");
        };
        assert!(update["content"][0]["content"].get("data").is_none());
        assert_eq!(
            media::collect(update, &store.artifacts()).unwrap(),
            [expected]
        );
    }

    async fn assert_image_retention(store: &Store, template: &Record, input: &Request) {
        let parent = serde_json::from_value(serde_json::json!({"id":"image-parent","provider":"claude-code","machine_id":"hawk","cwd":"/workspace","title":"Images","origin":"api","status":"running"})).unwrap();
        store.insert_session(&parent).await.unwrap();
        let mut request = input.clone();
        request.request_id = "image-retention".into();
        request.purpose = crate::managed_calls::Purpose::ImageGenerate;
        let mut record = template.clone();
        record.call_id = "image-call".into();
        record.child_session_id = "image-child".into();
        record.placement.parent_session_id = "image-parent".into();
        record.request_id = request.request_id.clone();
        record.request_digest = request.digest();
        record.purpose = request.purpose;
        store.admit_managed_call(&record, &request).await.unwrap();
        for state in [State::Starting, State::Running] {
            let mut next = record.clone();
            next.state = state;
            next.revision += 1;
            next.input_revision = Some(crate::managed_calls::canonical_digest(&serde_json::json!(
                "image-input"
            )));
            assert!(store.advance_managed_call(&record, &next).await.unwrap());
            record = next;
        }
        let url = store
            .artifacts()
            .put_bytes(b"retained-image-fixture", "image/png")
            .unwrap();
        let path = store
            .artifact_path(url.strip_prefix("/api/artifacts/").unwrap())
            .unwrap();
        std::fs::File::open(&path)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(48))
            .unwrap();
        let mut completed = record.clone();
        completed.state = State::Completed;
        completed.revision += 1;
        completed.result = Some(serde_json::json!({"artifacts":[{"url":url}]}));
        assert!(
            store
                .advance_managed_call(&record, &completed)
                .await
                .unwrap()
        );
        // No transcript event references these bytes: the retained receipt alone
        // keeps them alive, including after UI dismissal.
        store
            .dismiss_managed_call("image-parent", "image-call", true)
            .await
            .unwrap();
        assert_eq!(store.purge_deleted(0).await.unwrap(), 0);
        assert!(path.is_file());
        store.delete_session("image-parent").await.unwrap();
        assert_eq!(store.purge_deleted(0).await.unwrap(), 1);
        assert!(!path.exists());
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

    async fn assert_dismissal(store: &Store, first: &Record) {
        // UI cleanup cannot stop an active child, cross parent boundaries,
        // reset budget counts or erase the duplicate-admission receipt.
        let counts = store.managed_call_counts("parent-1").await.unwrap();
        assert!(
            !store
                .dismiss_managed_call("parent-1", "call-0", true)
                .await
                .unwrap()
        );
        assert!(
            !store
                .dismiss_managed_call("parent-2", "call-1", true)
                .await
                .unwrap()
        );
        assert!(
            store
                .dismiss_managed_call("parent-1", "call-1", true)
                .await
                .unwrap()
        );
        assert!(
            store
                .dismiss_managed_call("parent-1", "call-1", true)
                .await
                .unwrap()
        );
        assert_eq!(
            store.managed_calls("parent-1", None).await.unwrap().len(),
            1
        );
        assert_eq!(store.managed_call_counts("parent-1").await.unwrap(), counts);
        assert!(
            store
                .managed_call_request("parent-1", &first.request_id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .dismiss_managed_call("parent-1", "call-1", false)
                .await
                .unwrap()
        );
        assert_eq!(
            store.managed_calls("parent-1", None).await.unwrap().len(),
            2
        );
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
        assert_dismissal(store, first).await;
        repeated.call_id = "call-overlap-duplicate".into();
        assert_eq!(
            store
                .admit_managed_call(&repeated, overlapping)
                .await
                .unwrap()
                .call_id,
            overlapping_record.call_id
        );
        assert_clear_and_retention(store, first, overlapping_record).await;
    }

    async fn assert_clear_and_retention(
        store: &Store,
        first: &Record,
        overlapping_record: &Record,
    ) {
        let counts = store.managed_call_counts("parent-1").await.unwrap();
        let other_parent = store.managed_calls("parent-2", None).await.unwrap().len();
        store.clear_events("parent-1", 100).await.unwrap();
        assert!(
            store
                .managed_calls("parent-1", None)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.managed_call_counts("parent-1").await.unwrap(), counts);
        assert_eq!(
            store.managed_calls("parent-2", None).await.unwrap().len(),
            other_parent
        );
        assert!(
            store
                .managed_call_request("parent-1", &first.request_id)
                .await
                .unwrap()
                .is_some()
        );
        let fresh_request = Request::parse(br#"{"schema":1,"request_id":"after-clear","purpose":"review","instruction":"new conversation","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#).unwrap();
        let mut fresh = first.clone();
        fresh.call_id = "call-after-clear".into();
        fresh.child_session_id = "child-after-clear".into();
        fresh.request_id.clone_from(&fresh_request.request_id);
        fresh.request_digest = fresh_request.digest();
        fresh.created_at_ms = 100;
        fresh.updated_at_ms = 100;
        store
            .admit_managed_call(&fresh, &fresh_request)
            .await
            .unwrap();
        assert_eq!(
            store.managed_calls("parent-1", None).await.unwrap()[0].call_id,
            fresh.call_id
        );
        // A retried Clear keeps its original time boundary, including equality.
        store.clear_events("parent-1", 100).await.unwrap();
        assert_eq!(
            store.managed_calls("parent-1", None).await.unwrap().len(),
            1
        );
        let mut fresh_done = fresh.clone();
        fresh_done.state = State::Failed;
        fresh_done.revision += 1;
        assert!(
            store
                .advance_managed_call(&fresh, &fresh_done)
                .await
                .unwrap()
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
        assert!(
            !store
                .managed_calls("parent-1", None)
                .await
                .unwrap()
                .iter()
                .any(|call| call.call_id == overlapping_record.call_id)
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
