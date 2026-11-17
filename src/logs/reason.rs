//! Closed failure reasons for local evidence.
//!
//! The local store never keeps a warning's free text: error messages can carry
//! upstream responses, paths or agent output. A failure is still diagnosable
//! by its `reason` field, recorded as OTel `error.type`, whose values come only
//! from this module's fixed vocabulary. Typed errors (I/O, timeouts, WebSocket)
//! classify by type; Cowboy's own startup errors classify by their fixed
//! wording. Anything else is `unclassified`, never the text itself.

/// The reason for an error, walking its source chain.
pub fn failure_reason(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        if let Some(reason) = typed_reason(cause) {
            return reason;
        }
    }
    text_reason(&format!("{error:#}"))
}

/// The reason for any standard error and its sources.
pub fn error_reason(error: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut cause = Some(error);
    while let Some(current) = cause {
        if let Some(reason) = typed_reason(current) {
            return reason;
        }
        cause = current.source();
    }
    text_reason(&error.to_string())
}

fn typed_reason(error: &(dyn std::error::Error + 'static)) -> Option<&'static str> {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return Some(io_reason(error.kind()));
    }
    if error.is::<tokio::time::error::Elapsed>() {
        return Some("timeout");
    }
    if let Some(error) = error.downcast_ref::<tokio_tungstenite::tungstenite::Error>() {
        use tokio_tungstenite::tungstenite::Error;
        return match error {
            Error::Io(error) => Some(io_reason(error.kind())),
            Error::ConnectionClosed | Error::AlreadyClosed => Some("websocket_closed"),
            Error::Protocol(_) => Some("websocket_protocol"),
            Error::Tls(_) => Some("tls"),
            Error::Http(_) | Error::HttpFormat(_) | Error::Url(_) => Some("websocket_handshake"),
            _ => None,
        };
    }
    None
}

fn io_reason(kind: std::io::ErrorKind) -> &'static str {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::NotFound => "io_not_found",
        ErrorKind::PermissionDenied => "io_permission_denied",
        ErrorKind::ConnectionRefused => "connection_refused",
        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => "connection_reset",
        ErrorKind::BrokenPipe => "broken_pipe",
        ErrorKind::TimedOut => "timeout",
        ErrorKind::UnexpectedEof => "unexpected_eof",
        ErrorKind::StorageFull => "storage_full",
        ErrorKind::OutOfMemory => "out_of_memory",
        _ => "io_other",
    }
}

/// Cowboy's own fixed error wording, most specific first. Only the matched
/// reason is recorded.
const PHRASES: &[(&str, &str)] = &[
    ("did not complete ACP session/resume", "acp_resume_timeout"),
    ("did not complete ACP session/load", "acp_load_timeout"),
    ("did not complete ACP session/new", "acp_new_timeout"),
    ("did not complete ACP initialize", "acp_initialize_timeout"),
    ("did not complete ACP", "acp_startup_timeout"),
    (
        crate::generation_retention::NOT_INSTALLED_MARKER,
        "generation_not_installed",
    ),
    ("admission denied", "admission_denied"),
    ("Plugin authentication probe", "auth_probe_failed"),
    ("liveness probe", "liveness_timeout"),
    ("before readiness", "worker_exited_before_ready"),
    ("close_notify", "tls_unclean_close"),
    ("without closing handshake", "connection_reset"),
    ("Connection reset", "connection_reset"),
    ("Connection refused", "connection_refused"),
    ("timed out", "timeout"),
];

/// The reason for a Cowboy-worded error message.
pub fn text_reason(text: &str) -> &'static str {
    PHRASES
        .iter()
        .find(|(phrase, _)| text.contains(phrase))
        .map_or("unclassified", |(_, reason)| reason)
}

/// Agent stderr that is not routine: its local evidence keeps the closed
/// reason `agent_stderr`, never the line; journald keeps the line.
pub fn agent_stderr(session: &dyn std::fmt::Display, provider: &dyn std::fmt::Display, line: &str) {
    tracing::warn!(
        event_name = "cowboy.agent.stderr",
        session = %session,
        provider = %provider,
        child = true,
        reason = "agent_stderr",
        message = %line,
        "agent stderr"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_are_closed_and_never_echo_text() {
        let resume = anyhow::anyhow!(
            "worker sess-1 entered Some(Crashed) before readiness: agent did not complete ACP session/resume within 240s"
        );
        assert_eq!(failure_reason(&resume), "acp_resume_timeout");
        let refused =
            anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
                .context("connecting Machine WebSocket");
        assert_eq!(failure_reason(&refused), "connection_refused");
        assert_eq!(
            text_reason("IO error: peer closed connection without sending TLS close_notify"),
            "tls_unclean_close"
        );
        assert_eq!(text_reason("secret-body token=abc"), "unclassified");
        for (_, reason) in PHRASES {
            assert!(reason.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
        }
    }
}
