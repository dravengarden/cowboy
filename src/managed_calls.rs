//! Closed input contract for managed cross-Provider calls. A validated request
//! is caller input, never a parent identity or permission to execute.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[cfg(unix)]
pub mod cli;
#[cfg(unix)]
pub mod gateway;
#[cfg(all(unix, feature = "machine-host"))]
pub mod host;
#[cfg(unix)]
pub mod round;
#[cfg(all(unix, feature = "machine-host"))]
pub mod snapshot;

#[cfg(feature = "full")]
pub mod authority;
pub mod lifecycle;
pub mod protocol;
#[cfg(feature = "full")]
pub mod service;

pub const MAX_REQUEST_BYTES: usize = 256 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u16,
    pub request_id: String,
    pub purpose: Purpose,
    pub instruction: String,
    pub context: Context,
    pub access: Access,
    pub conversation: Conversation,
    /// A native turn constraint, never instructions appended to the prompt.
    #[serde(default, skip_serializing_if = "OutputFormat::is_text")]
    pub output: OutputFormat,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Review,
    Analysis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    ReadOnly,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub scope: ContextScope,
    #[serde(default)]
    pub files: Vec<String>,
    /// Absolute top level of the Git work tree to snapshot on the execution
    /// target. Omitted means the caller's current worktree. The target only
    /// accepts a work tree of a repository registered on that Machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// A lexical absolute path without `.`/`..`, empty or control components.
pub(crate) fn absolute_root(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
        && (value == "/"
            || value[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextScope {
    CurrentWorktree,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Conversation {
    Fresh {},
    Continue { child_session_id: String },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputFormat {
    Text {},
    JsonSchema { schema: serde_json::Value },
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Text {}
    }
}

impl OutputFormat {
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text {})
    }

    pub(crate) fn validate(&self) -> bool {
        match self {
            Self::Text {} => true,
            Self::JsonSchema { schema } => {
                schema.is_object()
                    && serde_json::to_vec(schema).is_ok_and(|bytes| bytes.len() <= 64 * 1024)
                    && bounded_schema(schema, 0)
            }
        }
    }
}

/// Bound the opaque native schema without implementing another schema engine.
/// Native adapters own dialect support/validation and must explicitly accept it
/// before launch. Remote references and schema resource rebasing are refused.
fn bounded_schema(value: &serde_json::Value, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match value {
        serde_json::Value::Object(fields) => fields.iter().all(|(key, value)| {
            !(key == "$id" && value.is_string())
                && (!matches!(key.as_str(), "$ref" | "$dynamicRef" | "$recursiveRef")
                    || value
                        .as_str()
                        .is_none_or(|reference| reference.starts_with('#')))
                && bounded_schema(value, depth + 1)
        }),
        serde_json::Value::Array(values) => {
            values.iter().all(|value| bounded_schema(value, depth + 1))
        }
        _ => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputError {
    RequestTooLarge,
    InvalidJson,
    InvalidContract,
    UnsupportedSchema,
    InvalidRequestId,
    InvalidInstruction,
    InvalidContext,
    InvalidConversation,
    InvalidLabels,
    InvalidOutput,
}

/// No input-bearing Debug implementation: prompts and paths can contain
/// private source. Even error messages must stay independent of input text.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedCallRequest([private])")
    }
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = serde_json::to_value(self).expect("closed input error");
        f.write_str(code.as_str().expect("error is a string"))
    }
}
impl std::error::Error for InputError {}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub(crate) fn relative_file(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value.contains(['\\', '\0', ':'])
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
}

pub fn valid_labels(labels: &BTreeMap<String, String>) -> bool {
    labels.len() <= 8
        && labels.iter().all(|(key, value)| {
            valid_id(key)
                && key.len() <= 32
                && value.len() <= 256
                && !value.chars().any(char::is_control)
        })
}

impl Request {
    /// Parse without accepting duplicate fields or leaking source in errors.
    /// File containment still requires a target-owned snapshot; lexical checking
    /// alone cannot authorize opening a symlink.
    pub fn parse(bytes: &[u8]) -> Result<Self, InputError> {
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(InputError::RequestTooLarge);
        }
        // Deserialize directly, rather than via Value, so serde rejects duplicate
        // struct fields instead of silently choosing the final occurrence.
        let request: Self = serde_json::from_slice(bytes).map_err(|error| {
            if error.is_syntax() || error.is_eof() {
                InputError::InvalidJson
            } else {
                InputError::InvalidContract
            }
        })?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), InputError> {
        if self.schema != 1 {
            return Err(InputError::UnsupportedSchema);
        }
        if !valid_id(&self.request_id) {
            return Err(InputError::InvalidRequestId);
        }
        if self.instruction.trim().is_empty()
            || self.instruction.len() > 128 * 1024
            || self.instruction.contains('\0')
        {
            return Err(InputError::InvalidInstruction);
        }
        if self.context.files.len() > 32
            || self
                .context
                .root
                .as_deref()
                .is_some_and(|root| !absolute_root(root) || root == "/")
            || self.context.files.iter().any(|file| !relative_file(file))
            || self
                .context
                .files
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.context.files.len()
        {
            return Err(InputError::InvalidContext);
        }
        if let Conversation::Continue { child_session_id } = &self.conversation
            && !valid_id(child_session_id)
        {
            return Err(InputError::InvalidConversation);
        }
        if !valid_labels(&self.labels) {
            return Err(InputError::InvalidLabels);
        }
        if !self.output.validate() {
            return Err(InputError::InvalidOutput);
        }
        Ok(())
    }

    /// Canonical semantic digest. This covers request bytes only; admission must
    /// additionally bind the Provider, parent scope and captured file contents.
    pub fn digest(&self) -> String {
        let value = serde_json::to_value(self).expect("closed request serializes");
        canonical_digest(&value)
    }
}

/// Key-order independent SHA-256 of a JSON value. `serde_json` is built with
/// `preserve_order`, and PostgreSQL JSONB rewrites object key order, so raw
/// serialization is not a stable identity across a store round trip.
pub(crate) fn canonical_digest(value: &serde_json::Value) -> String {
    fn sorted(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(fields) => {
                let mut keys: Vec<_> = fields.keys().collect();
                keys.sort();
                serde_json::Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), sorted(&fields[key])))
                        .collect(),
                )
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(sorted).collect())
            }
            other => other.clone(),
        }
    }
    let bytes = serde_json::to_vec(&sorted(value)).expect("JSON value serializes");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn example() -> Value {
        json!({"schema":1,"request_id":"review-r2-security-01","purpose":"review",
            "instruction":"Review this round.","context":{"scope":"current-worktree","files":["review/input.md"]},
            "access":"read-only","conversation":{"mode":"fresh"}})
    }

    fn parse(value: &Value) -> Result<Request, InputError> {
        Request::parse(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn native_output_schema_is_preserved_and_bound_to_request_identity() {
        let mut value = example();
        let text = parse(&value).unwrap();
        value["output"] = json!({"format":"text"});
        assert_eq!(parse(&value).unwrap().digest(), text.digest());
        value["output"]["ignored"] = json!(true);
        assert!(parse(&value).is_err());
        let schema = json!({"type":"object","additionalProperties":false,
            "properties":{"verdict":{"type":"string","enum":["approve","needs-attention"]}},
            "required":["verdict"]});
        value["output"] = json!({"format":"json_schema","schema":schema});
        let structured = parse(&value).unwrap();
        assert_eq!(structured.instruction, text.instruction);
        assert_ne!(structured.digest(), text.digest());
        assert_eq!(
            serde_json::to_value(&structured).unwrap()["output"]["schema"],
            schema
        );
        value["output"]["schema"]["properties"]["verdict"]["enum"] = json!(["approve"]);
        assert_ne!(parse(&value).unwrap().digest(), structured.digest());
        for schema in [
            json!({"$ref":"https://example.invalid/schema"}),
            json!({"$ref":"file:///private/schema"}),
            json!({"$dynamicRef":"https://example.invalid/schema"}),
            json!({"$id":"https://example.invalid/root","$ref":"#/$defs/result"}),
            json!({"description":"x".repeat(65536)}),
            json!(true),
        ] {
            value["output"]["schema"] = schema;
            assert_eq!(parse(&value).err(), Some(InputError::InvalidOutput));
        }
        let mut nested = json!({"type":"string"});
        for _ in 0..34 {
            nested = json!({"items":nested});
        }
        value["output"]["schema"] = nested;
        assert_eq!(parse(&value).err(), Some(InputError::InvalidOutput));
    }

    #[test]
    fn requests_cannot_select_their_parent_or_escalate_access() {
        for field in [
            "parent_session_id",
            "machine_id",
            "capability",
            "auth",
            "command",
        ] {
            let mut value = example();
            value[field] = json!("spoof");
            assert_eq!(parse(&value).unwrap_err(), InputError::InvalidContract);
        }
        let mut value = example();
        value["access"] = json!("full-access");
        assert_eq!(parse(&value).unwrap_err(), InputError::InvalidContract);
    }

    #[test]
    fn context_rejects_traversal_absolute_paths_and_ambiguous_names() {
        for path in [
            "../secret",
            "a/../../secret",
            "/tmp/secret",
            "a\\secret",
            "file:///secret",
            "a//b",
            "./input",
            "a/./b",
            "x\ny",
        ] {
            let mut value = example();
            value["context"]["files"] = json!([path]);
            assert_eq!(
                parse(&value).unwrap_err(),
                InputError::InvalidContext,
                "{path}"
            );
        }
    }

    #[test]
    fn context_root_is_an_exact_absolute_target_path() {
        let mut value = example();
        value["context"]["root"] = json!("/srv/worktrees/marketplace-service/task");
        assert!(parse(&value).is_ok());
        for root in [
            "relative/path",
            "/",
            "/a/../b",
            "/a/./b",
            "/a//b",
            "/a/b/",
            "/a\nb",
        ] {
            value["context"]["root"] = json!(root);
            assert_eq!(
                parse(&value).unwrap_err(),
                InputError::InvalidContext,
                "{root}"
            );
        }
    }

    #[test]
    fn exact_conversation_identity_is_required_for_continuation() {
        let mut value = example();
        value["conversation"] = json!({"mode":"continue"});
        assert!(parse(&value).is_err());
        value["conversation"] = json!({"mode":"continue","child_session_id":"child-1"});
        assert!(parse(&value).is_ok());
        value["conversation"] = json!({"mode":"fresh","child_session_id":"child-1"});
        assert!(parse(&value).is_err());
    }

    #[test]
    fn digest_is_order_independent_but_prompt_sensitive() {
        let mut value = example();
        value["labels"] = json!({"round":"2","aspect":"security"});
        let a = parse(&value).unwrap();
        let pretty = serde_json::to_vec_pretty(&value).unwrap();
        assert_eq!(a.digest(), Request::parse(&pretty).unwrap().digest());
        value["instruction"] = json!("Different task");
        assert_ne!(a.digest(), parse(&value).unwrap().digest());
        // Native schemas keep their caller order, but identity ignores it.
        value["output"] =
            json!({"format":"json_schema","schema":{"type":"object","required":["a"]}});
        let ordered = parse(&value).unwrap();
        let text = serde_json::to_string(&value).unwrap().replace(
            r#"{"type":"object","required":["a"]}"#,
            r#"{"required":["a"],"type":"object"}"#,
        );
        let reordered = Request::parse(text.as_bytes()).unwrap();
        assert_ne!(
            serde_json::to_string(&ordered).unwrap(),
            serde_json::to_string(&reordered).unwrap()
        );
        assert_eq!(ordered.digest(), reordered.digest());
    }

    #[test]
    fn duplicate_fields_and_large_inputs_fail_without_echoing_source() {
        let text = serde_json::to_string(&example()).unwrap();
        let duplicated = text.replacen("{", "{\"schema\":1,", 1);
        assert_eq!(
            Request::parse(duplicated.as_bytes()).unwrap_err(),
            InputError::InvalidContract
        );
        assert_eq!(
            Request::parse(&vec![b' '; MAX_REQUEST_BYTES + 1]).unwrap_err(),
            InputError::RequestTooLarge
        );
        assert_eq!(InputError::InvalidContract.to_string(), "invalid_contract");
        assert_eq!(
            format!("{:?}", parse(&example()).unwrap()),
            "ManagedCallRequest([private])"
        );
    }
}
