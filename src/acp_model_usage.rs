//! Per-model token usage of one session, as its adapter accounts it.
//!
//! Both production adapters end a prompt with `_meta.quota.model_usage`: one
//! row per model with that turn's `token_count` (claude-agent-acp
//! `turnQuotaMeta`, which also counts Task subagents and internal calls;
//! codex-acp `buildQuotaMeta`, one row for the session's current model). The
//! worker adds the rows of every turn it ran and carries the totals on the
//! session usage it reports, under `raw._cowboy.modelUsage`, so the
//! Controller and older clients pass it through unchanged.

use serde_json::{Value, json};

use crate::agent_model::SessionUsage;

/// Distinct models kept per session; a later unseen model is not added.
const MAX_MODELS: usize = 32;
/// Bytes kept of one model name.
const MODEL_BYTES: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    model: String,
    total: u64,
    input: u64,
    output: u64,
    cached: u64,
    turns: u32,
}

/// The models a session's turns ran on since this worker started.
#[derive(Debug, Default)]
pub struct ModelUsage {
    rows: Vec<Row>,
    /// The last usage reported, so a turn's rows can be republished on it.
    last: Option<SessionUsage>,
}

fn count(token_count: &Value, key: &str) -> u64 {
    token_count.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn bounded(model: &str) -> String {
    let model = model.trim();
    if model.len() <= MODEL_BYTES {
        return model.to_owned();
    }
    let mut cut = MODEL_BYTES;
    while !model.is_char_boundary(cut) {
        cut -= 1;
    }
    model[..cut].to_owned()
}

impl ModelUsage {
    /// Attach the totals to `usage` and remember it.
    pub fn annotate(&mut self, mut usage: SessionUsage) -> SessionUsage {
        self.apply(&mut usage.raw);
        self.last = Some(usage.clone());
        usage
    }

    /// Add one prompt response's `_meta`. Returns the last usage with the new
    /// totals to republish, or `None` when nothing changed or no usage was
    /// reported yet (the next `usage_update` carries them).
    pub fn record_turn(
        &mut self,
        meta: Option<&serde_json::Map<String, Value>>,
    ) -> Option<SessionUsage> {
        let rows = meta?.get("quota")?.get("model_usage")?.as_array()?;
        let mut changed = false;
        for row in rows {
            let Some(model) = row
                .get("model")
                .and_then(Value::as_str)
                .map(bounded)
                .filter(|model| !model.is_empty())
            else {
                continue;
            };
            let Some(tokens) = row.get("token_count").filter(|tokens| tokens.is_object()) else {
                continue;
            };
            let total = count(tokens, "totalTokens");
            if total == 0 {
                continue;
            }
            let index = match self.rows.iter().position(|known| known.model == model) {
                Some(index) => index,
                None if self.rows.len() < MAX_MODELS => {
                    self.rows.push(Row {
                        model,
                        total: 0,
                        input: 0,
                        output: 0,
                        cached: 0,
                        turns: 0,
                    });
                    self.rows.len() - 1
                }
                None => continue,
            };
            let entry = &mut self.rows[index];
            entry.total = entry.total.saturating_add(total);
            entry.input = entry.input.saturating_add(count(tokens, "inputTokens"));
            entry.output = entry.output.saturating_add(count(tokens, "outputTokens"));
            entry.cached = entry
                .cached
                .saturating_add(count(tokens, "cachedInputTokens"));
            entry.turns = entry.turns.saturating_add(1);
            changed = true;
        }
        if !changed {
            return None;
        }
        let mut usage = self.last.clone()?;
        self.apply(&mut usage.raw);
        self.last = Some(usage.clone());
        Some(usage)
    }

    fn apply(&self, raw: &mut Value) {
        let Some(raw) = raw.as_object_mut() else {
            return;
        };
        if self.rows.is_empty() {
            return;
        }
        let cowboy = raw.entry("_cowboy").or_insert_with(|| json!({}));
        let Some(cowboy) = cowboy.as_object_mut() else {
            return;
        };
        let mut rows = self.rows.clone();
        rows.sort_by_key(|row| std::cmp::Reverse(row.total));
        cowboy.insert(
            "modelUsage".to_owned(),
            rows.iter()
                .map(|row| {
                    json!({
                        "model": row.model,
                        "totalTokens": row.total,
                        "inputTokens": row.input,
                        "outputTokens": row.output,
                        "cachedInputTokens": row.cached,
                        "turns": row.turns,
                    })
                })
                .collect::<Vec<_>>()
                .into(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(used: u64) -> SessionUsage {
        SessionUsage {
            used,
            size: 200_000,
            raw: json!({ "sessionUpdate": "usage_update", "used": used, "size": 200_000 }),
            observed_at_ms: 7,
        }
    }

    fn meta(rows: &Value) -> serde_json::Map<String, Value> {
        json!({ "quota": { "token_count": {}, "model_usage": rows } })
            .as_object()
            .cloned()
            .unwrap()
    }

    #[test]
    fn turns_add_up_per_model_and_ride_on_the_last_usage() {
        let mut tally = ModelUsage::default();
        // claude-agent-acp: subagents and internal calls have their own rows.
        let first = meta(&json!([
            { "model": "claude-opus-4-5", "token_count": {
                "totalTokens": 1000, "inputTokens": 100, "cachedInputTokens": 800,
                "cachedWriteTokens": 50, "outputTokens": 50, "reasoningOutputTokens": 0 } },
            { "model": "claude-haiku-4-5", "token_count": { "totalTokens": 300, "inputTokens": 250, "outputTokens": 50 } },
        ]));
        assert!(
            tally.record_turn(Some(&first)).is_none(),
            "nothing to republish before a usage report"
        );
        let annotated = tally.annotate(usage(5));
        let rows = &annotated.raw["_cowboy"]["modelUsage"];
        assert_eq!(rows[0]["model"], "claude-opus-4-5");
        assert_eq!(rows[0]["cachedInputTokens"], 800);
        assert_eq!(rows[1]["model"], "claude-haiku-4-5");
        assert_eq!(annotated.raw["used"], 5, "the ACP update itself is kept");

        // codex-acp: one row for the current model.
        let second = meta(&json!([
            { "model": "claude-haiku-4-5", "token_count": { "totalTokens": 900, "inputTokens": 800, "outputTokens": 100 } },
        ]));
        let republished = tally.record_turn(Some(&second)).unwrap();
        assert_eq!(republished.used, 5);
        assert_eq!(republished.observed_at_ms, 7);
        let rows = &republished.raw["_cowboy"]["modelUsage"];
        assert_eq!(rows[0]["model"], "claude-haiku-4-5", "largest first");
        assert_eq!(rows[0]["totalTokens"], 1200);
        assert_eq!(rows[0]["turns"], 2);
        assert_eq!(rows[1]["turns"], 1);

        // A response without a breakdown, or with empty rows, changes nothing.
        assert!(tally.record_turn(None).is_none());
        assert!(tally.record_turn(Some(&meta(&json!([])))).is_none());
        assert!(
            tally
                .record_turn(Some(&meta(&json!([{ "model": "x", "token_count": null }]))))
                .is_none()
        );
        // A later usage report keeps carrying the totals.
        assert_eq!(
            tally.annotate(usage(9)).raw["_cowboy"]["modelUsage"][0]["totalTokens"],
            1200
        );
    }

    #[test]
    fn a_session_without_a_breakdown_reports_no_rows() {
        let mut tally = ModelUsage::default();
        let annotated = tally.annotate(usage(1));
        assert!(annotated.raw.get("_cowboy").is_none());
    }
}
