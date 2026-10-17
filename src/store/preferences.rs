//! Typed user preferences. A scope revision and its replay receipt commit together.
use super::{StorageBackend, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct Preferences {
    pub scope: String,
    pub revision: i64,
    pub values: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreferenceChange {
    pub scope: String,
    pub expected_revision: i64,
    pub changes: Value,
}

#[derive(Debug, PartialEq)]
pub(crate) enum PreferenceResult {
    Applied(Preferences),
    Conflict(Preferences),
    Invalid(&'static str),
}

pub(crate) fn defaults(scope: &str) -> Option<Preferences> {
    (scope == "about").then(|| Preferences {
        scope: scope.into(),
        revision: 0,
        values: json!({"usage_order":[],"machine_order":[]}),
    })
}

pub(crate) fn schema() -> Value {
    json!({"scopes":{"about":{"storage":"database","ownership":"current_user",
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
    pub(crate) async fn preferences(
        &self,
        owner: &str,
        scope: &str,
    ) -> Result<Option<Preferences>> {
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

    pub(crate) async fn preference_receipt(
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

    pub(crate) async fn update_preferences(
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
