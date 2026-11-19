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

/// Printable text only: no character of the Unicode general category
/// `Other` (`Cc` control, `Cf` format, `Cs` surrogate, `Co` private use, `Cn`
/// unassigned) and no line or paragraph separator, at most [`MAX_NAME`] code
/// points. The proxy and the Web apply the same rule (`[\p{C}\p{Zl}\p{Zp}]`).
/// The proxy rejects a forbidden character anywhere and then trims, so a
/// name it reports is already trimmed; an untrimmed one is not its output.
fn valid_name(name: &str) -> bool {
    use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory as _};
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME
        && !name.chars().any(|character| {
            character.general_category_group() == GeneralCategoryGroup::Other
                || matches!(
                    character.general_category(),
                    GeneralCategory::LineSeparator | GeneralCategory::ParagraphSeparator
                )
        })
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
