//! Widget interaction records (docs/widgets.md §8). The decision travels as an
//! ordinary user prompt; this table only remembers which input, revision and
//! option labels it answered, keyed by that prompt's client message id.
use super::super::{StorageBackend, Store};
use super::StoredWidget;
use anyhow::Result;

/// Records kept and returned per widget, newest first.
pub const INTERACTIONS_PER_WIDGET: i64 = 200;

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
    /// The widget's interactions, newest first.
    pub async fn widget_interactions(
        &self,
        session_id: &str,
        widget_id: &str,
    ) -> Result<Vec<StoredInteraction>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM session_widget_interactions \
             WHERE session_id = $1 AND widget_id = $2 ORDER BY created_at_ms DESC, cmid LIMIT $3"
        );
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, Row>(&sql)
                    .bind(session_id)
                    .bind(widget_id)
                    .bind(INTERACTIONS_PER_WIDGET)
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
            store.widget_interactions("s1", "w").await.unwrap(),
            vec![record]
        );
        assert!(
            store
                .record_widget_interaction(
                    "s1",
                    "missing",
                    &StoredInteraction {
                        cmid: "widget-other".into(),
                        ..store.widget_interactions("s1", "w").await.unwrap()[0].clone()
                    }
                )
                .await
                .unwrap()
                .is_none()
        );
    }
}
