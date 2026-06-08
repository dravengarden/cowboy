//! DeepSeek inference provider — OpenAI-compatible `/chat/completions`, with the
//! FULL native surface exposed (design §C/§D: never lowest-common-denominator).
//!
//! - [`DeepSeek::complete`] (trait) is the portable judge path.
//! - [`DeepSeek::chat`] takes a fully-typed [`ChatRequest`] → [`ChatResponse`]
//!   exposing every documented param + response field (incl. DeepSeek's
//!   prefix-cache token counts + the reasoner's `reasoning_content`).
//! - [`DeepSeek::raw`] is the unclamped escape hatch: arbitrary request JSON →
//!   the unmodified response JSON, so a brand-new vendor field is usable the day
//!   it ships.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{CompleteRequest, CompleteResponse, InferenceProvider, Message, ModelSource, Role, Usage};

const API_BASE: &str = "https://api.deepseek.com";
/// Default model — the cheap, fast non-thinking model, right for a short
/// classifier. NOT hardcoded into control flow: it's the default of a stored,
/// UI-selectable value (see `model_list` / Step 18).
pub const DEFAULT_MODEL: &str = "deepseek-v4-pro";

/// A configured DeepSeek client (one per stored config).
pub struct DeepSeek {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base: String,
}

impl DeepSeek {
    pub fn new(api_key: String, model: String) -> Self {
        Self { client: reqwest::Client::new(), api_key, model, base: API_BASE.to_owned() }
    }

    /// Built-in selectable models (id, human label). DYNAMIC by design — a
    /// `/models` fetch can replace this later; ids stay data, never control flow.
    pub fn model_list() -> Vec<(String, String)> {
        vec![
            (DEFAULT_MODEL.to_owned(), "V4 Pro — thinking, most accurate (default)".to_owned()),
            ("deepseek-v4-flash".to_owned(), "V4 Flash — fast & cheap".to_owned()),
        ]
    }

    /// Full-surface call: a fully-typed request → fully-typed response. The caller
    /// controls every parameter; nothing is clamped.
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base))
            .bearer_auth(&self.api_key)
            .json(req)
            .send()
            .await
            .context("deepseek: request failed")?;
        let status = resp.status();
        let body = resp.text().await.context("deepseek: read body")?;
        if !status.is_success() {
            anyhow::bail!("deepseek: HTTP {status}: {body}");
        }
        serde_json::from_str(&body).with_context(|| format!("deepseek: parse response: {body}"))
    }
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

#[async_trait]
impl InferenceProvider for DeepSeek {
    fn id(&self) -> &str {
        "deepseek"
    }

    fn models(&self) -> ModelSource {
        ModelSource::Static(Self::model_list())
    }

    async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse> {
        let messages = req
            .messages
            .iter()
            .map(|m: &Message| ChatMessage {
                role: role_str(m.role).to_owned(),
                content: Some(m.content.clone()),
                ..Default::default()
            })
            .collect();
        let chat = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: Some(req.temperature),
            max_tokens: Some(req.max_tokens),
            response_format: req.json.then(|| ResponseFormat { kind: "json_object".to_owned() }),
            ..Default::default()
        };
        let r = self.chat(&chat).await?;
        let text = r.choices.first().and_then(|c| c.message.content.clone()).unwrap_or_default();
        let u = r.usage.unwrap_or_default();
        Ok(CompleteResponse {
            text,
            usage: Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                cache_hit_tokens: u.prompt_cache_hit_tokens,
                cache_miss_tokens: u.prompt_cache_miss_tokens,
            },
        })
    }

    async fn raw(&self, body: serde_json::Value) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("deepseek: raw request failed")?;
        resp.json::<serde_json::Value>().await.context("deepseek: raw parse")
    }
}

// --- Full typed surface ------------------------------------------------------
// Every field DeepSeek documents on /chat/completions. `Option` + skip-if-none
// so an unset knob is simply omitted (server defaults apply); a caller wanting a
// field that isn't modelled yet uses `raw()`.

#[derive(Debug, Clone, Serialize, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    /// The reasoner model returns its chain-of-thought here, separate from
    /// `content`. Exposed (not dropped) per the full-surface rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<DsUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub index: u32,
    pub message: ChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub logprobs: Option<serde_json::Value>,
}

/// DeepSeek usage — incl. the prefix-cache hit/miss split that makes the cost
/// model work; surfaced so we can verify cache effectiveness in logs.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DsUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    #[serde(default)]
    pub prompt_cache_hit_tokens: u32,
    #[serde(default)]
    pub prompt_cache_miss_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_judge_shape() {
        let req = ChatRequest {
            model: "deepseek-v4-flash".to_owned(),
            messages: vec![ChatMessage { role: "user".to_owned(), content: Some("hi".to_owned()), ..Default::default() }],
            temperature: Some(0.0),
            max_tokens: Some(64),
            response_format: Some(ResponseFormat { kind: "json_object".to_owned() }),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(v["model"], "deepseek-v4-flash");
        assert_eq!(v["temperature"], 0.0);
        assert_eq!(v["response_format"]["type"], "json_object");
        // Unset knobs are omitted, not sent as null.
        assert!(v.get("top_p").is_none());
        assert!(v.get("tools").is_none());
    }

    #[test]
    fn response_parses_incl_cache_tokens() {
        let sample = r#"{
          "id": "abc", "model": "deepseek-v4-flash", "created": 1,
          "choices": [{"index":0,"message":{"role":"assistant","content":"{\"awaiting_user\":true}"},"finish_reason":"stop"}],
          "usage": {"prompt_tokens": 120, "completion_tokens": 8, "total_tokens": 128,
                    "prompt_cache_hit_tokens": 96, "prompt_cache_miss_tokens": 24}
        }"#;
        let r: ChatResponse = serde_json::from_str(sample).unwrap();
        assert_eq!(r.choices[0].message.content.as_deref(), Some("{\"awaiting_user\":true}"));
        let u = r.usage.unwrap();
        assert_eq!(u.prompt_cache_hit_tokens, 96);
        assert_eq!(u.prompt_cache_miss_tokens, 24);
    }
}
