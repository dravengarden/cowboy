//! confirm-detect skill (design §A/§I): classify the agent's last turn — is it
//! asking the user (`awaiting_user`) and/or did it finish the task (`done`)? ONE
//! LLM call, multi-field verdict. Recall-first: when genuinely unsure whether
//! it's a question, prefer `awaiting_user: true` (a false hold costs one tap; a
//! false drain sends a wrong answer). L1 (Step 16) handles the cheap-certain
//! cases; this is the L2 fallback.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{SkillMeta, Verdict};
use crate::inference::{CompleteResponse, Usage};

/// A judge run's full outcome — the verdict PLUS the observability detail the Info
/// / overlay surfaces let the user inspect (which layer decided, the raw model
/// output, token usage). `usage` is `None` for an L1 (no LLM call).
pub struct JudgeOutcome {
    pub verdict: Verdict,
    /// "L1" (deterministic stop-reason) or "L2" (the Codex judge).
    pub layer: &'static str,
    /// The model's raw response text (L2), or the L1 reason — what `output` shows.
    pub raw_output: String,
    pub usage: Option<Usage>,
}

/// The skill's inspectable metadata (Info UI).
#[must_use]
pub fn meta() -> SkillMeta {
    SkillMeta {
        id: "confirm-detect",
        title: "Confirm detection",
        description: "判断 agent 这轮是否在等你回答 / 是否完成了任务（用于队列 hold、提醒、未来推送）。",
        prompt_template: crate::inference::codex::DEVELOPER_INSTRUCTIONS,
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

/// Parse the model's output into a `Verdict`. ROBUST TO TRUNCATION: v4-pro
/// sometimes writes a long `reason` that `max_tokens` cuts mid-string, leaving
/// invalid JSON — but the two booleans are emitted FIRST, so we pull them by a
/// simple scan and never fail on a cut-off reason (the old strict parse turned
/// every truncation into "judge failed → hold the queue"). confidence/reason are
/// best-effort from a clean `{ … }` block when one exists.
fn parse_verdict(text: &str) -> Result<Verdict> {
    let awaiting_user = bool_field(text, "awaiting_user");
    let done = bool_field(text, "done");
    if let (Some(awaiting_user), Some(done)) = (awaiting_user, done) {
        let (confidence, reason) = extract_json(text)
            .and_then(|j| serde_json::from_str::<Raw>(j).ok())
            .map_or((0.0, String::new()), |r| (r.confidence, r.reason));
        return Ok(Verdict {
            awaiting_user,
            done,
            confidence,
            reason,
        });
    }
    // Neither boolean present → genuinely unparseable.
    let json = extract_json(text).unwrap_or(text);
    let raw: Raw = serde_json::from_str(json).with_context(|| format!("parse verdict: {text}"))?;
    Ok(Verdict {
        awaiting_user: raw.awaiting_user,
        done: raw.done,
        confidence: raw.confidence,
        reason: raw.reason,
    })
}

/// Find `"<key>": true|false` in the raw text without needing valid JSON. The key
/// is ASCII so byte indexing is safe.
fn bool_field(text: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{key}\"");
    let after = &text[text.find(&pat)? + pat.len()..];
    let v = after.trim_start_matches([':', ' ', '\t', '\n', '\r']);
    if v.starts_with("true") {
        Some(true)
    } else if v.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Best-effort: the substring from the first `{` to the last `}`.
fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then_some(&text[start..=end])
}

/// Convert one structured Codex response into the domain verdict recorded by
/// the Hub. `outputSchema` makes this strict JSON; the tolerant parser remains
/// as a compatibility guard around runtime regressions.
pub fn classify_response(resp: CompleteResponse) -> Result<JudgeOutcome> {
    let verdict = parse_verdict(&resp.text)?;
    Ok(JudgeOutcome {
        verdict,
        layer: "L2",
        raw_output: resp.text,
        usage: Some(resp.usage),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{CompleteResponse, Usage};

    #[test]
    fn parses_verdict_variants() {
        let v = parse_verdict(
            r#"{"awaiting_user": true, "done": false, "confidence": 0.9, "reason": "q"}"#,
        )
        .unwrap();
        assert!(v.awaiting_user && !v.done);
        // tolerant of surrounding prose / a code fence
        let v2 = parse_verdict("```json\n{\"awaiting_user\": false, \"done\": true}\n```").unwrap();
        assert!(!v2.awaiting_user && v2.done);
        // TRUNCATED JSON (the real bug): a long reason cut by max_tokens leaves
        // invalid JSON, but the booleans came first → still parses.
        let v3 = parse_verdict(
            "{\"awaiting_user\": false, \"done\": true, \"confidence\": 0.9, \"reason\": \"汇报完成并邀请用户去验证，结尾是礼貌性的邀请并非",
        )
        .unwrap();
        assert!(!v3.awaiting_user && v3.done);
    }

    #[test]
    fn structured_response_becomes_l2_outcome() {
        let o = classify_response(CompleteResponse {
            text: r#"{"awaiting_user": true, "done": true, "confidence": 0.8, "reason": "x"}"#
                .to_owned(),
            usage: Usage::default(),
        })
        .unwrap();
        assert_eq!(o.layer, "L2");
        assert!(o.verdict.awaiting_user);
        assert!(o.verdict.done);
        assert!(o.usage.is_some());
    }
}
