use serde_json::Value;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{mpsc, watch};

use crate::logs::now_ms;

pub(super) const COMPACTIONS: usize = 32;

/// The worker publishes attribution facts independently of the OTLP parser and
/// log writer. The latest native ID and first prompt survive collector backlog.
#[derive(Clone)]
pub(in crate::worker) struct Observer {
    native_id: watch::Sender<Option<String>>,
    first_prompt: Arc<OnceLock<Option<i64>>>,
    compacted: mpsc::Sender<i64>,
    dropped: Arc<AtomicU64>,
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
        };
        let facts = Facts {
            native_id: native_id_rx,
            first_prompt,
            compacted: compacted_rx,
            dropped,
        };
        (observer, facts)
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
