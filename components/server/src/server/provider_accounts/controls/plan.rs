//! The plan of a named-only account, as its Provider last reported it during
//! one of the account's sessions. The ACP proxy carries it on each
//! `usage_update` under `_meta.cowboy.account_plan` (`{id, name}`), which the
//! worker already relays and the Controller already persists as the
//! session's usage. Nothing is requested for it; it is informational only
//! and never gates admission, dispatch or revival. A value this module
//! cannot read is ignored.
use super::*;

const MAX_ID: usize = 64;
const MAX_NAME: usize = 80;

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

/// Printable text only, at most [`MAX_NAME`] code points: no control (`Cc`),
/// format (`Cf`), private-use (`Co`) or noncharacter code point and no line
/// or paragraph separator. The proxy and the Web apply the full
/// `[\p{C}\p{Zl}\p{Zp}]` rule with their Unicode tables; this dependency-free
/// copy additionally admits code points unassigned in Unicode, which the
/// proxy never reports. The proxy rejects a forbidden character anywhere and
/// then trims, so a name it reports is already trimmed.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME
        && !name.chars().any(forbidden_in_name)
}

/// The `Cf` code points of Unicode 16.
const FORMAT: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

fn forbidden_in_name(character: char) -> bool {
    let code = u32::from(character);
    character.is_control()
        || matches!(code, 0x2028 | 0x2029)
        || matches!(code, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
        || matches!(code, 0xFDD0..=0xFDEF)
        || code & 0xFFFE == 0xFFFE
        || FORMAT
            .iter()
            .any(|&(low, high)| (low..=high).contains(&code))
}

/// One session's reported plan and when it was observed.
fn reported(meta: &SessionMeta) -> Option<(i64, &str, &str)> {
    let usage = meta.usage.as_ref()?;
    let plan = usage.raw.pointer("/_meta/cowboy/account_plan")?;
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
