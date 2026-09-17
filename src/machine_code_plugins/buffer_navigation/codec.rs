//! Strict private observations. An unexpected reply never resets an effect.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeRef {
    pub instance: String,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum NativeState {
    Prepared {},
    Unknown {},
    Retained { locations: Vec<Location> },
    ReleaseUnknown {},
    Released {},
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    #[serde(rename = "type")]
    kind: ReplyKind,
    api_version: u8,
    pub navigation: NativeRef,
    pub state: NativeState,
}

#[derive(Deserialize)]
enum ReplyKind {
    #[serde(rename = "ownedBufferNavigation")]
    Navigation,
}

pub(super) fn reply(value: &Value) -> Result<Reply> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= 2 * 1024 * 1024, "navigation reply too large");
    let reply: Reply = serde_json::from_slice(&bytes)?;
    let ReplyKind::Navigation = reply.kind;
    ensure!(reply.api_version == 1, "unsupported navigation observation");
    BufferRef {
        instance: reply.navigation.instance.clone(),
        id: reply
            .navigation
            .id
            .strip_prefix("nav:")
            .unwrap_or("")
            .to_owned(),
    }
    .validate()?;
    if let NativeState::Retained { locations } = &reply.state {
        crate::machine_protocol::code_buffer_navigation::validate_locations(locations)?;
    }
    Ok(reply)
}
