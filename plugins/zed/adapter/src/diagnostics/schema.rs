use serde::Serialize;
use serde::ser::{SerializeSeq as _, Serializer};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LanguagePoint {
    pub(crate) row: u32,
    pub(crate) column: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LanguageDiagnostic {
    pub(crate) start: LanguagePoint,
    pub(crate) end: LanguagePoint,
    pub(crate) severity: i32,
    pub(crate) source: Option<String>,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provenance: Option<Provenance>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Provenance {
    /// The original native event stream's cache lifetime, never a process PID.
    pub(crate) instance: String,
    /// Decimal strings preserve the native u64 identities through JavaScript.
    pub(crate) buffer: String,
    pub(crate) buffer_epoch: String,
    pub(crate) server: String,
    pub(crate) source_kind: i32,
    pub(crate) registration: Option<String>,
}

impl LanguageDiagnostic {
    /// Owned read protocol one has a closed diagnostic schema. Keep its
    /// existing wire contract until an independently negotiated version exists.
    pub(crate) fn legacy(mut self) -> Self {
        self.code = None;
        self.provenance = None;
        self
    }
}

pub(crate) fn serialize_legacy<S: Serializer>(
    values: &[LanguageDiagnostic],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Legacy<'a> {
        start: &'a LanguagePoint,
        end: &'a LanguagePoint,
        severity: i32,
        source: &'a Option<String>,
        message: &'a str,
    }
    let mut sequence = serializer.serialize_seq(Some(values.len()))?;
    for value in values {
        sequence.serialize_element(&Legacy {
            start: &value.start,
            end: &value.end,
            severity: value.severity,
            source: &value.source,
            message: &value.message,
        })?;
    }
    sequence.end()
}
