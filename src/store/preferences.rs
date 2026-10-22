//! Typed user preferences. A scope revision and its replay receipt commit together.
use super::{StorageBackend, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
        "subscriptions" => json!({"legacy_labels":{}}),
        "trash" => json!({"retention_days": TRASH_RETENTION_DEFAULT_DAYS}),
        _ => return None,
    };
    Some(Preferences {
        scope: scope.into(),
        revision: 0,
        values,
    })
}

pub fn schema() -> Value {
    json!({"scopes":{"zed":{"storage":"database via existing folder/mobile-review sync","ownership":"calling session or its owner's folder",
        "read":"cowboy.preferences.get('zed')",
        "update":{"action":"preferences.update","arguments":{"scope":"zed","folder_id":"optional owned folder ID; omit for this session","expected":"exact override object from get: {} or {directory: value}","change":"{inherit:true} or {directory:null|'project'|'workspace::<machine>::<workspace>'|'session::<id>'}"}},
        "receipts":"preferences domain; accepted means queued, not committed. The 4096 most recently used receipts expire on eviction or Controller restart. On unknown, read current state before retry; retain expected/input/operation.",
        "activation":"Next code-context entry or Open default directory; never moves Agent workspace or starts a worker."},
        "subscriptions":{"storage":"database","ownership":"current_user",
        "fields":{"legacy_labels":{"type":"provider_label_map","default":{}}},
        "provider_label_map":{"max_items":128,"provider_pattern":"^[a-z0-9-]{1,64}$","label_max_utf8_bytes":128},
        "semantics":"Display names for each Provider's legacy default account. Replace the complete map using the observed revision; omitted entries restore Existing account. Labels must be trimmed, nonempty and contain no control characters. Does not change account identity, credentials, admission or session defaults."},
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
    defaults(&change.scope).is_some()
        && change.expected_revision >= 0
        && change.changes.as_object().is_some_and(|fields| {
            !fields.is_empty()
                && fields.iter().all(|(key, value)| {
                    if change.scope == "trash" {
                        return key == "retention_days"
                            && value.as_i64().is_some_and(|days| {
                                (1..=TRASH_RETENTION_MAX_DAYS).contains(&days)
                            });
                    }
                    if change.scope == "subscriptions" {
                        return key == "legacy_labels"
                            && value.as_object().is_some_and(|labels| {
                                labels.len() <= 128
                                    && labels.iter().all(|(provider, label)| {
                                        super::provider_accounts::provider_id_valid(provider)
                                            && label.as_str().is_some_and(|label| {
                                                label == label.trim()
                                                    && super::provider_accounts::label_valid(label)
                                            })
                                    })
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
        let hash = crate::admin::hex_sha256(&serde_json::to_vec(change)?);
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
            for (key,value) in change.changes.as_object().unwrap() { next.values[key] = value.clone(); }
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

    async fn legacy_label_contract(store: &Store) {
        let owner = uuid::Uuid::new_v4().to_string();
        let registry = store.provider_accounts(&owner).await.unwrap();
        let change = PreferenceChange {
            scope: "subscriptions".into(),
            expected_revision: 0,
            changes: json!({"legacy_labels":{"claude-code":"Claude · Lynx","codex":"Codex · Fox"}}),
        };
        let first = store
            .update_preferences(&owner, "label", &change)
            .await
            .unwrap();
        assert!(matches!(&first, PreferenceResult::Applied(p) if p.revision == 1));
        assert_eq!(
            store
                .update_preferences(&owner, "label", &change)
                .await
                .unwrap(),
            first
        );
        assert!(matches!(
            store
                .update_preferences(&owner, "stale", &change)
                .await
                .unwrap(),
            PreferenceResult::Conflict(_)
        ));
        assert_eq!(store.provider_accounts(&owner).await.unwrap(), registry);
        assert_eq!(
            store
                .preferences("other-owner", "subscriptions")
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        let reset = PreferenceChange {
            expected_revision: 1,
            changes: json!({"legacy_labels":{}}),
            ..change.clone()
        };
        assert!(matches!(
            store
                .update_preferences(&owner, "reset-labels", &reset)
                .await
                .unwrap(),
            PreferenceResult::Applied(_)
        ));
        assert_eq!(
            store
                .preferences(&owner, "subscriptions")
                .await
                .unwrap()
                .unwrap()
                .values,
            json!({"legacy_labels":{}})
        );
        // The exact historical result remains readable after the display changed.
        assert_eq!(
            store
                .update_preferences(&owner, "label", &change)
                .await
                .unwrap(),
            first
        );
    }

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
        legacy_label_contract(&store).await;
        let labels = PreferenceChange {
            scope: "subscriptions".into(),
            expected_revision: 0,
            changes: json!({"legacy_labels":{"claude-code":"Claude · Lynx"}}),
        };
        store
            .update_preferences("restart-owner", "persist-label", &labels)
            .await
            .unwrap();
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
                .preferences("restart-owner", "subscriptions")
                .await
                .unwrap()
                .unwrap()
                .values,
            labels.changes
        );
        assert!(
            reopened
                .preference_receipt("restart-owner", "persist-label")
                .await
                .unwrap()
                .is_some()
        );
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
        legacy_label_contract(&store).await;
    }

    #[test]
    fn legacy_labels_are_bounded_display_metadata_only() {
        for changes in [
            json!({"legacy_labels":null}),
            json!({"legacy_labels":[]}),
            json!({"legacy_labels":{"claude-code":" "}}),
            json!({"legacy_labels":{"claude-code":" padded "}}),
            json!({"legacy_labels":{"claude-code":"a\nb"}}),
            json!({"legacy_labels":{"claude-code":"界".repeat(43)}}),
            json!({"legacy_labels":{"bad/provider":"Name"}}),
            json!({"legacy_labels":{"CLAUDE":"Name"}}),
            json!({"legacy_labels":{"claude-code":{"token":"secret"}}}),
            json!({"legacy_labels":{},"enabled":true}),
            json!({"usage_order":[]}),
        ] {
            assert!(!valid(&PreferenceChange {
                scope: "subscriptions".into(),
                expected_revision: 0,
                changes
            }));
        }
        let labels: serde_json::Map<String, Value> = (0..129)
            .map(|i| (format!("provider-{i}"), json!("Label")))
            .collect();
        assert!(!valid(&PreferenceChange {
            scope: "subscriptions".into(),
            expected_revision: 0,
            changes: json!({"legacy_labels":labels})
        }));
        assert!(!valid(&PreferenceChange {
            scope: "about".into(),
            expected_revision: 0,
            changes: json!({"legacy_labels":{}})
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
