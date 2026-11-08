//! `/api/machine/browser-stream`: the Machine's half of one display stream.
//! A Machine route like the bulk lane: authenticated by the one-time stream
//! token the Controller minted for exactly one attachment plus the current
//! Machine connection, never by an owner credential.

use super::super::super::*;
use crate::machine_protocol::machine_browser::STREAM_TOKEN_HEADER;

pub(in crate::server) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = headers
        .get(STREAM_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(pairing) = state.browsers.take_pairing(token) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !state.machine_control.is_current(&pairing.connection) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| async move {
        // The viewer task owns both halves from here; a viewer that already
        // gave up closes this socket by dropping it.
        let _ = pairing.deliver.send(socket);
    })
}
