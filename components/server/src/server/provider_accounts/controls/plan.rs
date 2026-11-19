//! The plan of a named-only account, as its Provider last reported it during
//! one of the account's sessions. The ACP proxy carries it on each
//! `usage_update` under `_meta.cowboy` (`{sanitizer: 1, account_plan: {id,
//! name}}`), which the worker already relays and the Controller already persists as the
//! session's usage. Nothing is requested for it; it is informational only
//! and never gates admission, dispatch or revival. A value this module
//! cannot read is ignored.
use super::*;

mod unprintable;

const MAX_ID: usize = 64;
const MAX_NAME: usize = 80;
/// The `_meta.cowboy.sanitizer` generation of the Antigravity proxy whose
/// usage readings carry a plan it validated itself.
const SANITIZER: u64 = 1;

fn valid_id(id: &str) -> bool {
    id.len() <= MAX_ID
        && id
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

/// Printable text only, 1 to [`MAX_NAME`] code points, already trimmed: no
/// character of Unicode category C (control, format, surrogate, private use,
/// unassigned) and no line or paragraph separator. This is the proxy's and
/// the Web's `[\p{C}\p{Zl}\p{Zp}]` rule, as the table generated from it for
/// the Unicode version recorded in `plan/unprintable.rs`. The proxy rejects a
/// forbidden character anywhere and then trims, so a name it reports is
/// already trimmed.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME
        && !name.chars().any(unprintable)
}

fn unprintable(character: char) -> bool {
    let code = u32::from(character);
    unprintable::RANGES
        .binary_search_by(|&(low, high)| {
            if high < code {
                std::cmp::Ordering::Less
            } else if low > code {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// One session's reported plan and when it was observed.
fn reported(meta: &SessionMeta) -> Option<(i64, &str, &str)> {
    let usage = meta.usage.as_ref()?;
    let cowboy = usage.raw.pointer("/_meta/cowboy")?;
    // Only a proxy that rebuilds `_meta.cowboy` itself stamps this marker. An
    // earlier proxy forwarded `agy`'s `_meta` verbatim, so a plan without
    // the marker (a stored reading or a still-pinned worker) is ignored.
    if cowboy.get("sanitizer") != Some(&json!(SANITIZER)) {
        return None;
    }
    let plan = cowboy.get("account_plan")?;
    let id = plan.get("id")?.as_str().filter(|id| valid_id(id))?;
    let name = plan.get("name")?.as_str().filter(|name| valid_name(name))?;
    Some((usage.observed_at_ms, id, name))
}

/// The newest plan any governed session of `account` reported, or `null`.
pub(super) fn account_plan(sessions: &[SessionMeta], account: &ProviderAccountId) -> Value {
    sessions
        .iter()
        .filter(|meta| governed(meta) && meta.provider_account_id == *account)
        .filter_map(reported)
        .max_by_key(|(observed, _, _)| *observed)
        .map_or(
            Value::Null,
            |(observed, id, name)| json!({"id": id, "name": name, "observed_at_ms": observed}),
        )
}
