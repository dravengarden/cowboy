//! Material comparison of portable credential bundles.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableCredentialBundle {
    pub portable_schema: String,
    pub method_id: String,
    /// Opaque values encoded with standard base64. Bundle keys are declared by
    /// the Provider authentication contract and never appear in logs.
    pub values: std::collections::BTreeMap<String, String>,
}

impl PortableCredentialBundle {
    /// A Provider CLI may rewrite its credential file without refreshing it:
    /// a newer release adds metadata fields and an older one drops fields it
    /// does not know. Treating such a rewrite as a refresh promoted a new
    /// Service generation on every usage poll, redistributed it to every
    /// Machine and let the other CLI version rewrite it back. A refresh must
    /// change a value both sides carry; JSON object fields present on only one
    /// side are metadata when the objects share at least one equal field.
    /// Non-JSON values and bundle keys compare exactly.
    #[must_use]
    pub fn materially_equals(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        if self.portable_schema != other.portable_schema
            || self.method_id != other.method_id
            || !self.values.keys().eq(other.values.keys())
        {
            return false;
        }
        self.values.iter().all(|(key, value)| {
            let theirs = &other.values[key];
            value == theirs || json_values_share_equal_fields(value, theirs)
        })
    }
}

fn json_values_share_equal_fields(left: &str, right: &str) -> bool {
    use base64::Engine as _;
    let decode = |value: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    };
    let (Some(left), Some(right)) = (decode(left), decode(right)) else {
        return false;
    };
    common_fields_equal(&left, &right)
}

fn common_fields_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    match (left, right) {
        // One-sided fields are metadata only beside at least one shared,
        // equal field: `{}` becoming `{"token": …}` is a sign-in.
        (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
            let mut shared = false;
            left.iter().all(|(key, value)| {
                right.get(key).is_none_or(|other| {
                    shared = true;
                    common_fields_equal(value, other)
                })
            }) && (shared || (left.is_empty() && right.is_empty()))
        }
        _ => left == right,
    }
}
