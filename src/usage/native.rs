//! Owner-scoped observations; native credentials never enter the Controller.
use crate::store::provider_accounts::logins::NativeCompletion;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

type Entries = BTreeMap<(String, String), Arc<Slot>>;

#[derive(Default, Clone)]
pub(crate) struct Cache(Arc<parking_lot::Mutex<Entries>>);

#[derive(Default)]
pub(crate) struct Slot {
    pub data: parking_lot::Mutex<Entry>,
    pub collection: Arc<Mutex<()>>,
}

#[derive(Default, Clone)]
pub(crate) struct Entry {
    pub scope: Option<NativeCompletion>,
    pub usage: Option<serde_json::Value>,
    pub last_attempt_at_ms: i64,
    pub error: Option<&'static str>,
}

impl Cache {
    pub fn entry(&self, owner: &str, account: &str) -> Arc<Slot> {
        Arc::clone(
            self.0
                .lock()
                .entry((owner.into(), account.into()))
                .or_default(),
        )
    }

    pub fn retain(&self, owner: &str, ids: &[&str]) {
        self.0.lock().retain(|(entry_owner, account), _| {
            entry_owner != owner || ids.contains(&account.as_str())
        });
    }
}

impl Entry {
    pub fn bind(&mut self, completion: &NativeCompletion) {
        if self.scope.as_ref() != Some(completion) {
            *self = Self {
                scope: Some(completion.clone()),
                ..Self::default()
            };
        }
    }

    pub fn due(&self, now: i64) -> bool {
        now.saturating_sub(self.last_attempt_at_ms) >= 300_000
    }

    pub fn finish(
        &mut self,
        scope: &NativeCompletion,
        result: Result<serde_json::Value, &'static str>,
    ) {
        if self.scope.as_ref() != Some(scope) {
            return;
        }
        match result {
            Ok(value) => {
                self.usage = Some(value);
                self.error = None;
            }
            Err(code) => {
                self.error = Some(code);
                if matches!(
                    code,
                    "machine_usage_update_required" | "machine_unavailable"
                ) {
                    self.last_attempt_at_ms = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cache_is_owner_and_account_scoped_and_attempts_are_coalesced() {
        let cache = Cache::default();
        let first = cache.entry("owner", "otter");
        let _collection = first.collection.lock().await;
        let mut entry = first.data.lock();
        entry.last_attempt_at_ms = 1000;
        entry.usage = Some(serde_json::json!({"rate_limits":{"remaining":71}}));
        assert!(cache.entry("owner", "otter").collection.try_lock().is_err());
        assert!(cache.entry("other", "otter").data.lock().usage.is_none());
        assert!(cache.entry("owner", "lynx").data.lock().usage.is_none());
        assert!(!entry.due(300_999));
        assert!(entry.due(301_000));
    }

    #[test]
    fn superseded_query_cannot_fill_a_new_login_and_failure_retains_only_its_own_snapshot() {
        let old: NativeCompletion = serde_json::from_value(serde_json::json!({
            "machine_id":"machine","recipient_fingerprint":"key",
            "profile":{"account_id":"11111111-1111-4111-8111-111111111111",
            "provider_id":"claude-code","request_id":"login","generation":1,"method":"native",
            "provider_version":"1","generation_digest":"digest","auth_contract_fingerprint":"contract",
            "identity":{"authority":"claude","subject":"otter"}}
        })).unwrap();
        let mut new = old.clone();
        new.profile.generation += 1;
        let mut entry = Entry::default();
        entry.bind(&old);
        let usage = serde_json::json!({"rate_limits":{"remaining":71}});
        entry.finish(&old, Ok(usage.clone()));
        entry.finish(&old, Err("native_usage_unavailable"));
        assert_eq!(entry.usage, Some(usage.clone()));
        assert!(entry.error.is_some());
        entry.bind(&new);
        assert!(entry.usage.is_none());
        entry.finish(&old, Ok(usage));
        assert!(entry.usage.is_none());
        assert!(entry.error.is_none());
    }
}
