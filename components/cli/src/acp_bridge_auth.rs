//! Authentication and WebSocket URL helpers for the ACP stdio bridge.

use anyhow::anyhow;
use reqwest::Url;
use tokio_tungstenite::tungstenite::Error as WsError;

pub(super) fn approval_required_message(
    pending: &crate::client_auth_client::PendingApproval,
) -> String {
    format!(
        "Cowboy is waiting for you to approve this computer. Open {} in a browser, check that \
         the client fingerprint is {}, approve it, then try again.",
        pending.url, pending.fingerprint
    )
}

pub(super) fn is_fatal_auth_status(status: u16) -> bool {
    status == 401 || status == 403
}

pub(super) fn websocket_handshake_status(error: &WsError) -> Option<u16> {
    match error {
        WsError::Http(response) => Some(response.status().as_u16()),
        _ => None,
    }
}

pub(super) fn websocket_auth_rejected(error: &WsError) -> bool {
    websocket_handshake_status(error).is_some_and(is_fatal_auth_status)
}

pub(super) fn daemon_auth_rejection_message(context: &str) -> String {
    format!("{context}. Run `cowboy login <Cowboy URL>` and try again.")
}

pub(super) fn daemon_auth_rejected(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<WsError>()
            .is_some_and(websocket_auth_rejected)
            || cause.to_string().contains("rejected this computer login")
            // A person refused this computer. Retrying would file a new
            // approval request every few seconds for them to refuse again.
            || cause.to_string().contains("device authorization was denied")
    })
}

pub(super) fn websocket_url(base: &Url) -> anyhow::Result<Url> {
    let mut url = base.join("ws")?;
    let scheme = if base.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    url.set_scheme(scheme)
        .map_err(|()| anyhow!("cannot convert daemon URL to WebSocket URL"))?;
    // Only the global session index: with the complete bootstrap every
    // connection downloaded every session's transcript tail and had to finish
    // within BOOTSTRAP_TIMEOUT. One session is hydrated when it is loaded.
    url.query_pairs_mut().append_pair("bootstrap", "lazy");
    Ok(url)
}
