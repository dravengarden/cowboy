//! Controller registry of per-primary-connection secret tokens authenticating
//! the independent HTTPS post-edit inspection endpoint (`POST /api/machine/post-edit-inspect`,
//! Decision 2A / R4).
//!
//! Mirrors `machine_lanes.rs`: tokens are minted on `/api/machine/connect`
//! upgrade, registered ONLY after the Machine's Ed25519 challenge proof is
//! accepted, and automatically revoked by `Registration::drop` when that
//! primary WebSocket connection ends.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::machine_control::ConnectionToken;

#[derive(Clone)]
pub struct PostEditTarget {
    pub connection: ConnectionToken,
    pub machine_id: String,
}

static TARGETS: LazyLock<parking_lot::Mutex<HashMap<String, PostEditTarget>>> =
    LazyLock::new(Default::default);

pub struct Registration(String);

impl Drop for Registration {
    fn drop(&mut self) {
        TARGETS.lock().remove(&self.0);
    }
}

pub fn register(token: String, target: PostEditTarget) -> Registration {
    TARGETS.lock().insert(token.clone(), target);
    Registration(token)
}

#[must_use]
pub fn lookup(token: &str) -> Option<PostEditTarget> {
    TARGETS.lock().get(token).cloned()
}

/// Whether the Machine owning `connection` announced post-edit support on its
/// primary handshake (meaning it enforces Machine-owned `"zed_post_edit"`
/// concurrency slots across WebSocket disconnects/reconnects).
#[must_use]
pub fn supports_connection(connection: &ConnectionToken) -> bool {
    TARGETS
        .lock()
        .values()
        .any(|target| target.connection.same(connection))
}
