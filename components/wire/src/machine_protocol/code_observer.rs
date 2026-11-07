//! Private Controller/Machine observation protocol. Serialized IDs cannot mint
//! authority: the Controller retains the complete Session/binding/trust scope,
//! and the Machine requires the current installed grant and original root pin.
use serde::{Deserialize, Serialize};

pub const ADAPTER: &str = "code-observer";
pub const PROTOCOL_VERSION: u16 = 42;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub instance: String,
    /// Decimal text preserves u64 positions through JavaScript consumers.
    pub position: String,
}

fn nonce(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Cursor {
    pub fn valid(&self) -> bool {
        nonce(&self.instance)
            && self
                .position
                .parse::<u64>()
                .is_ok_and(|position| position.to_string() == self.position)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Continuation {
    pub id: String,
    pub parent_session_id: String,
    pub grant_id: String,
    pub root: String,
    pub root_incarnation: String,
}

impl std::fmt::Debug for Continuation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CodeObservationContinuation([private])")
    }
}

impl Continuation {
    pub fn valid(&self) -> bool {
        let root = std::path::Path::new(&self.root);
        nonce(&self.id)
            && nonce(&self.root_incarnation)
            && crate::managed_calls::valid_id(&self.parent_session_id)
            && crate::managed_calls::valid_id(&self.grant_id)
            && self.root.len() <= 4096
            && self.root != "/"
            && root.is_absolute()
            && !self.root.contains(['\0', '\\'])
            && root.components().all(|part| {
                matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
            && root.components().collect::<std::path::PathBuf>().to_str()
                == Some(self.root.as_str())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Start {
        continuation: Continuation,
    },
    Read {
        continuation: Continuation,
        cursor: Option<Cursor>,
    },
    Close {
        continuation: Continuation,
    },
}

impl Request {
    pub fn continuation(&self) -> &Continuation {
        match self {
            Self::Start { continuation }
            | Self::Read { continuation, .. }
            | Self::Close { continuation } => continuation,
        }
    }

    pub fn valid(&self) -> bool {
        self.continuation().valid()
            && match self {
                Self::Read {
                    cursor: Some(cursor),
                    ..
                } => cursor.valid(),
                _ => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_messages_are_closed_bounded_and_require_negotiation() {
        let continuation = Continuation {
            id: "a".repeat(32),
            parent_session_id: "session".into(),
            grant_id: "grant".into(),
            root: "/workspace/root".into(),
            root_incarnation: "b".repeat(32),
        };
        assert!(continuation.valid());
        for root in [
            "/",
            "relative",
            "/workspace/../other",
            "/workspace//root",
            "/workspace/./root",
        ] {
            let mut invalid = continuation.clone();
            invalid.root = root.into();
            assert!(!invalid.valid());
        }
        let mut payload = serde_json::to_value(Request::Start { continuation }).unwrap();
        payload["authority"] = serde_json::json!("forged");
        assert!(serde_json::from_value::<Request>(payload).is_err());
        for position in ["", "01", "-1", "18446744073709551616"] {
            assert!(
                !Cursor {
                    instance: "a".repeat(32),
                    position: position.into()
                }
                .valid()
            );
        }
        let request = super::super::MachineCommand::AdapterRequest {
            request_id: "observer".into(),
            adapter: ADAPTER.into(),
            payload: serde_json::json!({}),
            workspace_incarnation: None,
        };
        assert_eq!(request.minimum_protocol(), PROTOCOL_VERSION);
    }
}
