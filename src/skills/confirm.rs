//! confirm-detect skill (design §A/§I): classify the agent's last turn — is it
//! asking the user (`awaiting_user`) and/or did it finish the task (`done`)? ONE
//! LLM call, multi-field verdict. Recall-first: when genuinely unsure whether
//! it's a question, prefer `awaiting_user: true` (a false hold costs one tap; a
//! false drain sends a wrong answer). L1 (Step 16) handles the cheap-certain
//! cases; this is the L2 fallback.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{SkillMeta, Verdict};
use crate::inference::{CompleteRequest, InferenceProvider, Message, Usage};

/// A judge run's full outcome — the verdict PLUS the observability detail the Info
/// / overlay surfaces let the user inspect (which layer decided, the raw model
/// output, token usage). `usage` is `None` for an L1 (no LLM call).
pub struct JudgeOutcome {
    pub verdict: Verdict,
    /// "L1" (deterministic stop-reason) or "L2" (the DeepSeek judge).
    pub layer: &'static str,
    /// The model's raw response text (L2), or the L1 reason — what `output` shows.
    pub raw_output: String,
    pub usage: Option<Usage>,
}

/// The stable system prefix — instructions + few-shot. Kept FIRST + constant so
/// DeepSeek's prefix cache hits across turns; only the per-turn text varies.
const SYSTEM_PROMPT: &str = r#"你是一个「coding agent 回合结束」分类器。输入是 agent 刚说完、停下来把控制权交还给用户的最后一段话（可能中文也可能英文）。只输出一个 JSON（不要解释、不要 markdown）：
{"awaiting_user": <bool>, "done": <bool>, "confidence": <0..1>, "reason": "<简短>"}

按 agent 的真实意图判断，不要只看表面措辞、有没有问号、或客套话。

awaiting_user —— agent 是否在「等用户回应之后才能继续」：
- true：明确提问、让用户在选项间挑一个、请求确认某个有风险/不可逆的操作、或表示缺少信息或需要用户拍板才能往下做。
- false：只是陈述、汇报进度、说明自己做了什么、宣布完成；以及不影响继续的客套结尾（如「有需要再说」「还有别的吗」「希望有帮助」）——这些并不是在等你回应。
- 真正拿不准、但确实像在征求用户拍板时 → 偏 true（漏掉一个真问题，比偶尔多问一次代价更大）。

done —— 当前交付的任务/产物是否已经完成（值得提示「完成了」）：
- true：明确说做完了 / 跑通了 / 已提交。
- false：还在中途、只是过程汇报、或仅仅在提问。
- 两者可同时为 true（例：「X 做完了，要不要接着做 Y？」）。

示例（覆盖各种角落，看意图而非措辞）：
「你想用方案 A 还是 B?」→ {"awaiting_user": true, "done": false}
「需要我帮你部署上线吗?」→ {"awaiting_user": true, "done": false}
「这步不可逆，确认要删旧文件吗?」→ {"awaiting_user": true, "done": false}
「全部完成，测试通过，已提交。」→ {"awaiting_user": false, "done": true}
「搞定，有需要随时说。」→ {"awaiting_user": false, "done": true}
「登录接口改好了，要不要顺便加测试?」→ {"awaiting_user": true, "done": true}
「我先跑一下测试看看结果。」→ {"awaiting_user": false, "done": false}
「All set — pushed. Let me know if anything else.」→ {"awaiting_user": false, "done": true}"#;

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
) -> Result<JudgeOutcome> {
    let ctx = crate::provider::confirm::TurnEndCtx { stop_reason, final_text };
    if let Some(v) = crate::provider::confirm::l1(agent_provider, &ctx) {
        // Deterministic — no LLM call.
        let raw_output = v.reason.clone();
        return Ok(JudgeOutcome { verdict: v, layer: "L1", raw_output, usage: None });
    }
    let messages = vec![Message::system(SYSTEM_PROMPT), Message::user(final_text.to_owned())];
    let resp = inference.complete(CompleteRequest::json_judge(messages, 128)).await?;
    let verdict = parse_verdict(&resp.text)?;
    Ok(JudgeOutcome { verdict, layer: "L2", raw_output: resp.text, usage: Some(resp.usage) })
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
        let o = classify("claude-code", Some("EndTurn"), &mock, "做完了 A，要不要做 B?").await.unwrap();
        assert_eq!(o.layer, "L2");
        assert!(o.verdict.awaiting_user);
        assert!(o.verdict.done);
        assert!(o.usage.is_some());
    }

    #[tokio::test]
    async fn cancelled_short_circuits_l1_no_llm() {
        // A non-EndTurn stop is deterministic → L2 must NOT be called.
        let o = classify("codex", Some("Cancelled"), &NeverCalled, "anything").await.unwrap();
        assert_eq!(o.layer, "L1");
        assert!(!o.verdict.awaiting_user);
        assert!(!o.verdict.done);
        assert!(o.usage.is_none());
    }
}
