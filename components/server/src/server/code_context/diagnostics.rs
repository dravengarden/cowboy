use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::server) struct CodePoint {
    pub row: u32,
    pub column: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::server) struct CodeDiagnostic {
    pub start: CodePoint,
    pub end: CodePoint,
    pub severity: i32,
    pub source: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::server) struct Provenance {
    instance: String,
    buffer: String,
    buffer_epoch: String,
    server: String,
    source_kind: i32,
    registration: Option<String>,
}

impl Provenance {
    pub fn bounded(&self) -> bool {
        let identity = |value: &str| value.parse::<u64>().is_ok_and(|id| id.to_string() == value);
        self.instance.len() == 32
            && self
                .instance
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && identity(&self.buffer)
            && identity(&self.buffer_epoch)
            && identity(&self.server)
            && (0..=2).contains(&self.source_kind)
            && self
                .registration
                .as_ref()
                .is_none_or(|value| value.len() <= 256)
    }

    /// Identity equality is not analysis freshness or a text-version mapping.
    pub fn belongs_to(&self, snapshot: &super::snapshot::BufferSnapshot) -> bool {
        self.bounded() && snapshot.identifies(&self.instance, &self.buffer, &self.buffer_epoch)
    }
}
