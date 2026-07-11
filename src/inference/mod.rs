//! Model-backed inference used by cowboy's turn-end classifier.

use serde::Serialize;

pub mod codex;

/// The portable response + the universally useful usage / prefix-cache counters.
#[derive(Debug, Clone)]
pub struct CompleteResponse {
    pub text: String,
    pub usage: Usage,
}

/// Token accounting reported by Codex app-server.
#[derive(Debug, Clone, Default, Serialize)]
#[allow(clippy::struct_field_names)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cache_hit_tokens: u32,
    pub cache_miss_tokens: u32,
}
