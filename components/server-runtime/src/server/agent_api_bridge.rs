//! Runtime bridge from `cowboy-server-runtime` to the top-level Agent API
//! implementation in `cowboy-server`, plus shared widget transcript projection
//! and parent-session read authorization.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use serde_json::{Map, Value, json};

use super::AppState;
use crate::core::{Event, SessionMeta};
use crate::machine_control::ConnectionToken;
use crate::managed_calls::protocol::Grant;
use crate::managed_calls::service::Ledger;
use crate::store::session_widgets::StoredWidget;
use crate::widgets;

pub struct AgentApiProgram {
    pub code: String,
    pub typescript: bool,
    pub module: bool,
    pub timeout_ms: u64,
}

pub trait AgentApiBridge: Send + Sync {
    fn status(&self, state: &Arc<AppState>, meta: &SessionMeta, grant: &Grant) -> Value;

    fn invoke<'a>(
        &'a self,
        state: &'a Arc<AppState>,
        meta: &'a SessionMeta,
        ledger: &'a Ledger,
        action: &'a str,
        arguments: Value,
        operation: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Value> + Send + 'a>>;

    fn read<'a>(
        &'a self,
        state: &'a Arc<AppState>,
        connection: &'a ConnectionToken,
        grant: &'a Grant,
        meta: &'a SessionMeta,
        ledger: &'a Ledger,
        program: AgentApiProgram,
    ) -> Pin<Box<dyn Future<Output = Value> + Send + 'a>>;

    fn recover_decisions<'a>(
        &'a self,
        state: &'a Arc<AppState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Orders each widget's store write before its transcript push, so a later
/// snapshot can never be fanned out before an earlier one.
pub static WIDGET_WRITES: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

pub fn push_widget(
    state: &AppState,
    session_id: &str,
    widget: &StoredWidget,
    operation: &str,
    decided: Map<String, Value>,
) {
    let title = widget.document["title"]
        .as_str()
        .unwrap_or("Widget")
        .to_owned();
    let update = json!({
        "sessionUpdate": "notice",
        "severity": "info",
        "title": title,
        "description": widgets::description(&widget.document),
        "_cowboy": {"widget": {
            "widgetId": widget.widget_id,
            "handle": widget.handle,
            "revision": widget.revision,
            "stateVersion": widget.state_version,
            "publishedAtMs": widget.created_at_ms,
            "updatedAtMs": widget.updated_at_ms,
            "origin": {"kind": "mcp", "operation": operation},
            "closed": widget.closed,
            "decided": decided,
            "document": widget.document,
        }},
    });
    state.hub.push(session_id, Event::Update { update });
}

pub async fn active_preference_owner(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    meta: &SessionMeta,
) -> Result<String, &'static str> {
    let owner = meta.owner_user_id.as_deref().ok_or("permission_denied")?;
    let user = store
        .ok_or("unavailable")?
        .user_by_id(owner)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("permission_denied")?;
    if meta.system
        || user.disabled_at_ms.is_some()
        || hub
            .session_info(&meta.id)
            .is_none_or(|info| info.meta.closing || info.meta.owner_user_id != meta.owner_user_id)
    {
        return Err("permission_denied");
    }
    Ok(owner.to_owned())
}

pub async fn read_parent(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    meta: &SessionMeta,
) -> Result<(), &'static str> {
    if hub.session_info(&meta.id).is_none_or(|info| {
        info.meta.system || info.meta.closing || info.meta.owner_user_id != meta.owner_user_id
    }) {
        return Err("permission_denied");
    }
    if meta.owner_user_id.is_some() {
        active_preference_owner(hub, store, meta).await?;
    }
    Ok(())
}
