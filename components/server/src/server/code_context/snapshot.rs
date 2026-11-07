//! Identity of the mirrored text, not evidence that it matches disk or inputs.
use serde::{Deserialize, Serialize};

use super::super::CodeBufferVersion;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::server) enum SourceState {
    Matching,
    Changed,
    #[default]
    Unavailable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::server) struct BufferSnapshot {
    instance: String,
    buffer: String,
    buffer_epoch: String,
    version: Vec<CodeBufferVersion>,
    content: Content,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Content {
    sha256: String,
    utf8_bytes: u32,
}

impl BufferSnapshot {
    pub fn bounded(&self) -> bool {
        let hex = |value: &str, length| {
            value.len() == length
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        let identity = |value: &str| value.parse::<u64>().is_ok_and(|id| id.to_string() == value);
        hex(&self.instance, 32)
            && identity(&self.buffer)
            && identity(&self.buffer_epoch)
            && self.version.len() <= 64
            && self
                .version
                .windows(2)
                .all(|pair| pair[0].replica_id < pair[1].replica_id)
            && self.content.utf8_bytes <= 4 * 1024 * 1024
            && hex(&self.content.sha256, 64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_projection_keeps_exact_identity_or_rejects_it() {
        let value = json!({"instance":"a".repeat(32),"buffer":u64::MAX.to_string(),
            "bufferEpoch":"1","version":[{"replicaId":1,"timestamp":2}],
            "content":{"sha256":"b".repeat(64),"utf8Bytes":0}});
        let snapshot: BufferSnapshot = serde_json::from_value(value.clone()).unwrap();
        assert!(snapshot.bounded());
        assert_eq!(serde_json::to_value(snapshot).unwrap(), value);
        for (pointer, replacement) in [
            ("/buffer", json!("+1")),
            ("/bufferEpoch", json!("01")),
            ("/content/sha256", json!("B".repeat(64))),
            ("/content/utf8Bytes", json!(4 * 1024 * 1024 + 1)),
            (
                "/version",
                json!([{"replicaId":1,"timestamp":2},{"replicaId":1,"timestamp":3}]),
            ),
            (
                "/version",
                json!(
                    (0..65)
                        .map(|id| json!({"replicaId":id,"timestamp":1}))
                        .collect::<Vec<_>>()
                ),
            ),
        ] {
            let mut invalid = value.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                !serde_json::from_value::<BufferSnapshot>(invalid)
                    .unwrap()
                    .bounded()
            );
        }
    }

    #[test]
    fn language_wire_decodes_new_snapshot_without_inventing_one_for_old_engines() {
        use super::super::ZedAdapterResponse;
        let mut reply = json!({"type":"bufferLanguage","api_version":1,"path":"file.rs",
            "version":[],"diagnostics":[],"inlay_hints":[],"semantic_tokens":[]});
        let ZedAdapterResponse::BufferLanguage {
            buffer_snapshot, ..
        } = serde_json::from_value(reply.clone()).unwrap()
        else {
            panic!("wrong response")
        };
        assert!(buffer_snapshot.is_none());
        reply["bufferSnapshot"] = json!({"instance":"a".repeat(32),"buffer":"7",
            "bufferEpoch":"1","version":[],"content":{"sha256":"b".repeat(64),"utf8Bytes":0}});
        let ZedAdapterResponse::BufferLanguage {
            buffer_snapshot, ..
        } = serde_json::from_value(reply).unwrap()
        else {
            panic!("wrong response")
        };
        assert!(buffer_snapshot.unwrap().bounded());
    }
}
