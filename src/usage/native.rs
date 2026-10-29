//! Owner-scoped observations; native credentials never enter the Controller.
use crate::store::provider_accounts::logins::NativeCompletion;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

type Entries = BTreeMap<(String, String), Arc<Slot>>;

/// Successful observations optionally survive a Controller restart, so a
/// named card does not go blank while the persisted request budget still
/// defers its next collection. Each stays bound to its exact login.
#[derive(Default, Clone)]
pub struct Cache(Arc<parking_lot::Mutex<Entries>>, Option<Arc<PathBuf>>);

#[derive(serde::Serialize, serde::Deserialize)]
struct Persisted {
    schema: u32,
    entries: Vec<PersistedEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedEntry {
    owner: String,
    account: String,
    scope: NativeCompletion,
    usage: serde_json::Value,
    last_attempt_at_ms: i64,
}

#[derive(Default)]
pub struct Slot {
    pub data: parking_lot::Mutex<Entry>,
    pub collection: Arc<Mutex<()>>,
}

#[derive(Default, Clone)]
pub struct Entry {
    pub scope: Option<NativeCompletion>,
    pub usage: Option<serde_json::Value>,
    pub last_attempt_at_ms: i64,
    pub error: Option<&'static str>,
}

impl Cache {
    /// A cache restored from, and saved to, `path`. A missing or unreadable
    /// file starts empty; it only ever holds non-secret observations.
    #[must_use]
    pub fn persistent(path: PathBuf) -> Self {
        let entries = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Persisted>(&bytes).ok())
            .filter(|persisted| persisted.schema == 1)
            .map(|persisted| {
                persisted
                    .entries
                    .into_iter()
                    .map(|entry| {
                        let slot = Slot::default();
                        *slot.data.lock() = Entry {
                            scope: Some(entry.scope),
                            usage: Some(entry.usage),
                            last_attempt_at_ms: entry.last_attempt_at_ms,
                            error: None,
                        };
                        ((entry.owner, entry.account), Arc::new(slot))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self(
            Arc::new(parking_lot::Mutex::new(entries)),
            Some(Arc::new(path)),
        )
    }

    /// Save every successful bound observation. Failure only loses the
    /// restart continuity, never an observation in memory.
    pub fn persist(&self) {
        let Some(path) = self.1.as_deref() else {
            return;
        };
        let entries = self
            .0
            .lock()
            .iter()
            .filter_map(|((owner, account), slot)| {
                let data = slot.data.lock();
                Some(PersistedEntry {
                    owner: owner.clone(),
                    account: account.clone(),
                    scope: data.scope.clone()?,
                    usage: data.usage.clone()?,
                    last_attempt_at_ms: data.last_attempt_at_ms,
                })
            })
            .collect();
        if let Err(error) = save(path, &Persisted { schema: 1, entries }) {
            tracing::warn!(%error, "named account usage could not be persisted");
        }
    }

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

fn save(path: &Path, persisted: &Persisted) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec(persisted)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::File::open(parent)?.sync_all()
}

impl Entry {
    #[must_use]
    pub fn snapshot(&self, completion: &NativeCompletion) -> Self {
        if self.scope.as_ref() == Some(completion) {
            self.clone()
        } else {
            Self::default()
        }
    }

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

    fn completion(generation: u64) -> NativeCompletion {
        serde_json::from_value(serde_json::json!({
            "machine_id": "ovh",
            "recipient_fingerprint": "recipient",
            "profile": {
                "account_id": "fd376b94-c493-4d27-81da-67a1d154ca17",
                "provider_id": "claude-code",
                "generation": generation,
                "provider_version": "1",
                "generation_digest": "digest",
                "auth_contract_fingerprint": "contract",
                "method": "native",
                "request_id": "login",
                "identity": {"authority": "fixture", "subject": "personal"}
            }
        }))
        .unwrap()
    }

    #[test]
    fn a_successful_observation_survives_restart_only_for_its_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage-named.json");
        let cache = Cache::persistent(path.clone());
        let login = completion(4);
        {
            let slot = cache.entry("owner", "mink");
            let mut entry = slot.data.lock();
            entry.bind(&login);
            entry.last_attempt_at_ms = 1000;
            entry.finish(&login, Ok(serde_json::json!({"rate_limits": {"used": 3}})));
        }
        // A failure is never persisted over a success, nor shown after restart.
        cache.entry("owner", "lynx").data.lock().bind(&login);
        cache.persist();
        let restored = Cache::persistent(path.clone());
        let entry = restored.entry("owner", "mink").data.lock().snapshot(&login);
        assert_eq!(
            entry.usage,
            Some(serde_json::json!({"rate_limits": {"used": 3}}))
        );
        assert_eq!(entry.last_attempt_at_ms, 1000);
        assert!(!entry.due(1000 + 299_999));
        assert!(restored.entry("owner", "lynx").data.lock().usage.is_none());
        assert!(restored.entry("other", "mink").data.lock().usage.is_none());
        // Another login generation never sees the stored observation.
        let replaced = restored
            .entry("owner", "mink")
            .data
            .lock()
            .snapshot(&completion(5));
        assert!(replaced.usage.is_none());
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(
                &std::fs::metadata(&path).unwrap().permissions()
            ) & 0o777,
            0o600
        );
        // A damaged file starts empty instead of failing startup.
        std::fs::write(&path, b"{").unwrap();
        assert!(
            Cache::persistent(path)
                .entry("owner", "mink")
                .data
                .lock()
                .usage
                .is_none()
        );
    }
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
        let current = serde_json::json!({"rate_limits":{"remaining":42}});
        entry.finish(&new, Ok(current.clone()));
        assert!(entry.snapshot(&old).usage.is_none());
        assert_eq!(entry.snapshot(&new).usage, Some(current.clone()));
        entry.finish(&old, Ok(usage));
        assert_eq!(entry.usage, Some(current));
        assert!(entry.error.is_none());
    }
}
