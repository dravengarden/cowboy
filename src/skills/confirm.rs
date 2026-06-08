//! confirm-detect skill (design §A/§I): classify the agent's last turn — is it
//! asking the user (`awaiting_user`) and/or did it finish the task (`done`)? ONE
//! LLM call, multi-field verdict. Recall-first: when genuinely unsure whether
//! it's a question, prefer `awaiting_user: true` (a false hold costs one tap; a
//! false drain sends a wrong answer). L1 (Step 16) handles the cheap-certain
//! cases; this is the L2 fallback.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{SkillMeta, Verdict};
use crate::inference::{CompleteRequest, InferenceProvider, Message};

/// The stable system prefix — instructions + few-shot. Kept FIRST + constant so
/// DeepSeek's prefix cache hits across turns; only the per-turn text varies.
const SYSTEM_PROMPT: &str = r#"你是一个「turn 结束」分类器。每次给你 coding agent 刚刚说完、停下来交还给用户的最后一段话，你要判断两件事，并且只输出一个 JSON 对象（不要解释、不要代码块）：
{"awaiting_user": <bool>, "done": <bool>, "confidence": <0..1>, "reason": "<简短>"}

判定规则：
- awaiting_user：agent 是否在向用户提问、请求确认、给出选项让用户选、或表示需要用户输入/决定才能继续。即使是间接询问、或只在结尾问一句，也算 true。拿不准时偏向 true。
- done：agent 是否已经完成了当前的任务/交付物（值得通知用户「完成了」）。纯过程汇报、还在继续干活、或只是问问题，则为 false。
- 两者可以同时为 true，例如「A 做完了，要不要继续做 B?」。
- confidence 只是参考，不影响判定。

示例：
输入：「我把登录接口改好了，你看下还要不要加测试？」→ {"awaiting_user": true, "done": true, "confidence": 0.9, "reason": "完成并提问"}
输入：「全部完成，已通过所有测试。」→ {"awaiting_user": false, "done": true, "confidence": 0.95, "reason": "完成无提问"}
输入：「你想用方案 A 还是方案 B?」→ {"awaiting_user": true, "done": false, "confidence": 0.97, "reason": "二选一提问"}
输入：「正在重构模块，稍后继续。」→ {"awaiting_user": false, "done": false, "confidence": 0.8, "reason": "过程汇报"}
输入：「需要我帮你把它部署上线吗？」→ {"awaiting_user": true, "done": false, "confidence": 0.9, "reason": "征求是否继续"}
输入：「Done. Let me know if you want anything else.」→ {"awaiting_user": false, "done": true, "confidence": 0.85, "reason": "完成+礼貌结尾，非实质提问"}"#;

/// The skill's inspectable metadata (Info UI).
#[must_use]
pub fn meta() -> SkillMeta {
    SkillMeta {
        id: "confirm-detect",
        title: "Confirm detection",
        description: "判断 agent 这轮是否在等你回答 / 是否完成了任务（用于队列 hold、提醒、未来推送）。",
        prompt_template: SYSTEM_PROMPT,
        extract: "LLM 以 JSON 返回 {awaiting_user, done, confidence, reason}；awaiting_user=true 即 hold 队列 + 显示「在等你」widget。",
    }
}

/// Raw JSON shape the model returns (lenient: missing fields default).
#[derive(Debug, Deserialize, Default)]
struct Raw {
    #[serde(default)]
    awaiting_user: bool,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    reason: String,
}

/// Parse the model's JSON text into a `Verdict`. Tolerant of a leading/trailing
/// fence or prose by extracting the first `{ ... }` block.
fn parse_verdict(text: &str) -> Result<Verdict> {
    let json = extract_json(text).unwrap_or(text);
    let raw: Raw = serde_json::from_str(json).with_context(|| format!("parse verdict: {text}"))?;
    Ok(Verdict {
        awaiting_user: raw.awaiting_user,
        done: raw.done,
        confidence: raw.confidence,
        reason: raw.reason,
    })
}

/// Best-effort: the substring from the first `{` to the last `}`.
fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then_some(&text[start..=end])
}

/// Classify the agent's last turn. Tries the agent-provider's **L1** detector
/// first (deterministic, no LLM — design §B); only an ambiguous `EndTurn` falls
/// through to the **L2** DeepSeek judge below. The system prompt is the stable
/// cached prefix; only `final_text` varies (cheap).
///
/// # Errors
/// If the L2 inference call or the JSON parse fails (the caller treats an error
/// as "stay held" — continuity over a wrong drain).
pub async fn classify(
    agent_provider: &str,
    stop_reason: Option<&str>,
    inference: &dyn InferenceProvider,
    final_text: &str,
) -> Result<Verdict> {
    let ctx = crate::provider::confirm::TurnEndCtx { stop_reason, final_text };
    if let Some(v) = crate::provider::confirm::l1(agent_provider, &ctx) {
        return Ok(v); // deterministic — no LLM call
    }
    let messages = vec![Message::system(SYSTEM_PROMPT), Message::user(final_text.to_owned())];
    let resp = inference.complete(CompleteRequest::json_judge(messages, 128)).await?;
    parse_verdict(&resp.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{CompleteResponse, ModelSource, Usage};
    use async_trait::async_trait;

    #[test]
    fn parses_verdict_variants() {
        let v =
            parse_verdict(r#"{"awaiting_user": true, "done": false, "confidence": 0.9, "reason": "q"}"#)
                .unwrap();
        assert!(v.awaiting_user && !v.done);
        // tolerant of surrounding prose / a code fence
        let v2 = parse_verdict("```json\n{\"awaiting_user\": false, \"done\": true}\n```").unwrap();
        assert!(!v2.awaiting_user && v2.done);
    }

    struct Mock(&'static str);
    #[async_trait]
    impl InferenceProvider for Mock {
        fn id(&self) -> &str {
            "mock"
        }
        fn models(&self) -> ModelSource {
            ModelSource::Static(vec![])
        }
        async fn complete(&self, _req: CompleteRequest) -> Result<CompleteResponse> {
            Ok(CompleteResponse { text: self.0.to_owned(), usage: Usage::default() })
        }
        async fn raw(&self, _body: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }

    // A mock that panics if called — proves L1 short-circuits without the LLM.
    struct NeverCalled;
    #[async_trait]
    impl InferenceProvider for NeverCalled {
        fn id(&self) -> &str {
            "never"
        }
        fn models(&self) -> ModelSource {
            ModelSource::Static(vec![])
        }
        async fn complete(&self, _req: CompleteRequest) -> Result<CompleteResponse> {
            panic!("L1 should have short-circuited — L2 must not run");
        }
        async fn raw(&self, _body: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }

    #[tokio::test]
    async fn end_turn_falls_through_to_l2() {
        let mock = Mock(r#"{"awaiting_user": true, "done": true, "confidence": 0.8, "reason": "x"}"#);
        // EndTurn is ambiguous → L1 returns None → the LLM (mock) decides.
        let v = classify("claude-code", Some("EndTurn"), &mock, "做完了 A，要不要做 B?").await.unwrap();
        assert!(v.awaiting_user);
        assert!(v.done);
    }

    #[tokio::test]
    async fn cancelled_short_circuits_l1_no_llm() {
        // A non-EndTurn stop is deterministic → L2 must NOT be called.
        let v = classify("codex", Some("Cancelled"), &NeverCalled, "anything").await.unwrap();
        assert!(!v.awaiting_user);
        assert!(!v.done);
    }
}
