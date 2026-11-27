use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{mpsc, watch};

use super::{LaunchSpec, WorkerArgs, antigravity};
use crate::agent_model::TurnUsage;
use crate::logs::now_ms;

pub(super) const COMPACTIONS: usize = 32;

/// Content-free 16-hex-char partial launch signature over worker-known
/// structural inputs before adapter-side tool collision resolution (provider,
/// signed version/generation, system/placement profile, sorted MCP binding
/// names, and presence booleans for Matrix memory / call context / execution
/// descriptor attachments). Never hashes prompts, paths, or credential values.
pub(super) fn compute_launch_prefix_fingerprint(args: &WorkerArgs, spec: &LaunchSpec) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"cowboy.launch_prefix.v2\0");
    hasher.update(args.provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(args.provider_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(args.provider_generation_digest.as_bytes());
    hasher.update(b"\0");
    hasher.update(if args.system {
        b"system\0".as_slice()
    } else {
        b"user\0".as_slice()
    });
    let placement = match args.execution_binding.as_ref() {
        Some(binding) if binding.managed().is_some() => "managed",
        Some(_) => "remote",
        None => "local",
    };
    hasher.update(placement.as_bytes());
    hasher.update(b"\0");
    let effective = |name: &str| -> Option<String> {
        spec.env
            .get(name)
            .cloned()
            .or_else(|| {
                (!spec.removes_inherited_env(name))
                    .then(|| std::env::var(name).ok())
                    .flatten()
            })
            .filter(|v| !v.is_empty())
    };
    let extensions_path = effective("COWBOY_SESSION_EXTENSIONS");
    let (has_memory, ext_has_mcp) = match extensions_path
        .as_deref()
        .and_then(inspect_session_extensions_flags)
    {
        // When the session extensions descriptor is present, it is authoritative
        // and supersedes legacy COWBOY_MATRIX_CONFIG (matching launcher precedence).
        Some((ext_memory, ext_mcp)) => (ext_memory, ext_mcp),
        None => (
            effective("COWBOY_MATRIX_CONFIG").is_some()
                || spec.env.keys().any(|k| k.starts_with("COWBOY_MEMORY_")),
            false,
        ),
    };
    let has_call_ctx = effective("COWBOY_CALL_CONTEXT").is_some();
    let has_exec_desc = effective("COWBOY_EXECUTION_DESCRIPTOR").is_some();
    hasher.update([
        u8::from(has_memory),
        u8::from(ext_has_mcp),
        u8::from(has_call_ctx),
        u8::from(has_exec_desc),
    ]);
    hasher.update(b"\0");
    if let Some(plan) = args.mcp_plan.as_deref() {
        let mut names: Vec<&str> = plan.bindings.iter().map(|b| b.name.as_str()).collect();
        names.sort_unstable();
        for name in names {
            hasher.update(name.as_bytes());
            hasher.update(b"\0");
        }
    }
    let digest = format!("{:x}", hasher.finalize());
    digest[..16].to_owned()
}

fn inspect_session_extensions_flags(path: &str) -> Option<(bool, bool)> {
    use std::io::Read as _;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut buf).ok()?;
    if buf.len() > 64 * 1024 {
        return None;
    }
    let json: Value = serde_json::from_slice(&buf).ok()?;
    let has_memory = json
        .get("memory")
        .or_else(|| json.get("matrix"))
        .is_some_and(|v| !v.is_null());
    let has_mcp = json
        .get("mcp")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()));
    Some((has_memory, has_mcp))
}

/// The worker publishes attribution facts independently of the OTLP parser and
/// log writer. The latest native ID and first prompt survive collector backlog.
#[derive(Clone)]
pub(in crate::worker) struct Observer {
    native_id: watch::Sender<Option<String>>,
    first_prompt: Arc<OnceLock<Option<i64>>>,
    compacted: mpsc::Sender<i64>,
    dropped: Arc<AtomicU64>,
    antigravity: Option<Arc<antigravity::Recorder>>,
}

pub(super) struct Facts {
    pub(super) native_id: watch::Receiver<Option<String>>,
    pub(super) first_prompt: Arc<OnceLock<Option<i64>>>,
    pub(super) compacted: mpsc::Receiver<i64>,
    pub(super) dropped: Arc<AtomicU64>,
}

impl Observer {
    pub(super) fn new(native_id: Option<String>) -> (Self, Facts) {
        let (native_id_tx, native_id_rx) = watch::channel(native_id);
        let (compacted_tx, compacted_rx) = mpsc::channel(COMPACTIONS);
        let first_prompt = Arc::new(OnceLock::new());
        let dropped = Arc::new(AtomicU64::new(0));
        let observer = Self {
            native_id: native_id_tx,
            first_prompt: Arc::clone(&first_prompt),
            compacted: compacted_tx,
            dropped: Arc::clone(&dropped),
            antigravity: None,
        };
        let facts = Facts {
            native_id: native_id_rx,
            first_prompt,
            compacted: compacted_rx,
            dropped,
        };
        (observer, facts)
    }

    pub(super) fn with_antigravity(mut self, recorder: Arc<antigravity::Recorder>) -> Self {
        self.antigravity = Some(recorder);
        self
    }

    pub(in crate::worker) fn native_id(&self, id: &str) {
        self.native_id.send_replace(Some(id.into()));
    }

    pub(in crate::worker) fn prompt(&self, content: &[Value]) {
        self.first_prompt.get_or_init(|| {
            content
                .first()
                .is_some_and(crate::context_rollover::is_renewal_block)
                .then(now_ms)
        });
    }

    pub(in crate::worker) fn compacted(&self, at: i64) {
        if self.compacted.try_send(at).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(in crate::worker) fn prompt_started(&self, at_ms: i64) {
        if let Some(recorder) = &self.antigravity {
            recorder.prompt_started(at_ms);
        }
    }

    pub(in crate::worker) fn observe_config_options(&self, options: &Value) {
        let Some(recorder) = &self.antigravity else {
            return;
        };
        if let Some(model) = options.as_array().and_then(|items| {
            items.iter().find_map(|item| {
                (item.get("id").and_then(Value::as_str) == Some("model"))
                    .then(|| item.get("currentValue").and_then(Value::as_str))
                    .flatten()
            })
        }) {
            recorder.set_model(model);
        }
    }

    pub(in crate::worker) fn record_prompt_observation(
        &self,
        stop_reason: &str,
        usage: Option<TurnUsage>,
        meta: Option<&Map<String, Value>>,
    ) {
        if let Some(recorder) = &self.antigravity {
            recorder.record_turn(stop_reason, usage, meta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_backlog_keeps_native_id_and_renewal_but_counts_compaction_loss() {
        let (observer, mut facts) = Observer::new(Some("original".into()));
        for at in 0..=COMPACTIONS {
            observer.compacted(i64::try_from(at).unwrap());
        }
        observer.native_id("fork");
        observer.prompt(&[serde_json::json!({
            "type": "text",
            "text": "<system-reminder>\nLinked Cowboy sessions: []\nContext renewed: fixture</system-reminder>"
        })]);
        assert_eq!(facts.native_id.borrow().as_deref(), Some("fork"));
        assert!(facts.first_prompt.get().unwrap().is_some());
        assert_eq!(facts.dropped.load(Ordering::Relaxed), 1);
        assert_eq!(facts.compacted.try_recv().unwrap(), 0);
        observer.prompt(&[]);
        assert!(facts.first_prompt.get().unwrap().is_some());
    }
}
