//! Standing observation is bound to the original worker, connection, Session
//! and policy. It cannot authorize acquisition, synchronization or delivery.
use super::*;
use crate::core::LanguageServerTrust;
use crate::machine_control::SessionReadScope;
use crate::machine_protocol::{code_observer, session_code};
use std::sync::atomic::{AtomicBool, Ordering};

const CAPACITY: usize = 256;
const IDLE: Duration = Duration::from_secs(300);
const REQUEST_WAIT: Duration = Duration::from_millis(500);

struct Context<'a> {
    hub: &'a crate::core::Hub,
    machine_control: &'a crate::machine_control::MachineControl,
    runtime_router: &'a crate::runtime_router::RuntimeRouter,
    service_id: &'a str,
    managed_calls: &'a Coordinator,
}

impl<'a> From<&'a AppState> for Context<'a> {
    fn from(state: &'a AppState) -> Self {
        Self {
            hub: &state.hub,
            machine_control: &state.machine_control,
            runtime_router: &state.runtime_router,
            service_id: &state.service_id,
            managed_calls: &state.managed_calls,
        }
    }
}

impl Context<'_> {
    fn authority(&self, parent: &str) -> Option<Authority> {
        current_authority_in(self.hub, self.runtime_router, self.service_id, parent)
            .map(|(_, authority)| authority)
    }
}

#[derive(Default)]
pub(super) struct Observations(Mutex<HashMap<String, Arc<Entry>>>);

struct Entry {
    id: String,
    scope: SessionReadScope,
    trust: LanguageServerTrust,
    grant: Grant,
    authority: Authority,
    last_used: Mutex<Instant>,
    last_cleanup: Mutex<Option<Instant>>,
    retiring: AtomicBool,
    // Recorded before Start dispatch. Cancellation or a lost response keeps
    // the exact request available for retry and for Close-before-Start fencing.
    operation: tokio::sync::Mutex<Option<code_observer::Continuation>>,
}

impl Entry {
    fn current(&self, state: &Context<'_>) -> bool {
        if self.retiring.load(Ordering::Acquire)
            || !self.grant.agent_api
            || !self.trust.current(state.hub)
            || !state.hub.code_scope_is_current(self.scope.session())
            || !state
                .machine_control
                .session_read_scope_is_current(&self.scope)
        {
            return false;
        }
        let Some(connection) = self.scope.connection() else {
            return false;
        };
        if self.authority.placement().machine_id != connection.machine_id() {
            return false;
        }
        let installed = state
            .managed_calls
            .grants
            .lock()
            .get(&self.grant.parent_session_id)
            .is_some_and(|entry| {
                entry.grant == self.grant
                    && entry
                        .installed_on
                        .as_ref()
                        .is_some_and(|token| token.same(connection))
                    && entry.authority.accepts(&self.authority)
            });
        installed
            && state
                .authority(&self.grant.parent_session_id)
                .is_some_and(|current| self.authority.accepts(&current))
    }

    async fn start(&self, state: &Context<'_>) -> Result<(), String> {
        let mut operation = self.operation.lock().await;
        if !self.current(state) {
            return Err("observation ended".into());
        }
        let connection = self.scope.connection().ok_or("observation unavailable")?;
        if operation.is_none() {
            let reply = state
                .machine_control
                .adapter_request_on_connection(
                    connection,
                    session_code::ADAPTER,
                    json!({"action":"observe", "root":self.scope.session().cwd()}),
                )
                .await?;
            let incarnation = reply
                .get("incarnation")
                .and_then(Value::as_str)
                .ok_or("invalid root observation")?;
            let continuation = code_observer::Continuation {
                id: self.id.clone(),
                parent_session_id: self.grant.parent_session_id.clone(),
                grant_id: self.grant.grant_id.clone(),
                root: self.scope.session().cwd().into(),
                root_incarnation: incarnation.into(),
            };
            if !continuation.valid() {
                return Err("invalid root observation".into());
            }
            *operation = Some(continuation);
        }
        if !self.current(state) {
            return Err("observation ended".into());
        }
        let original = operation.as_ref().expect("recorded continuation");
        let verified = state.machine_control.adapter_request_on_connection(connection,
            session_code::ADAPTER, json!({"action":"verify", "root":original.root, "incarnation":original.root_incarnation})).await?;
        if verified == json!({"current":false}) {
            self.retiring.store(true, Ordering::Release);
            return Err("observation root ended".into());
        }
        if verified != json!({"current":true}) || !self.current(state) {
            return Err("observation ended".into());
        }
        let request = code_observer::Request::Start {
            continuation: operation.as_ref().expect("recorded continuation").clone(),
        };
        let reply = request_on(state, connection, request).await?;
        if !self.current(state) {
            return Err("observation ended".into());
        }
        if reply != json!({"state":"started"}) {
            return Err("invalid observer acknowledgement".into());
        }
        Ok(())
    }
}

async fn request_on(
    state: &Context<'_>,
    connection: &ConnectionToken,
    request: code_observer::Request,
) -> Result<Value, String> {
    state
        .machine_control
        .adapter_request_on_connection(
            connection,
            code_observer::ADAPTER,
            serde_json::to_value(request).map_err(|_| "observer encoding failed")?,
        )
        .await
}

fn reserve(state: &Context<'_>, scope: &SessionReadScope) -> Option<Arc<Entry>> {
    let connection = scope.connection()?;
    let (current, protocol) = state
        .machine_control
        .connection_protocol(connection.machine_id())?;
    if !current.same(connection) || protocol < code_observer::PROTOCOL_VERSION {
        return None;
    }
    let trust = state
        .hub
        .observe_language_server_trust(scope.session().session_id())?;
    if trust.scope() != scope.session() {
        return None;
    }
    let (grant, authority) = {
        let grants = state.managed_calls.grants.lock();
        let issued = grants.get(scope.session().session_id())?;
        (issued.grant.clone(), issued.authority.clone())
    };
    let mut entries = state.managed_calls.observations.0.lock();
    if let Some(entry) = entries
        .values()
        .find(|entry| entry.scope == *scope && entry.current(state))
    {
        *entry.last_used.lock() = Instant::now();
        return Some(Arc::clone(entry));
    }
    // An unresolved old observer for this parent owns its slot until cleanup;
    // a fresh ID cannot turn an uncertain Start into a duplicate instance.
    let mut previous = false;
    for entry in entries
        .values()
        .filter(|entry| entry.grant.parent_session_id == scope.session().session_id())
    {
        entry.retiring.store(true, Ordering::Release);
        previous = true;
    }
    if previous {
        return None;
    }
    if entries.len() >= CAPACITY {
        return None;
    }
    let mut random = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
    let id = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let entry = Arc::new(Entry {
        id: id.clone(),
        scope: scope.clone(),
        trust,
        grant,
        authority,
        last_used: Mutex::new(Instant::now()),
        last_cleanup: Mutex::new(None),
        retiring: AtomicBool::new(false),
        operation: tokio::sync::Mutex::new(None),
    });
    if !entry.current(state) {
        return None;
    }
    entries.insert(id, Arc::clone(&entry));
    Some(entry)
}

/// Actual diagnostic demand only; passive summaries do not prolong observation.
/// Observation failure is optional evidence, never a failure of the native turn.
pub(in crate::server) async fn touch(state: &AppState, scope: &SessionReadScope) {
    let context = Context::from(state);
    if let Some(entry) = reserve(&context, scope) {
        let _ = tokio::time::timeout(REQUEST_WAIT, entry.start(&context)).await;
    }
}

async fn retire(state: &Context<'_>, entry: &Entry) -> bool {
    let Some(connection) = entry.scope.connection() else {
        return true;
    };
    // Original transport loss cancels the Machine grant. Never redirect Close
    // to its replacement, and do not call this confirmed native thread exit.
    if !state.machine_control.is_current(connection) {
        return true;
    }
    let operation = entry.operation.lock().await;
    let Some(continuation) = operation.as_ref() else {
        return true;
    };
    request_on(
        state,
        connection,
        code_observer::Request::Close {
            continuation: continuation.clone(),
        },
    )
    .await
    .is_ok_and(|reply| reply == json!({"state":"closed"}))
}

async fn maintain(state: &Context<'_>) {
    let mut entries: Vec<_> = state
        .managed_calls
        .observations
        .0
        .lock()
        .values()
        .cloned()
        .collect();
    // Mark every ended scope before bounded cleanup. A slow Close cannot keep
    // another policy's old continuation available to new reads.
    for entry in &entries {
        if !entry.current(state) || entry.last_used.lock().elapsed() >= IDLE {
            entry.retiring.store(true, Ordering::Release);
        }
    }
    entries.sort_by_key(|entry| *entry.last_cleanup.lock());
    for entry in entries
        .into_iter()
        .filter(|entry| entry.retiring.load(Ordering::Acquire))
        .take(32)
    {
        *entry.last_cleanup.lock() = Some(Instant::now());
        if matches!(
            tokio::time::timeout(REQUEST_WAIT, retire(state, &entry)).await,
            Ok(true)
        ) {
            state.managed_calls.observations.0.lock().remove(&entry.id);
        }
    }
}

pub(super) fn spawn(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(RECONCILE_INTERVAL).await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            if *state.shutdown.borrow() {
                break;
            }
            maintain(&Context::from(state.as_ref())).await;
        }
    });
}

#[cfg(test)]
mod tests;
