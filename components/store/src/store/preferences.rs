//! Typed user preferences. A scope revision and its replay receipt commit together.
use super::{StorageBackend, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
mod superagent;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Preferences {
    pub scope: String,
    pub revision: i64,
    pub values: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferenceChange {
    pub scope: String,
    pub expected_revision: i64,
    pub changes: Value,
}

#[derive(Debug, PartialEq)]
pub enum PreferenceResult {
    Applied(Preferences),
    Conflict(Preferences),
    Invalid(&'static str),
}

/// Days a trashed session or folder is kept before permanent deletion.
pub const TRASH_RETENTION_DEFAULT_DAYS: i64 = 30;
pub const TRASH_RETENTION_MAX_DAYS: i64 = 365;

/// The owner's Trash retention in days, or the default for a missing or
/// malformed value.
pub fn trash_retention_days(preferences: Option<&Preferences>) -> i64 {
    preferences
        .and_then(|prefs| prefs.values.get("retention_days"))
        .and_then(Value::as_i64)
        .filter(|days| (1..=TRASH_RETENTION_MAX_DAYS).contains(days))
        .unwrap_or(TRASH_RETENTION_DEFAULT_DAYS)
}

pub fn defaults(scope: &str) -> Option<Preferences> {
    let values = match scope {
        "about" => json!({"usage_order":[],"machine_order":[]}),
        "trash" => json!({"retention_days": TRASH_RETENTION_DEFAULT_DAYS}),
        crate::superagent::SCOPE => json!({"configuration":crate::superagent::Settings::default()}),
        crate::context_rollover::PREFERENCE_SCOPE => crate::context_rollover::default_preferences(),
        _ => return None,
    };
    Some(Preferences {
        scope: scope.into(),
        revision: 0,
        values,
    })
}

pub fn schema() -> Value {
    json!({"scopes":{"code":{"meaning":"Review directory defaults: the code context Review opens for a session","storage":"database via existing folder/mobile-review sync","ownership":"calling session or its owner's folder",
        "read":"cowboy.preferences.get('code')",
        "update":{"action":"preferences.update","arguments":{"scope":"code","folder_id":"optional owned folder ID; omit for this session","expected":"exact override object from get: {} or {directory: value}","change":"{inherit:true} or {directory:null|'project'|'workspace::<machine>::<workspace>'|'session::<id>'}"}},
        "receipts":"preferences domain; accepted means queued, not committed. The 4096 most recently used receipts expire on eviction or Controller restart. On unknown, read current state before retry; retain expected/input/operation.",
        "activation":"Next code-context entry or Open default directory; never moves Agent workspace or starts a worker."},
        "superagent":{"storage":"database","ownership":"current_user","default":{"configuration":crate::superagent::Settings::default()},
        "update":"preferences.update with expected_revision and changes:{configuration}; supplied sources/projects/sessions replace their field, omitted fields are retained; atomic validation and durable receipt",
        "discovery":"cowboy_read({help:'superagent'})","semantics":"Explicit project coordinator bindings and session enrollment; immutable Markdown knowledge grants no external-action permission. Coordinators cannot edit settings."},
        "context_rollover":{"storage":"database","ownership":"current_user",
        "fields":{"enabled":{"type":"boolean","default":true},
                  "min_context_tokens":{"type":"integer","min":8_000,"max":2_000_000,"default":crate::context_rollover::DEFAULT_MIN_CONTEXT_TOKENS},
                  "idle_minutes":{"type":"provider_minutes_map","default":{},"values":"integer 1..10080, or null to never renew that Provider","builtin":{"claude-code":60}}},
        "semantics":"Default for renewing a session's agent context when its next message arrives after the Provider prompt cache expired: same session, worktree and settings; a fresh native context reads the earlier history on demand. Applies only when idle >= idle_minutes for the session's Provider and the context holds >= min_context_tokens. idle_minutes replaces the complete map; omitted Providers use the built-in preset (only claude-code renews by default). Per-session override: sessions.context_rollover_set."},
        "trash":{"storage":"database","ownership":"current_user",
        "fields":{"retention_days":{"type":"integer","min":1,"max":365,"default":30}},
        "semantics":"Days a session or folder stays in Trash before the Controller deletes it permanently. Applies to items already in Trash: each deadline is trashed_at + retention_days. Move items with sessions.trash, bring them back with sessions.restore."},
        "about":{"storage":"database","ownership":"current_user",
        "fields":{"usage_order":{"type":"unique_id_array","default":[]},
                  "machine_order":{"type":"unique_id_array","default":[]}},
        "unique_id_array":{"max_items":256,"id_pattern":"^[A-Za-z0-9_.:-]{1,128}$"},
        "semantics":"Listed IDs first; unlisted items retain default order. Empty arrays restore defaults. Omitted fields are unchanged."}}})
}

fn valid(change: &PreferenceChange) -> bool {
    if change.scope == crate::superagent::SCOPE {
        return change.expected_revision >= 0 && crate::superagent::patch_shape(&change.changes);
    }
    defaults(&change.scope).is_some()
        && change.expected_revision >= 0
        && change.changes.as_object().is_some_and(|fields| {
            !fields.is_empty()
                && fields.iter().all(|(key, value)| {
                    if change.scope == crate::context_rollover::PREFERENCE_SCOPE {
                        return crate::context_rollover::valid_preference(
                            key,
                            value,
                            super::provider_accounts::provider_id_valid,
                        );
                    }
                    if change.scope == "trash" {
                        return key == "retention_days"
                            && value.as_i64().is_some_and(|days| {
                                (1..=TRASH_RETENTION_MAX_DAYS).contains(&days)
                            });
                    }
                    matches!(key.as_str(), "usage_order" | "machine_order")
                        && value.as_array().is_some_and(|ids| {
                            let mut unique = std::collections::HashSet::new();
                            ids.len() <= 256
                                && ids.iter().all(|id| {
                                    id.as_str().is_some_and(|id| {
                                        !id.is_empty()
                                            && id.len() <= 128
                                            && id.bytes().all(|c| {
                                                c.is_ascii_alphanumeric() || b"-_.:".contains(&c)
                                            })
                                            && unique.insert(id)
                                    })
                                })
                        })
                })
        })
}

impl Store {
    pub async fn preferences(&self, owner: &str, scope: &str) -> Result<Option<Preferences>> {
        let Some(default) = defaults(scope) else {
            return Ok(None);
        };
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT value FROM user_preferences WHERE owner_user_id=$1 AND scope=$2",
                )
                .bind(owner)
                .bind(scope)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(Some(
            value
                .map(|v| serde_json::from_str(&v))
                .transpose()?
                .unwrap_or(default),
        ))
    }

    /// The retired `subscriptions` scope (legacy card labels) as it was last
    /// stored, read-only: a Web release from before the legacy card
    /// retirement compares its revision. A revision-zero empty value when the
    /// owner never wrote one. Nothing writes this scope any more.
    pub async fn retired_subscriptions_preferences(&self, owner: &str) -> Result<Preferences> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT value FROM user_preferences WHERE owner_user_id=$1 AND scope='subscriptions'",
                )
                .bind(owner)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(match value {
            Some(value) => serde_json::from_str(&value)?,
            None => Preferences {
                scope: "subscriptions".into(),
                revision: 0,
                values: json!({"legacy_labels":{}}),
            },
        })
    }

    pub async fn preference_receipt(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<Preferences>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT result FROM user_preference_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&$db.pool).await?
        }; }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    pub async fn update_preferences(
        &self,
        owner: &str,
        operation: &str,
        change: &PreferenceChange,
    ) -> Result<PreferenceResult> {
        if !valid(change) || !crate::managed_calls::valid_id(operation) {
            return Ok(PreferenceResult::Invalid("invalid_preferences"));
        }
        let hash = crate::digest::hex_sha256(&serde_json::to_vec(change)?);
        macro_rules! write { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            // Acquire the owner's writer lock before reading, including on SQLite.
            sqlx::query("INSERT INTO user_preference_owners (owner_user_id, revision) VALUES ($1, 1) ON CONFLICT (owner_user_id) DO UPDATE SET revision=user_preference_owners.revision+1")
                .bind(owner).execute(&mut *tx).await?;
            let receipt: Option<(String,String)> = sqlx::query_as("SELECT request_hash,result FROM user_preference_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_optional(&mut *tx).await?;
            if let Some((old_hash, result)) = receipt {
                return Ok(if hash == old_hash { PreferenceResult::Applied(serde_json::from_str(&result)?) }
                    else { PreferenceResult::Invalid("operation_reused") });
            }
            let value: Option<String> = sqlx::query_scalar("SELECT value FROM user_preferences WHERE owner_user_id=$1 AND scope=$2")
                .bind(owner).bind(&change.scope).fetch_optional(&mut *tx).await?;
            let mut next: Preferences = value.map(|v| serde_json::from_str(&v)).transpose()?.unwrap_or_else(|| defaults(&change.scope).unwrap());
            if next.revision != change.expected_revision { return Ok(PreferenceResult::Conflict(next)); }
            if change.scope == crate::superagent::SCOPE {
                match crate::superagent::Settings::patch_value(&next.values["configuration"], &change.changes) {
                    Ok(configuration) => next.values["configuration"] = serde_json::to_value(configuration)?,
                    Err(code) => return Ok(PreferenceResult::Invalid(code)),
                }
            } else {
                for (key,value) in change.changes.as_object().unwrap() { next.values[key] = value.clone(); }
            }
            next.revision += 1;
            let encoded = serde_json::to_string(&next)?;
            sqlx::query("INSERT INTO user_preferences (owner_user_id,scope,value) VALUES ($1,$2,$3) ON CONFLICT (owner_user_id,scope) DO UPDATE SET value=excluded.value")
                .bind(owner).bind(&change.scope).bind(&encoded).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO user_preference_operations (owner_user_id,operation_id,request_hash,result) VALUES ($1,$2,$3,$4)")
                .bind(owner).bind(operation).bind(&hash).bind(&encoded).execute(&mut *tx).await?;
            tx.commit().await?;
            PreferenceResult::Applied(next)
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_lines)] // Shared end-to-end persistence/replay contract for both databases.
    async fn contract(store: &Store) {
        let owner = format!("preferences-test-{}", uuid::Uuid::new_v4());
        let other = format!("{owner}-other");
        let change = PreferenceChange {
            scope: "about".into(),
            expected_revision: 0,
            changes: json!({"usage_order":["openai","anthropic"]}),
        };
        assert_eq!(
            store
                .preferences(&owner, "about")
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        let applied = store
            .update_preferences(&owner, "first", &change)
            .await
            .unwrap();
        let PreferenceResult::Applied(first) = &applied else {
            panic!("not applied");
        };
        assert_eq!(first.revision, 1);
        assert_eq!(first.values["machine_order"], json!([]));
        assert_eq!(
            store
                .update_preferences(&owner, "first", &change)
                .await
                .unwrap(),
            applied
        );
        let machine = PreferenceChange {
            expected_revision: 1,
            changes: json!({"machine_order":["hawk"]}),
            ..change.clone()
        };
        let competing = PreferenceChange {
            expected_revision: 1,
            changes: json!({"machine_order":["ovh"]}),
            ..change.clone()
        };
        let (a, b) = tokio::join!(
            store.update_preferences(&owner, "second", &machine),
            store.update_preferences(&owner, "third", &competing)
        );
        assert!(matches!(
            (&a.as_ref().unwrap(), &b.as_ref().unwrap()),
            (PreferenceResult::Applied(_), PreferenceResult::Conflict(_))
                | (PreferenceResult::Conflict(_), PreferenceResult::Applied(_))
        ));
        assert_eq!(
            store
                .preferences(&owner, "about")
                .await
                .unwrap()
                .unwrap()
                .revision,
            2
        );
        // Receipts preserve their exact committed result after later writes.
        assert_eq!(
            store
                .preference_receipt(&owner, "first")
                .await
                .unwrap()
                .as_ref(),
            Some(first)
        );
        assert_eq!(
            store
                .update_preferences(&owner, "first", &change)
                .await
                .unwrap(),
            applied
        );
        assert_eq!(
            store.preference_receipt(&other, "first").await.unwrap(),
            None
        );
        assert_eq!(
            store
                .preferences(&other, "about")
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        assert_eq!(
            store
                .update_preferences(&owner, "first", &machine)
                .await
                .unwrap(),
            PreferenceResult::Invalid("operation_reused")
        );
        let reset = PreferenceChange {
            expected_revision: 2,
            changes: json!({"usage_order":[]}),
            ..change
        };
        assert!(matches!(
            store
                .update_preferences(&owner, "reset", &reset)
                .await
                .unwrap(),
            PreferenceResult::Applied(_)
        ));
        assert_eq!(
            store
                .preferences(&owner, "about")
                .await
                .unwrap()
                .unwrap()
                .values["usage_order"],
            json!([])
        );
    }

    #[tokio::test]
    async fn the_retired_subscriptions_scope_stays_readable_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("prefs.sqlite").display()
        );
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        // The general reader no longer knows the scope ...
        assert!(
            store
                .preferences("owner", "subscriptions")
                .await
                .unwrap()
                .is_none()
        );
        // ... the compatibility projection always answers.
        let empty = store
            .retired_subscriptions_preferences("owner")
            .await
            .unwrap();
        assert_eq!(
            (empty.revision, empty.values),
            (0, json!({"legacy_labels":{}}))
        );
        let StorageBackend::Sqlite(db) = &store.backend else {
            panic!("sqlite")
        };
        let stored = json!({"scope":"subscriptions","revision":4,"values":{"legacy_labels":{"codex":"OpenAI · Pika"}}});
        sqlx::query("INSERT INTO user_preferences (owner_user_id,scope,value) VALUES ('owner','subscriptions',$1)")
            .bind(stored.to_string())
            .execute(&db.pool)
            .await
            .unwrap();
        let kept = store
            .retired_subscriptions_preferences("owner")
            .await
            .unwrap();
        assert_eq!(kept.revision, 4);
        assert_eq!(kept.values["legacy_labels"]["codex"], "OpenAI · Pika");
    }

    #[tokio::test]
    async fn sqlite_preferences_survive_reopen_and_serialize_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("prefs.sqlite").display()
        );
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        contract(&store).await;
        let change = PreferenceChange {
            scope: "about".into(),
            expected_revision: 0,
            changes: json!({"machine_order":["persisted"]}),
        };
        store
            .update_preferences("restart-owner", "persist", &change)
            .await
            .unwrap();
        drop(store);
        let reopened = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .preferences("restart-owner", "about")
                .await
                .unwrap()
                .unwrap()
                .values["machine_order"],
            json!(["persisted"])
        );
        assert!(
            reopened
                .preference_receipt("restart-owner", "persist")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
    async fn postgres_preferences_contract() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        contract(&store).await;
    }

    #[test]
    fn the_retired_legacy_label_preference_is_no_scope() {
        // The legacy cards are retired (docs/legacy-card-retirement.md).
        assert!(defaults("subscriptions").is_none());
        assert!(schema()["scopes"].get("subscriptions").is_none());
        assert!(!valid(&PreferenceChange {
            scope: "subscriptions".into(),
            expected_revision: 0,
            changes: json!({"legacy_labels":{"claude-code":"Claude · Lynx"}})
        }));
    }

    #[test]
    fn closed_schema_rejects_unknown_fields_duplicate_ids_and_unbounded_input() {
        for changes in [
            json!({}),
            json!({"machine_order":["a","a"]}),
            json!({"machine_order":null}),
            json!({"secret":"x"}),
            json!({"usage_order":["bad\n"]}),
            json!({"usage_order":["x".repeat(129)]}),
        ] {
            assert!(!valid(&PreferenceChange {
                scope: "about".into(),
                expected_revision: 0,
                changes
            }));
        }
        assert!(defaults("service").is_none());
        assert!(serde_json::from_value::<PreferenceChange>(json!({"scope":"about","expected_revision":0,"changes":{"usage_order":[]},"owner":"other"})).is_err());
    }
}
