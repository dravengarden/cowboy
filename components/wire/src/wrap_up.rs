//! Shared Antigravity wrap-up steering wire constants, capability clamping,
//! and cross-runtime (`COWBOY_WS`) prompt classification helpers.

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

/// Machine WebSocket protocol version that introduces `CoreCommand::WrapUpHint`
/// and outer-to-broker `outer_machine_protocol` propagation.
pub const WRAP_UP_STEERING_PROTOCOL_VERSION: u16 = 53;
/// Runtime IPC protocol version that introduces `WorkerSnapshot::wrap_up_steering`,
/// `Frame::Hello::outer_machine_protocol`, `CoreCommand::WrapUpHint`, and
/// `WorkerCommand::WrapUpHint`.
pub const WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION: u16 = 8;
/// Maximum serialized byte size of a single `WrapUpHint` frame.
pub const MAX_WRAP_UP_HINT_BYTES: usize = 512;
/// Minimum signed Antigravity Plugin release that implements R1 dual metadata
/// stripping, `attemptNonce` stripping, and root wrap-up hint attachment.
pub const ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION: &str = "0.2.35";
/// Top-level ACP `initialize` `_meta` key stamped by P2 `agy-proxy`.
pub const WRAP_UP_STEERING_CAPABILITY_KEY: &str = "cowboy.dev/wrapUpSteering";
/// Per-attempt nonce key injected into `session/prompt` `params._meta` only
/// for verified P2 Antigravity launches.
pub const ATTEMPT_NONCE_META_KEY: &str = "cowboy.dev/attemptNonce";

#[must_use]
pub fn is_antigravity_provider(provider: &str) -> bool {
    provider == "antigravity"
}

/// Strict `MAJOR.MINOR.PATCH` comparison (no prerelease or trailing suffix).
#[must_use]
pub fn semver_at_least(version: &str, minimum: &str) -> bool {
    fn parse_triple(raw: &str) -> Option<(u64, u64, u64)> {
        let mut parts = raw.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some((major, minor, patch))
    }
    match (parse_triple(version), parse_triple(minimum)) {
        (Some(v), Some(m)) => v >= m,
        _ => false,
    }
}

/// Closed cross-runtime whitespace set shared identically with JS
/// (`antigravity-acp/compaction.mjs:isCowboyWs`). Explicitly includes `U+FEFF`
/// and excludes `U+0085` (NEL).
#[must_use]
pub fn is_cowboy_ws(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

#[must_use]
pub fn trim_cowboy_ws(text: &str) -> &str {
    text.trim_matches(is_cowboy_ws)
}

#[must_use]
pub fn trim_start_cowboy_ws(text: &str) -> &str {
    text.trim_start_matches(is_cowboy_ws)
}

/// Whether `text` is an ASCII case-insensitive `/compact` command under
/// `COWBOY_WS` trimming.
#[must_use]
pub fn is_compaction_prompt_text(text: &str) -> bool {
    let trimmed = trim_cowboy_ws(text);
    let Some(rest) = trimmed
        .get(..8)
        .filter(|prefix| prefix.eq_ignore_ascii_case("/compact"))
        .map(|_| &trimmed[8..])
    else {
        return false;
    };
    rest.is_empty() || rest.chars().next().is_some_and(is_cowboy_ws)
}

/// Concatenate all text blocks iff `blocks` is non-empty and every block has
/// `type == "text"` with a string `text` field.
fn all_text_concat(blocks: &[serde_json::Value]) -> Option<String> {
    if blocks.is_empty() {
        return None;
    }
    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(serde_json::Value::as_str) != Some("text") {
            return None;
        }
        out.push_str(block.get("text").and_then(serde_json::Value::as_str)?);
    }
    Some(out)
}

/// Whether ACP JSON content `blocks` represent a native `/compact` prompt.
#[must_use]
pub fn is_compaction_prompt_blocks(blocks: &[serde_json::Value]) -> bool {
    all_text_concat(blocks).is_some_and(|text| is_compaction_prompt_text(&text))
}

/// Unified `COWBOY_WS` slash-command classifier for raw text (matches
/// `session_context::is_command` with closed `COWBOY_WS` trimming and boundary).
#[must_use]
pub fn is_command_text_cowboy_ws(text: &str) -> bool {
    let Some(after_slash) = trim_start_cowboy_ws(text).strip_prefix('/') else {
        return false;
    };
    let name = after_slash.split(is_cowboy_ws).next().unwrap_or("");
    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
}

/// Unified `COWBOY_WS` slash-command classifier over ACP content blocks:
/// recognizes both multi-block `/compact` (including split fragments) and
/// leading-text slash commands.
#[must_use]
pub fn is_command_blocks_cowboy_ws(blocks: &[serde_json::Value]) -> bool {
    if is_compaction_prompt_blocks(blocks) {
        return true;
    }
    if let Some(concat) = all_text_concat(blocks) {
        return is_command_text_cowboy_ws(&concat);
    }
    blocks
        .iter()
        .find_map(|block| {
            (block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(serde_json::Value::as_str))
                .flatten()
        })
        .is_some_and(is_command_text_cowboy_ws)
}

/// Returns `true` iff `blocks` contains at least one non-text block (such as
/// an image/resource) OR at least one text block with a non-`COWBOY_WS` char.
#[must_use]
pub fn has_substantive_prompt_blocks(blocks: &[serde_json::Value]) -> bool {
    blocks.iter().any(|block| {
        if block.get("type").and_then(serde_json::Value::as_str) == Some("text") {
            block
                .get("text")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| !trim_cowboy_ws(text).is_empty())
        } else {
            true
        }
    })
}

/// Hard-clamps native mid-turn delivery capabilities for Antigravity, which
/// never supports native `current_turn` or `next_checkpoint` steering.
/// Fail-closed delivery capability clamp: clears native current-turn and
/// next-checkpoint steering whenever provider identity is missing (`None`) or
/// is Antigravity.
pub fn clamp_delivery_capabilities_for_provider(
    provider: Option<&str>,
    supports_current_turn: &mut bool,
    next_checkpoint: &mut bool,
) {
    if provider.is_none_or(is_antigravity_provider) {
        *supports_current_turn = false;
        *next_checkpoint = false;
    }
}

/// Validate a caller-supplied authored wrap-up operation ID against the Rev 10
/// contract: non-empty, `<= 128` bytes, ASCII `[A-Za-z0-9._:-]`.
#[must_use]
pub fn is_valid_wrap_up_operation_id(op: &str) -> bool {
    !op.is_empty()
        && op.len() <= 128
        && op
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// Effective `wrap_up_steering` gate across Controller, Machine broker, and
/// Worker hops: fails closed if provider is unknown or not Antigravity, if
/// outer Machine protocol < 53, if runtime protocol < 8, or if host config
/// disables wrap-up steering.
#[must_use]
pub fn effective_wrap_up_steering(
    provider: Option<&str>,
    raw_wrap_up: bool,
    outer_machine_protocol: Option<u16>,
    runtime_protocol: u16,
    config_enabled: bool,
) -> bool {
    raw_wrap_up
        && config_enabled
        && provider.is_some_and(is_antigravity_provider)
        && outer_machine_protocol.is_some_and(|p| p >= WRAP_UP_STEERING_PROTOCOL_VERSION)
        && runtime_protocol >= WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION
}

/// Validates that a serialized `WrapUpHint` runtime frame fits inside
/// [`MAX_WRAP_UP_HINT_BYTES`] (512 bytes) before enqueueing on any hint lane.
#[must_use]
pub fn wrap_up_hint_within_size_limit<T: serde::Serialize>(frame: &T) -> bool {
    serde_json::to_vec(frame).is_ok_and(|bytes| bytes.len() <= MAX_WRAP_UP_HINT_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct VectorFile {
        compaction_cases: Vec<VectorCase>,
        substantive_cases: Vec<VectorCase>,
    }

    #[derive(serde::Deserialize)]
    struct VectorCase {
        name: String,
        blocks: Vec<serde_json::Value>,
        expected: bool,
    }

    #[test]
    fn shared_cowboy_ws_vectors_match_contract() {
        let raw = include_str!("../../../contracts/cowboy-ws-compaction-vectors.json");
        let vectors: VectorFile = serde_json::from_str(raw).expect("valid vectors json");
        for case in vectors.compaction_cases {
            assert_eq!(
                is_compaction_prompt_blocks(&case.blocks),
                case.expected,
                "compaction vector {}",
                case.name
            );
        }
        for case in vectors.substantive_cases {
            assert_eq!(
                has_substantive_prompt_blocks(&case.blocks),
                case.expected,
                "substantive vector {}",
                case.name
            );
        }
    }

    #[test]
    fn semver_check_enforces_exact_numeric_triples() {
        assert!(semver_at_least(
            "0.2.35",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
        assert!(semver_at_least(
            "0.2.36",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
        assert!(semver_at_least(
            "0.3.0",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
        assert!(semver_at_least(
            "1.0.0",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
        assert!(!semver_at_least(
            "0.2.34",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
        assert!(!semver_at_least(
            "0.2.35-rc.1",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
        assert!(!semver_at_least(
            "",
            ANTIGRAVITY_WRAP_UP_MIN_PROVIDER_VERSION
        ));
    }

    #[test]
    fn antigravity_delivery_clamping_and_protocol_gate_fail_closed() {
        let mut current = true;
        let mut next_cp = true;
        clamp_delivery_capabilities_for_provider(Some("antigravity"), &mut current, &mut next_cp);
        assert!(!current);
        assert!(!next_cp);

        let mut codex_current = true;
        let mut codex_cp = true;
        clamp_delivery_capabilities_for_provider(Some("codex"), &mut codex_current, &mut codex_cp);
        assert!(codex_current);
        assert!(codex_cp);

        assert!(effective_wrap_up_steering(
            Some("antigravity"),
            true,
            Some(53),
            8,
            true
        ));
        assert!(!effective_wrap_up_steering(
            Some("antigravity"),
            true,
            Some(52),
            8,
            true
        ));
        assert!(!effective_wrap_up_steering(
            Some("antigravity"),
            true,
            None,
            8,
            true
        ));
        assert!(!effective_wrap_up_steering(
            Some("antigravity"),
            true,
            Some(53),
            7,
            true
        ));
        assert!(!effective_wrap_up_steering(
            Some("antigravity"),
            true,
            Some(53),
            8,
            false
        ));
        assert!(!effective_wrap_up_steering(
            Some("codex"),
            true,
            Some(53),
            8,
            true
        ));
        assert!(!effective_wrap_up_steering(None, true, Some(53), 8, true));
        let mut missing_current = true;
        let mut missing_checkpoint = true;
        clamp_delivery_capabilities_for_provider(
            None,
            &mut missing_current,
            &mut missing_checkpoint,
        );
        assert!(!missing_current);
        assert!(!missing_checkpoint);
        assert!(is_valid_wrap_up_operation_id("op-123_a.b:c"));
        assert!(!is_valid_wrap_up_operation_id(""));
        assert!(!is_valid_wrap_up_operation_id(&"a".repeat(129)));
        assert!(!is_valid_wrap_up_operation_id("bad space"));
        assert!(!is_valid_wrap_up_operation_id("bad/slash"));
    }

    #[test]
    fn wrap_up_hint_size_limit_rejects_oversized_frames() {
        let normal = serde_json::json!({
            "type": "core_command",
            "command": {"command": "wrap_up_hint", "session_id": "sess-123", "target": {"worker_epoch": "epoch-1", "turn_id": "turn-1"}}
        });
        assert!(wrap_up_hint_within_size_limit(&normal));
        let huge = serde_json::json!({
            "type": "core_command",
            "command": {"command": "wrap_up_hint", "session_id": "s".repeat(600), "target": {"worker_epoch": "epoch-1", "turn_id": "turn-1"}}
        });
        assert!(!wrap_up_hint_within_size_limit(&huge));
    }
}
