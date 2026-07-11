//! Pluggable LLM inference providers (design: tasks/active/confirm-detect-skills §C).
//!
//! A GENERAL, reusable inference layer. The skills' L2 judge is the first
//! consumer; other features will call LLMs too. Design rule (user directive): do
//! NOT flatten providers to a lowest-common-denominator. Each provider exposes its
//! FULL native surface through its OWN typed request/response (see [`deepseek`]),
//! and a [`InferenceProvider::raw`] JSON pass-through makes a brand-new vendor
//! feature usable the day it ships. THIS module holds only the portable core that
//! generic callers (the judge) use; reach for the provider's concrete type when
//! you want its full power.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod deepseek;

/// Chat role for the portable message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// A portable chat message (the core path; provider types carry the rest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

/// The portable request a generic caller can express. Provider-specific power
/// (FIM, grammar modes, logprobs, every native param) lives on each provider's
/// OWN request type — nothing here clamps it.
#[derive(Debug, Clone)]
pub struct CompleteRequest {
    pub messages: Vec<Message>,
    /// Force a JSON-object response (judges want structured output).
    pub json: bool,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl CompleteRequest {
    /// A deterministic judge call WITHOUT forced JSON mode — thinking models
    /// (deepseek-v4-pro) commonly reject `response_format=json_object`, so we
    /// instruct JSON-only in the prompt and parse tolerantly instead.
    pub fn judge(messages: Vec<Message>, max_tokens: u32) -> Self {
        Self {
            messages,
            json: false,
            temperature: 0.0,
            max_tokens,
        }
    }

    /// A deterministic, JSON-mode request sized for a short classifier verdict.
    pub fn json_judge(messages: Vec<Message>, max_tokens: u32) -> Self {
        Self {
            messages,
            json: true,
            temperature: 0.0,
            max_tokens,
        }
    }
}

/// The portable response + the universally useful usage / prefix-cache counters.
#[derive(Debug, Clone)]
pub struct CompleteResponse {
    pub text: String,
    pub usage: Usage,
}

/// Token accounting, incl. DeepSeek-style prefix-cache hit/miss (0 when a
/// provider doesn't report it). Logged to verify cache effectiveness.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cache_hit_tokens: u32,
    pub cache_miss_tokens: u32,
}

/// Where a provider's selectable models come from. DYNAMIC by design — model ids
/// churn (e.g. DeepSeek's legacy aliases deprecate 2026-07-24), so callers + the
/// UI read this rather than hardcoding ids.
#[derive(Debug, Clone)]
pub enum ModelSource {
    /// Built-in list of (id, human label).
    Static(Vec<(String, String)>),
    // Future: Endpoint(String) — fetch the provider's `/models`.
}

/// The general inference-provider contract. Implementors expose their full native
/// surface through their own types + [`Self::raw`]; this trait is just the
/// portable core, kept dyn-safe (via `async_trait`) so a registry can hold
/// `Box<dyn InferenceProvider>`.
#[async_trait]
pub trait InferenceProvider: Send + Sync {
    /// Provider id, e.g. `"deepseek"`.
    fn id(&self) -> &str;
    /// Selectable models (dynamic).
    fn models(&self) -> ModelSource;
    /// Portable structured completion for generic callers.
    async fn complete(&self, req: CompleteRequest) -> Result<CompleteResponse>;
    /// Full-surface escape hatch: arbitrary provider request JSON → the
    /// unmodified provider response JSON. Nothing is clamped.
    async fn raw(&self, body: serde_json::Value) -> Result<serde_json::Value>;
}
