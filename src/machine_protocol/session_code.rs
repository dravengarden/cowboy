//! Private core filesystem observations; never Plugin or native Zed authority.

use serde::{Deserialize, Serialize};

pub const ADAPTER: &str = "session-code";
pub const PROTOCOL_VERSION: u16 = 25;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Observe {
        root: String,
    },
    Verify {
        root: String,
        incarnation: String,
    },
    Read {
        root: String,
        incarnation: String,
        operation: serde_json::Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_session_envelopes_require_the_new_protocol_and_reject_unknown_fields() {
        let command = crate::machine_protocol::MachineCommand::AdapterRequest {
            request_id: "read".into(),
            adapter: ADAPTER.into(),
            payload: serde_json::json!({"action":"observe", "root":"/workspace"}),
            workspace_incarnation: None,
        };
        assert_eq!(command.minimum_protocol(), PROTOCOL_VERSION);
        for value in [
            serde_json::json!({"action":"observe", "root":"/workspace", "incarnation":"forged"}),
            serde_json::json!({"action":"verify", "root":"/workspace"}),
            serde_json::json!({"action":"restore", "root":"/workspace"}),
            serde_json::json!({"action":"read", "root":"/workspace", "incarnation":"x", "operation":{}, "adapter":"zed"}),
        ] {
            assert!(serde_json::from_value::<Request>(value).is_err());
        }
    }
}
