//! The login registry shared by the Controller connection and the Machine
//! browser: every native or Service login a Machine may still cancel, feed a
//! code, or bind a browser to. `machine_cli` owns the entries' lifetime;
//! `machine_browser` only reads and annotates them under the same lock.

use std::sync::Arc;

pub(crate) struct LoginSession {
    pub(crate) cancel: tokio::sync::watch::Sender<bool>,
    pub(crate) input: tokio::sync::mpsc::Sender<String>,
    pub(crate) candidate: Option<crate::machine_plugins::AuthCandidateCleanup>,
    /// The named account of a native login; `None` for Service-owned logins,
    /// which no Machine browser may bind to.
    pub(crate) account: Option<crate::provider_account::ProviderAccountId>,
    pub(crate) provider: String,
    pub(crate) expires_at_ms: i64,
    /// The verification URL this Machine emitted for the login, once known.
    pub(crate) verification_url: Option<String>,
    /// The Machine browser instance and incarnation bound to this login.
    pub(crate) browser: Option<(String, u64)>,
}

pub(crate) type LoginSessions =
    Arc<parking_lot::Mutex<std::collections::HashMap<String, LoginSession>>>;
