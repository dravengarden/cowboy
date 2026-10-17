//! Cached Service account observations, separate from per-session usage.
//! Keep Provider payloads intact: new Plugins need no MCP-specific parser.
use std::collections::BTreeSet;

use super::*;
use crate::plugin_host::PluginUsageSpec;
use crate::usage::{ProviderUsage, UsageSnapshot};

const PAGE_BYTES: usize = 32 * 1024;

pub(super) async fn read(
    state: &AppState,
    parent: &SessionMeta,
    op: &str,
    args: &Value,
) -> Result<Value, String> {
    // Account observations have the same product-user audience as /api/usage.
    // An unowned legacy session on an authenticated Service is not that grant.
    if !account_access(state.product_auth_enabled, parent.owner_user_id.as_deref()) {
        return Err("permission_denied".into());
    }
    let value = state
        .usage
        .cached_view(|snapshot, bindings, refreshing| {
            project(snapshot, bindings, refreshing, op, args)
        })
        .await?;
    actions::read_parent(&state.hub, state.store.as_ref(), parent).await?;
    Ok(value)
}

fn account_access(authenticated_service: bool, owner: Option<&str>) -> bool {
    !authenticated_service || owner.is_some()
}

fn header(
    binding: &PluginUsageSpec,
    usage: Option<&ProviderUsage>,
    refreshing: &BTreeSet<String>,
) -> Value {
    json!({
        "account": binding.account,
        "label": binding.product_label(),
        "status": usage.map_or("unavailable", |usage| usage.status),
        "source": usage.map(|usage| usage.source),
        "observed_at_ms": usage.map(|usage| usage.observed_at_ms).filter(|time| *time > 0),
        "refresh": usage.and_then(|usage| usage.refresh.as_ref()),
        "refreshing": refreshing.contains(&binding.account),
        "error": usage.and_then(|usage| usage.error.as_ref()),
    })
}

fn project(
    snapshot: &UsageSnapshot,
    bindings: &[PluginUsageSpec],
    refreshing: &BTreeSet<String>,
    op: &str,
    args: &Value,
) -> Result<Value, String> {
    let observation = |account: &str| {
        snapshot
            .providers
            .iter()
            .find(|usage| usage.provider == account)
    };
    if op == "usage.list" {
        only_keys(args, &["after", "limit"])?;
        let limit = page_limit(args, 20, 100)?;
        let after = after_id(args)?;
        let mut accounts: Vec<_> = bindings
            .iter()
            .filter(|binding| after.is_none_or(|after| binding.account.as_str() > after))
            .collect();
        accounts.sort_by(|left, right| left.account.cmp(&right.account));
        accounts.dedup_by(|left, right| left.account == right.account);
        let more = accounts.len() > limit;
        let accounts: Vec<_> = accounts.into_iter().take(limit).map(|binding| {
            let usage = observation(&binding.account);
            let mut value = header(binding, usage, refreshing);
            value["sections"] = json!({
                "account": usage.is_some_and(|usage| usage.account.as_ref().is_some_and(|v| !v.is_null())),
                "quota": usage.is_some_and(|usage| usage.rate_limits.as_ref().is_some_and(|v| !v.is_null())),
                "activity": usage.is_some_and(|usage| usage.activity.as_ref().is_some_and(|v| !v.is_null())),
            });
            value
        }).collect();
        let next = more.then(|| accounts.last().unwrap()["account"].clone());
        return Ok(json!({"accounts": accounts, "next": next,
            "next_refresh_at_ms": snapshot.next_refresh_at_ms,
            "refresh_interval_ms": snapshot.refresh_interval_ms}));
    }
    only_keys(
        args,
        &["account", "section", "path", "offset", "limit", "describe"],
    )?;
    let account = arg_str(args, "account")?;
    let binding = bindings
        .iter()
        .find(|binding| binding.account == account)
        .ok_or("not_found")?;
    let section = match args.get("section") {
        None | Some(Value::Null) => "quota",
        Some(Value::String(section))
            if matches!(section.as_str(), "quota" | "account" | "activity") =>
        {
            section
        }
        _ => return Err("invalid_argument".into()),
    };
    let usage = observation(account);
    let data = usage.and_then(|usage| match section {
        "quota" => usage.rate_limits.as_ref(),
        "account" => usage.account.as_ref(),
        _ => usage.activity.as_ref(),
    });
    let mut value = header(binding, usage, refreshing);
    value["section"] = json!(section);
    value["available"] = json!(data.is_some_and(|v| !v.is_null()));
    value["page"] = json_page(data, args)?;
    Ok(value)
}

fn json_page(data: Option<&Value>, args: &Value) -> Result<Value, String> {
    let path = match args.get("path") {
        None | Some(Value::Null) => "",
        Some(Value::String(path)) if valid_pointer(path) => path,
        _ => return Err("invalid_argument".into()),
    };
    let limit = page_limit(args, 50, 200)?;
    let offset =
        usize::try_from(arg_u64(args, "offset", 0, u64::MAX)?).map_err(|_| "invalid_argument")?;
    let describe = optional_bool(args, "describe")?;
    let value = match data {
        Some(value) => value.pointer(path).ok_or("not_found")?,
        None if path.is_empty() => &Value::Null,
        None => return Err("not_found".into()),
    };
    let (kind, total) = match value {
        Value::Array(items) => ("array", items.len()),
        Value::Object(items) => ("object", items.len()),
        _ => ("value", 1),
    };
    if offset > total || (kind == "value" && offset != 0) {
        return Err("invalid_cursor".into());
    }
    let mut budget = PAGE_BYTES;
    let data = match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .skip(offset)
                .take(limit)
                .map(|value| copy_value(value, describe, &mut budget))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(items) => {
            let mut entries: Vec<_> = items.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                entries
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|(key, value)| {
                        let mut counter = ByteBudget(budget);
                        serde_json::to_writer(&mut counter, key)
                            .map_err(|_| "selection_too_large")?;
                        budget = counter.0;
                        copy_value(value, describe, &mut budget).map(|value| (key.clone(), value))
                    })
                    .collect::<Result<_, _>>()?,
            )
        }
        _ => copy_value(value, describe, &mut budget)?,
    };
    // Account for object keys and framing too, including malicious long keys.
    serde_json::to_writer(ByteBudget(PAGE_BYTES), &data).map_err(|_| "selection_too_large")?;
    let end = offset.saturating_add(limit).min(total);
    Ok(
        json!({"path":path,"kind":kind,"describe":describe,"data":data,
        "range":{"offset":offset,"next":(end < total).then_some(end),"total":total}}),
    )
}

fn copy_value(value: &Value, describe: bool, budget: &mut usize) -> Result<Value, String> {
    let description;
    let value = if describe {
        description = describe_value(value);
        &description
    } else {
        value
    };
    // Check before cloning a large descendant; describe reveals its shape.
    let mut counter = ByteBudget(*budget);
    serde_json::to_writer(&mut counter, value).map_err(|_| "selection_too_large".to_owned())?;
    *budget = counter.0;
    Ok(value.clone())
}

fn valid_pointer(path: &str) -> bool {
    if path.len() > 512 || (!path.is_empty() && !path.starts_with('/')) {
        return false;
    }
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
            return false;
        }
    }
    true
}

fn describe_value(value: &Value) -> Value {
    match value {
        Value::Object(items) => json!({"type":"object","length":items.len()}),
        Value::Array(items) => json!({"type":"array","length":items.len()}),
        Value::String(text) => json!({"type":"string","bytes":text.len()}),
        Value::Number(_) => json!({"type":"number"}),
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Null => json!({"type":"null"}),
    }
}

struct ByteBudget(usize);
impl std::io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("page budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
