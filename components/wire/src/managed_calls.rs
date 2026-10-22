//! Closed input contract for managed cross-Provider calls. A validated request
//! is caller input, never a parent identity or permission to execute.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub mod lifecycle;
pub mod media;
pub mod protocol;
pub mod review;

pub const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// Shared by the CLI and MCP; neither surface may silently weaken a review.
pub fn review_instruction(design: bool, focus: &str, base: Option<&str>) -> String {
    let task = if base.is_some() && design {
        "Review the changed design documents in the captured worktree for gaps, contradictions and risks."
    } else if base.is_some() {
        "Review the change in the captured worktree. Read surrounding code as needed."
    } else if design {
        "Review the design documents changed in this worktree for gaps, contradictions and risks."
    } else {
        "Review this worktree's change (uncommitted work and commits not yet on the default branch). Read surrounding code as needed."
    };
    let focus = focus.trim();
    let focus = if focus.is_empty() {
        String::new()
    } else {
        format!("\nFocus: {focus}")
    };
    let result = format!(
        "{task}{focus}\nReport actionable correctness, security, data-loss or contract problems, most severe first. Each finding needs path:line, trigger, impact and a concrete fix. Be concise; omit praise, restated diff and style nits. Do not omit distinct serious findings to meet a count or length target. If blocked or incomplete, say so and identify the missing evidence."
    );
    match base {
        Some(base) => format!("{result}\n{}", review::input_contract(base)),
        None => format!(
            "{result} Only after completing the requested review with no findings, reply exactly: No findings."
        ),
    }
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

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
    /// A review of design artifacts (specs, plans, records) rather than code.
    DesignReview,
    Analysis,
    ImageGenerate,
    ImageEdit,
}

impl Purpose {
    pub fn profile(self) -> cowboy_provider_sdk::ManagedRuntimeProfile {
        use cowboy_provider_sdk::ManagedRuntimeProfile;
        match self {
            Self::ImageGenerate | Self::ImageEdit => ManagedRuntimeProfile::ImageV1,
            _ => ManagedRuntimeProfile::ReadOnlyV1,
        }
    }

    pub fn is_image(self) -> bool {
        matches!(self, Self::ImageGenerate | Self::ImageEdit)
    }
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
pub fn absolute_root(value: &str) -> bool {
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

    pub fn validate(&self) -> bool {
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

pub fn relative_file(value: &str) -> bool {
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
        if !review::validate(self) {
            return Err(InputError::InvalidContext);
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
        if self.purpose.is_image()
            && (!self.output.is_text()
                || self.context.files.len() > media::MAX_IMAGES
                || (self.purpose == Purpose::ImageEdit
                    && self.context.files.is_empty()
                    && matches!(self.conversation, Conversation::Fresh {})))
        {
            return Err(InputError::InvalidContext);
        }
        Ok(())
    }

    /// The explicit task and file names only; never copy the parent's history
    /// or all project guidance into a native image turn.
    pub fn prompt(&self) -> String {
        if let Some(prompt) = review::prompt(self) {
            return prompt;
        }
        if !self.purpose.is_image() {
            return self.instruction.clone();
        }
        format!(
            "Use native image generation to {} the requested image. Return the generated image through the native tool, with a brief final caption. Do not substitute code, an SVG, a URL or a claim of success. Input image paths relative to this workspace: {}.\n{}",
            if self.purpose == Purpose::ImageEdit {
                "edit"
            } else {
                "generate"
            },
            serde_json::to_string(&self.context.files).expect("paths serialize"),
            self.instruction,
        )
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
pub fn canonical_digest(value: &serde_json::Value) -> String {
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
