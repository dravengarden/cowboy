//! Byte-bounded windowing for `calls.result`.
//!
//! Ordinary text reads (`documents.read`, `sessions.message`) stay on the
//! 4,000-scalar cap. `calls.result` accepts up to 12,000 Unicode scalars when
//! explicitly requested, then shrinks by scalar count if the serialized JSON
//! (including UTF-8 width, JSON escaping, and metadata) would exceed the
//! 16 KiB `cowboy_read` response budget with headroom for the `CallView`
//! envelope.

use serde_json::{Value, json};

use super::{arg_chars, arg_u64, explicit_arg};

/// Explicit maximum Unicode scalars for a single `calls.result` window.
pub(super) const MAX_CALL_RESULT_CHARS: u64 = 12_000;

/// Serialized JSON budget for the inner `result` object, reserving 2 KiB for
/// the surrounding `CallView` metadata envelope so `cowboy_read` stays within
/// `MAX_RESULT_BYTES` (16 KiB).
const RESULT_JSON_BUDGET_BYTES: usize = crate::agent_api::MAX_RESULT_BYTES - 2048;

/// Apply a Unicode-scalar window to a managed call's `result` object, fitting
/// the serialized JSON within `RESULT_JSON_BUDGET_BYTES` so large ASCII reviews
/// fit in one page while multi-byte or heavily escaped text never triggers
/// `result_too_large`.
pub(super) fn result_window(result: &mut Value, args: &Value) -> Result<(), String> {
    let offset = usize::try_from(arg_u64(args, "offset", 0, u64::from(u32::MAX))?)
        .map_err(|_| "invalid_argument")?;
    let requested = usize::try_from(arg_chars(args, 1000, MAX_CALL_RESULT_CHARS)?)
        .map_err(|_| "invalid_argument")?;
    let Some(full_text) = result["text"].as_str().map(str::to_owned) else {
        partial_structured(result);
        return Ok(());
    };
    let char_starts: Vec<usize> = full_text.char_indices().map(|(idx, _)| idx).collect();
    let total = char_starts.len();
    // Preserve unwindowed safety: an implicit read over 1,000 chars refuses
    // instead of silently presenting a prefix as the complete review.
    if !explicit_arg(args, "offset") && !explicit_arg(args, "chars") && total > requested {
        return Err("text_window_required".into());
    }
    let offset = offset.min(total);
    let remaining = total - offset;
    let max_take = requested.min(remaining);

    let slice_text = |take: usize| -> &str {
        if take == 0 {
            return "";
        }
        let start = char_starts[offset];
        let end = char_starts
            .get(offset + take)
            .copied()
            .unwrap_or(full_text.len());
        &full_text[start..end]
    };

    let build_candidate = |take: usize| -> Value {
        let mut candidate = result.clone();
        let end = offset + take;
        candidate["text"] = json!(slice_text(take));
        candidate["range"] = json!({
            "offset": offset,
            "next": (end < total).then_some(end),
            "total": total,
        });
        partial_structured(&mut candidate);
        candidate
    };

    let fits = |candidate: &Value| -> bool {
        serde_json::to_vec(candidate).is_ok_and(|bytes| bytes.len() <= RESULT_JSON_BUDGET_BYTES)
    };

    let candidate = build_candidate(max_take);
    if max_take <= 1 || fits(&candidate) {
        *result = candidate;
        return Ok(());
    }

    // Binary search the largest Unicode-scalar slice in 1..max_take whose
    // serialized JSON fits inside RESULT_JSON_BUDGET_BYTES. Always take at
    // least 1 scalar when remaining > 0 so `range.next` strictly advances.
    let mut lo = 1usize;
    let mut hi = max_take - 1;
    let mut best = 1usize;
    let mut best_candidate = build_candidate(1);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let cand = build_candidate(mid);
        if fits(&cand) {
            best = mid;
            best_candidate = cand;
            lo = mid + 1;
        } else {
            hi = mid.saturating_sub(1);
            if mid == 0 {
                break;
            }
        }
    }
    let _ = best;
    *result = best_candidate;
    Ok(())
}

/// A schema result's `structured` is `JSON.parse` of the complete text, so a
/// page that holds only part of that text must not repeat the whole parse:
/// it alone can exceed the response budget on every page. Scalars such as
/// `verdict` stay; the omitted keys are listed for a caller that pages the
/// text and parses it inside its program.
fn partial_structured(result: &mut Value) {
    let complete = result["range"]["offset"] == 0 && result["range"]["next"].is_null();
    if complete || result.get("range").is_none() {
        return;
    }
    let Some(structured) = result.get_mut("structured").and_then(Value::as_object_mut) else {
        return;
    };
    let omitted: Vec<String> = structured
        .iter()
        .filter(|(_, value)| value.is_object() || value.is_array())
        .map(|(key, _)| key.clone())
        .collect();
    if omitted.is_empty() {
        return;
    }
    structured.retain(|_, value| !value.is_object() && !value.is_array());
    result["structured_omitted"] = json!(omitted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_review_up_to_11k_chars_fits_in_one_page() {
        let text = "a".repeat(11_000);
        let mut result = json!({"text": text, "truncated": false});
        result_window(&mut result, &json!({"offset": 0, "chars": 12_000})).unwrap();
        assert_eq!(result["text"].as_str().unwrap().len(), 11_000);
        assert_eq!(
            result["range"],
            json!({"offset": 0, "next": null, "total": 11_000})
        );
        assert!(serde_json::to_vec(&result).unwrap().len() <= RESULT_JSON_BUDGET_BYTES);
    }

    #[test]
    fn cjk_and_escaped_text_auto_fit_and_reconstruct_losslessly() {
        for sample in [
            "中".repeat(12_000),
            "\"\\\n\t".repeat(4_000),
            "a中🙂\"\\n".repeat(3_000),
        ] {
            let mut offset = 0usize;
            let mut rebuilt = String::new();
            let mut pages = 0usize;
            loop {
                let mut result = json!({"text": sample, "truncated": false});
                result_window(&mut result, &json!({"offset": offset, "chars": 12_000})).unwrap();
                let serialized = serde_json::to_vec(&result).unwrap().len();
                assert!(
                    serialized <= RESULT_JSON_BUDGET_BYTES,
                    "page exceeded budget: {serialized}"
                );
                rebuilt.push_str(result["text"].as_str().unwrap());
                pages += 1;
                assert!(pages < 20, "paging did not converge");
                match result["range"]["next"].as_u64() {
                    Some(next) => {
                        assert!(next as usize > offset, "range.next must advance");
                        offset = next as usize;
                    }
                    None => break,
                }
            }
            assert_eq!(rebuilt, sample);
            assert!(pages >= 2);
        }
    }

    #[test]
    fn large_complete_structured_parse_shrinks_instead_of_exceeding_budget() {
        let findings: Vec<Value> = (0..180)
            .map(|i| json!({"title": format!("finding-{i}"), "detail": "x".repeat(30)}))
            .collect();
        let structured =
            json!({"review_complete": true, "verdict": "needs-attention", "findings": findings});
        let text = structured.to_string();
        assert!(text.len() > 8_000 && text.len() < 12_000);
        let mut result = json!({"text": text, "structured": structured, "truncated": false});
        result_window(&mut result, &json!({"offset": 0, "chars": 12_000})).unwrap();
        let serialized = serde_json::to_vec(&result).unwrap().len();
        assert!(
            serialized <= RESULT_JSON_BUDGET_BYTES,
            "serialized={serialized}"
        );
        assert!(result["range"]["next"].is_u64());
        assert_eq!(result["structured_omitted"], json!(["findings"]));
    }

    #[test]
    fn unwindowed_and_offset_only_defaults_remain_unchanged() {
        let text = "x".repeat(2_000);
        let mut unwindowed = json!({"text": text});
        assert_eq!(
            result_window(&mut unwindowed, &json!({})),
            Err("text_window_required".into())
        );
        let mut offset_only = json!({"text": text});
        result_window(&mut offset_only, &json!({"offset": 0})).unwrap();
        assert_eq!(offset_only["text"].as_str().unwrap().chars().count(), 1000);
        assert_eq!(offset_only["range"]["next"], 1000);
    }
}
