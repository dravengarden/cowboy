//! Authoritative widget documents (docs/widgets.md §6.3). Validation and the
//! transcript projection live with the Agent API writer; this module only
//! stores, compares revisions and tombstones.
use super::{StorageBackend, Store};
use anyhow::Result;
use serde_json::Value;

/// Open (not closed) widgets a session may hold at once.
pub const OPEN_WIDGETS_PER_SESSION: i64 = 200;
/// Widgets a session may hold in total, closed included.
pub const WIDGETS_PER_SESSION: i64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredWidget {
    pub widget_id: String,
    pub handle: String,
    pub revision: i64,
    pub state_version: i64,
    pub closed: bool,
    pub document: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, PartialEq)]
pub enum PublishOutcome {
    Committed(StoredWidget),
    Exists(StoredWidget),
    Limit,
}

/// What an update changes; validation happened before.
#[derive(Debug)]
pub enum WidgetWrite {
    Document(Value),
    Close(Option<String>),
}

#[derive(Debug, PartialEq)]
pub enum UpdateOutcome {
    Committed(StoredWidget),
    Conflict(i64),
    Unknown,
    Closed,
}

type Row = (String, String, i64, i64, bool, String, i64, i64);

fn widget(row: Row) -> Result<StoredWidget> {
    let (
        widget_id,
        handle,
        revision,
        state_version,
        closed,
        document,
        created_at_ms,
        updated_at_ms,
    ) = row;
    Ok(StoredWidget {
        widget_id,
        handle,
        revision,
        state_version,
        closed,
        document: serde_json::from_str(&document)?,
        created_at_ms,
        updated_at_ms,
    })
}

const COLUMNS: &str =
    "widget_id, handle, revision, state_version, closed, document, created_at_ms, updated_at_ms";

impl Store {
    /// A live (not cleared) widget of the session.
    pub async fn session_widget(
        &self,
        session_id: &str,
        widget_id: &str,
    ) -> Result<Option<StoredWidget>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM session_widgets WHERE session_id = $1 AND widget_id = $2 AND cleared = FALSE"
        );
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, Row>(&sql)
                    .bind(session_id)
                    .bind(widget_id)
                    .fetch_optional(&$db.pool)
                    .await?
            };
        }
        let row = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        row.map(widget).transpose()
    }

    /// Live widgets of the session in creation order after `after`.
    pub async fn session_widgets(
        &self,
        session_id: &str,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredWidget>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM session_widgets WHERE session_id = $1 AND cleared = FALSE \
             AND ($2 IS NULL OR widget_id > $2) ORDER BY widget_id LIMIT $3"
        );
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, Row>(&sql)
                    .bind(session_id)
                    .bind(after)
                    .bind(limit)
                    .fetch_all(&$db.pool)
                    .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        rows.into_iter().map(widget).collect()
    }

    /// Insert revision 1 of a widget, or report the existing one.
    pub async fn publish_session_widget(
        &self,
        session_id: &str,
        widget_id: &str,
        document: &Value,
        now_ms: i64,
    ) -> Result<PublishOutcome> {
        let encoded = serde_json::to_string(document)?;
        let select = format!(
            "SELECT {COLUMNS} FROM session_widgets WHERE session_id = $1 AND widget_id = $2"
        );
        macro_rules! publish { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            if let Some(row) = sqlx::query_as::<_, Row>(&select).bind(session_id).bind(widget_id).fetch_optional(&mut *tx).await? {
                tx.rollback().await?;
                return Ok(PublishOutcome::Exists(widget(row)?));
            }
            let (total, open): (i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COALESCE(SUM(CASE WHEN closed = FALSE AND cleared = FALSE THEN 1 ELSE 0 END), 0) \
                 FROM session_widgets WHERE session_id = $1",
            )
            .bind(session_id).fetch_one(&mut *tx).await?;
            if total >= WIDGETS_PER_SESSION || open >= OPEN_WIDGETS_PER_SESSION {
                tx.rollback().await?;
                return Ok(PublishOutcome::Limit);
            }
            let handle = format!("w{}", total + 1);
            sqlx::query(
                "INSERT INTO session_widgets (session_id, widget_id, handle, revision, state_version, closed, cleared, document, created_at_ms, updated_at_ms) \
                 VALUES ($1, $2, $3, 1, 1, FALSE, FALSE, $4, $5, $5)",
            )
            .bind(session_id).bind(widget_id).bind(&handle).bind(&encoded).bind(now_ms)
            .execute(&mut *tx).await?;
            tx.commit().await?;
            PublishOutcome::Committed(StoredWidget {
                widget_id: widget_id.to_owned(),
                handle,
                revision: 1,
                state_version: 1,
                closed: false,
                document: document.clone(),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            })
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => publish!(db),
            StorageBackend::Sqlite(db) => publish!(db),
        })
    }

    /// Compare-and-swap on the document revision of a live widget.
    pub async fn update_session_widget(
        &self,
        session_id: &str,
        widget_id: &str,
        expected_revision: i64,
        write: WidgetWrite,
        now_ms: i64,
    ) -> Result<UpdateOutcome> {
        let Some(current) = self.session_widget(session_id, widget_id).await? else {
            return Ok(UpdateOutcome::Unknown);
        };
        if current.closed {
            return Ok(UpdateOutcome::Closed);
        }
        if current.revision != expected_revision {
            return Ok(UpdateOutcome::Conflict(current.revision));
        }
        let (document, closed, revision) = match write {
            WidgetWrite::Document(document) => (document, false, current.revision + 1),
            WidgetWrite::Close(summary) => {
                let mut document = current.document.clone();
                if let Some(summary) = summary {
                    document["summary"] = Value::String(summary);
                }
                (document, true, current.revision)
            }
        };
        let encoded = serde_json::to_string(&document)?;
        macro_rules! update { ($db:expr) => {
            sqlx::query(
                "UPDATE session_widgets SET document = $1, revision = $2, state_version = state_version + 1, closed = $3, updated_at_ms = $4 \
                 WHERE session_id = $5 AND widget_id = $6 AND revision = $7 AND closed = FALSE AND cleared = FALSE",
            )
            .bind(&encoded).bind(revision).bind(closed).bind(now_ms)
            .bind(session_id).bind(widget_id).bind(expected_revision)
            .execute(&$db.pool).await?.rows_affected()
        }; }
        let affected = match &self.backend {
            StorageBackend::Postgres(db) => update!(db),
            StorageBackend::Sqlite(db) => update!(db),
        };
        if affected != 1 {
            // A concurrent write won; report what is there now.
            return Ok(match self.session_widget(session_id, widget_id).await? {
                None => UpdateOutcome::Unknown,
                Some(widget) if widget.closed => UpdateOutcome::Closed,
                Some(widget) => UpdateOutcome::Conflict(widget.revision),
            });
        }
        Ok(UpdateOutcome::Committed(StoredWidget {
            revision,
            state_version: current.state_version + 1,
            closed,
            document,
            updated_at_ms: now_ms,
            ..current
        }))
    }
}

/// Clearing a transcript drops its anchors (leaving the backfill sentinel) and
/// tombstones its widgets, which are never published or updated again
/// (docs/widgets.md §6.3). Runs inside the Clear transaction.
pub(super) async fn clear_transcript_projections_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session_id: &str,
) -> Result<()> {
    use anyhow::Context as _;
    sqlx::query("DELETE FROM transcript_anchors WHERE session_id = $1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("DELETE transcript anchors for {session_id}"))?;
    sqlx::query(
        "INSERT INTO transcript_anchors(session_id, anchor_key, seq, created_at_ms) VALUES ($1, $2, 0, $3)",
    )
    .bind(session_id)
    .bind(crate::transcript_anchors::BACKFILL_SENTINEL)
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(&mut **tx)
    .await
    .with_context(|| format!("INSERT anchor sentinel for {session_id}"))?;
    sqlx::query("UPDATE session_widgets SET cleared = TRUE WHERE session_id = $1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("tombstone widgets for {session_id}"))?;
    Ok(())
}

pub(super) async fn clear_transcript_projections_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_id: &str,
) -> Result<()> {
    use anyhow::Context as _;
    sqlx::query("DELETE FROM transcript_anchors WHERE session_id = ?1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("DELETE SQLite transcript anchors for {session_id}"))?;
    sqlx::query(
        "INSERT INTO transcript_anchors(session_id, anchor_key, seq, created_at_ms) VALUES (?1, ?2, 0, ?3)",
    )
    .bind(session_id)
    .bind(crate::transcript_anchors::BACKFILL_SENTINEL)
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(&mut **tx)
    .await
    .with_context(|| format!("INSERT SQLite anchor sentinel for {session_id}"))?;
    sqlx::query("UPDATE session_widgets SET cleared = TRUE WHERE session_id = ?1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .with_context(|| format!("tombstone SQLite widgets for {session_id}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn publish_update_close_and_clear() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", directory.path().to_owned())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        store
            .insert_session(&super::super::storage_contract_tests::session("s1"))
            .await
            .unwrap();
        let document = json!({"schema":"cowboy.widget/1","title":"T","root":{"type":"divider"}});
        let PublishOutcome::Committed(first) = store
            .publish_session_widget("s1", "a", &document, 10)
            .await
            .unwrap()
        else {
            panic!("not committed")
        };
        assert_eq!(
            (first.handle.as_str(), first.revision, first.state_version),
            ("w1", 1, 1)
        );
        assert!(matches!(
            store
                .publish_session_widget("s1", "a", &document, 11)
                .await
                .unwrap(),
            PublishOutcome::Exists(_)
        ));
        let next = json!({"schema":"cowboy.widget/1","title":"U","root":{"type":"divider"}});
        assert_eq!(
            store
                .update_session_widget("s1", "a", 7, WidgetWrite::Document(next.clone()), 12)
                .await
                .unwrap(),
            UpdateOutcome::Conflict(1)
        );
        let UpdateOutcome::Committed(updated) = store
            .update_session_widget("s1", "a", 1, WidgetWrite::Document(next), 12)
            .await
            .unwrap()
        else {
            panic!("not committed")
        };
        assert_eq!((updated.revision, updated.state_version), (2, 2));
        let UpdateOutcome::Committed(closed) = store
            .update_session_widget("s1", "a", 2, WidgetWrite::Close(Some("done".into())), 13)
            .await
            .unwrap()
        else {
            panic!("not committed")
        };
        assert!(closed.closed && closed.state_version == 3 && closed.document["summary"] == "done");
        assert_eq!(
            store
                .update_session_widget("s1", "a", 2, WidgetWrite::Close(None), 14)
                .await
                .unwrap(),
            UpdateOutcome::Closed
        );
        store.clear_events("s1", 0).await.unwrap();
        assert!(store.session_widget("s1", "a").await.unwrap().is_none());
        assert!(
            store
                .session_widgets("s1", None, 50)
                .await
                .unwrap()
                .is_empty()
        );
        let PublishOutcome::Committed(again) = store
            .publish_session_widget("s1", "b", &document, 15)
            .await
            .unwrap()
        else {
            panic!("not committed")
        };
        assert_eq!(again.handle, "w2", "handles are never reused after Clear");
    }
}
