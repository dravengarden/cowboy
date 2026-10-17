//! Cached Service account observations, separate from per-session usage.
//! Keep Provider payloads intact: new Plugins need no MCP-specific parser.
use std::collections::BTreeSet;

use super::*;
use crate::plugin_host::PluginUsageSpec;
use crate::usage::{ProviderUsage, UsageSnapshot};

use super::json_reads::page as json_page;

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

#[cfg(test)]
mod tests;
