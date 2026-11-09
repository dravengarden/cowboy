//! Widget interaction records (docs/widgets.md §8). The decision travels as an
//! ordinary user prompt; this table only remembers which input, revision and
//! option labels it answered, keyed by that prompt's client message id.
use super::super::{StorageBackend, Store};
use super::StoredWidget;
use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredInteraction {
    pub cmid: String,
    pub revision: i64,
    pub node_id: String,
    /// The node's `group`, or its id when it has none: one decision per key.
    pub group_key: String,
    /// Digest of what the person saw (labels and options); a later revision
    /// that changes it asks again.
    pub signature: String,
    pub labels: Vec<String>,
    pub message: String,
    pub created_at_ms: i64,
}

type Row = (String, i64, String, String, String, String, String, i64);

fn interaction(row: Row) -> Result<StoredInteraction> {
    let (cmid, revision, node_id, group_key, signature, labels, message, created_at_ms) = row;
    Ok(StoredInteraction {
        cmid,
        revision,
        node_id,
        group_key,
        signature,
        labels: serde_json::from_str(&labels)?,
        message,
        created_at_ms,
    })
}

const COLUMNS: &str =
    "cmid, revision, node_id, group_key, signature, labels, message, created_at_ms";

impl Store {
    /// The widget's decisions, newest first: `active` keeps only those no
    /// update retired (the authoritative decided state, unbounded because at
    /// most one per input group stays active); otherwise the newest `limit`.
    pub async fn widget_interactions(
        &self,
        session_id: &str,
        widget_id: &str,
        active: bool,
        limit: i64,
    ) -> Result<Vec<StoredInteraction>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM session_widget_interactions \
             WHERE session_id = $1 AND widget_id = $2 AND ($3 = FALSE OR retired = FALSE) \
             ORDER BY created_at_ms DESC, cmid LIMIT $4"
        );
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, Row>(&sql)
                    .bind(session_id)
                    .bind(widget_id)
                    .bind(active)
                    .bind(limit)
                    .fetch_all(&$db.pool)
                    .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        rows.into_iter().map(interaction).collect()
    }

    /// The decision recorded under `cmid`, whatever its widget.
    pub async fn widget_interaction(
        &self,
        session_id: &str,
        cmid: &str,
    ) -> Result<Option<StoredInteraction>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM session_widget_interactions WHERE session_id = $1 AND cmid = $2"
        );
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, Row>(&sql)
                    .bind(session_id)
                    .bind(cmid)
                    .fetch_optional(&$db.pool)
                    .await?
            };
        }
        let row = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        row.map(interaction).transpose()
    }

    /// Retire decisions an update made obsolete; they never settle again.
    pub async fn retire_widget_interactions(
        &self,
        session_id: &str,
        widget_id: &str,
        cmids: &[String],
    ) -> Result<()> {
        for cmid in cmids {
            let sql = "UPDATE session_widget_interactions SET retired = TRUE \
                       WHERE session_id = $1 AND widget_id = $2 AND cmid = $3";
            macro_rules! retire {
                ($db:expr) => {
                    sqlx::query(sql)
                        .bind(session_id)
                        .bind(widget_id)
                        .bind(cmid)
                        .execute(&$db.pool)
                        .await?
                        .rows_affected()
                };
            }
            match &self.backend {
                StorageBackend::Postgres(db) => retire!(db),
                StorageBackend::Sqlite(db) => retire!(db),
            };
        }
        Ok(())
    }

    /// Whether the session's stored transcript holds the user echo of
    /// `cmid`: the durable proof that the prompt reached the conversation.
    pub async fn prompt_echoed(&self, session_id: &str, cmid: &str) -> Result<bool> {
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => sqlx::query_scalar::<_, i32>(
                "SELECT 1 FROM events WHERE session_id = $1 AND payload->>'cmid' = $2 LIMIT 1",
            )
            .bind(session_id)
            .bind(cmid)
            .fetch_optional(&db.pool)
            .await?
            .is_some(),
            StorageBackend::Sqlite(db) => sqlx::query_scalar::<_, i32>(
                "SELECT 1 FROM events WHERE session_id = $1 AND instr(payload, $2) > 0 \
                 AND json_extract(payload, '$.cmid') = $2 LIMIT 1",
            )
            .bind(session_id)
            .bind(cmid)
            .fetch_optional(&db.pool)
            .await?
            .is_some(),
        })
    }

    /// Active decisions since `since_ms` on live widgets whose prompt has no
    /// stored echo: `(session_id, cmid, message)`, oldest first.
    pub async fn undelivered_widget_decisions(
        &self,
        since_ms: i64,
    ) -> Result<Vec<(String, String, String)>> {
        let select = "SELECT i.session_id, i.cmid, i.message FROM session_widget_interactions i \
             JOIN session_widgets w ON w.session_id = i.session_id AND w.widget_id = i.widget_id \
             WHERE i.retired = FALSE AND w.cleared = FALSE AND i.created_at_ms >= $1 ";
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => sqlx::query_as(&format!(
                "{select} AND NOT EXISTS (SELECT 1 FROM events e WHERE e.session_id = i.session_id \
                 AND e.payload->>'cmid' = i.cmid) ORDER BY i.created_at_ms"
            ))
            .bind(since_ms)
            .fetch_all(&db.pool)
            .await?,
            StorageBackend::Sqlite(db) => sqlx::query_as(&format!(
                "{select} AND NOT EXISTS (SELECT 1 FROM events e WHERE e.session_id = i.session_id \
                 AND instr(e.payload, i.cmid) > 0 AND json_extract(e.payload, '$.cmid') = i.cmid) \
                 ORDER BY i.created_at_ms"
            ))
            .bind(since_ms)
            .fetch_all(&db.pool)
            .await?,
        })
    }

    /// Record a decision once per `cmid` and advance the widget's state
    /// version so every client redraws the input as decided. Returns the
    /// widget after the change, or `None` when the record already existed or
    /// the widget is gone.
    pub async fn record_widget_interaction(
        &self,
        session_id: &str,
        widget_id: &str,
        record: &StoredInteraction,
    ) -> Result<Option<StoredWidget>> {
        let labels = serde_json::to_string(&record.labels)?;
        macro_rules! record {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                // Reserve the writer first (SQLite deferred-upgrade rule).
                let bumped = sqlx::query(
                    "UPDATE session_widgets SET state_version = state_version + 1, updated_at_ms = $3 \
                     WHERE session_id = $1 AND widget_id = $2 AND cleared = FALSE",
                )
                .bind(session_id)
                .bind(widget_id)
                .bind(record.created_at_ms)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if bumped != 1 {
                    tx.rollback().await?;
                    return Ok(None);
                }
                let inserted = sqlx::query(
                    "INSERT INTO session_widget_interactions \
                     (session_id, widget_id, cmid, revision, node_id, group_key, signature, labels, message, created_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT DO NOTHING",
                )
                .bind(session_id)
                .bind(widget_id)
                .bind(&record.cmid)
                .bind(record.revision)
                .bind(&record.node_id)
                .bind(&record.group_key)
                .bind(&record.signature)
                .bind(&labels)
                .bind(&record.message)
                .bind(record.created_at_ms)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if inserted != 1 {
                    tx.rollback().await?;
                    return Ok(None);
                }
                tx.commit().await?;
            }};
        }
        match &self.backend {
            StorageBackend::Postgres(db) => record!(db),
            StorageBackend::Sqlite(db) => record!(db),
        }
        self.session_widget(session_id, widget_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_decision_is_recorded_once_and_bumps_the_state_version() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", directory.path().to_owned())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        store
            .insert_session(&super::super::super::storage_contract_tests::session("s1"))
            .await
            .unwrap();
        let document = json!({"schema":"cowboy.widget/1","title":"t","root":{"type":"divider"}});
        store
            .publish_session_widget("s1", "w", &document, 1)
            .await
            .unwrap();
        let record = StoredInteraction {
            cmid: "widget-abc".into(),
            revision: 1,
            node_id: "ok".into(),
            group_key: "ok".into(),
            signature: "sig".into(),
            labels: vec!["Approve".into()],
            message: "Widget #w1 (revision 1): Approve".into(),
            created_at_ms: 5,
        };
        let widget = store
            .record_widget_interaction("s1", "w", &record)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(widget.state_version, 2);
        assert!(
            store
                .record_widget_interaction("s1", "w", &record)
                .await
                .unwrap()
                .is_none(),
            "a retried decision records nothing new"
        );
        assert_eq!(
            store
                .widget_interactions("s1", "w", true, i64::MAX)
                .await
                .unwrap(),
            vec![record.clone()]
        );
        assert!(
            store
                .record_widget_interaction(
                    "s1",
                    "missing",
                    &StoredInteraction {
                        cmid: "widget-other".into(),
                        ..record.clone()
                    }
                )
                .await
                .unwrap()
                .is_none()
        );
    }
}
