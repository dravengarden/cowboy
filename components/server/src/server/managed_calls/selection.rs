//! Evidence for the calling model's decision. No quota thresholds or silent
//! model fallback: the agent chooses a signed preset within session authority.
use crate::usage::UsageSnapshot;
use serde_json::{Value, json};

mod routing;

pub(super) const GUIDANCE: &str = "For Review/Design Review, the Controller picks the reviewer from providers[].review_routing by declared risk; cost matters. Declare risk from the review root's own guidance (its AGENTS.md/CLAUDE.md Review risk section) and the affected paths: risk:'high' (a change that can lose or corrupt durable data, weaken credentials, authentication or account isolation, or touch money and usage limits, or a risk area that guidance names; size, concurrency or a wide refactor alone is routine) needs a reason and gets Sol Max (Claude Opus). Everything else is routine (Sol Max; Claude Opus). Astra Max and Fable cost about twice Sol Max and are never chosen automatically: name them only when the user asked for that model, with risk:'high' and that request as the reason; do not pick them yourself for a high-risk change. Prefer another vendor's reviewer (an Auto call tries other vendors first); the caller's own vendor is allowed, but a review never runs the caller's own model (same_model). Omit preset unless the user named one; a routine review refuses a stronger preset (preset_outside_tier), and a user-named stronger preset is sent with risk:'high' and that request as its reason. A fix-confirmation round is routine: continue the same review child when it runs a routine-tier preset; a child on a stronger preset (for example Astra Max) refuses a routine round (preset_outside_tier), so start a fresh routine review with the same task and base (it reuses the captured input). Continue with risk:'high' only when the fixes themselves changed a high-risk design, not because the first round was high. A review child takes at most three completed rounds (review_rounds_exhausted): findings that survive them call for redesigning that mechanism, not another round. Evidence and test-run audits are analysis calls, not reviews; design and contract checks stay design reviews. Costs are measured tokens, not quota; inspect quota windows and reset times before choosing a Provider. review_candidates is the routine order for older callers. No silent fallback after a failed call: diagnose before retrying. usedPercent is USED; resetsAt is epoch seconds. Large quota payloads: use cowboy.usage.read.";

pub(super) fn family(model: &str) -> Option<&'static str> {
    // Signed presets may carry a context suffix such as `opus[1m]`.
    let model = model.split_once('[').map_or(model, |(base, _)| base);
    if model == "opus" || model.starts_with("claude-opus-") {
        Some("opus")
    } else if model == "fable" || model.starts_with("claude-fable-") {
        Some("fable")
    } else if model.starts_with("gpt-") && model.ends_with("-astra") {
        Some("astra")
    } else if model.starts_with("gpt-") && model.ends_with("-sol") {
        Some("sol")
    } else {
        None
    }
}

/// The preset a fresh review runs: the explicit one if its tier admits it,
/// otherwise the tier's automatic choice. `reviewers` is every preset the
/// session's family lists admit, so a known reviewer outside the tier gets
/// its own refusal.
pub(super) fn review_preset(
    choices: &[Value],
    reviewers: &[Value],
    explicit: Option<&str>,
) -> Result<String, &'static str> {
    let named =
        |list: &[Value], id: &str| list.iter().any(|choice| choice["id"].as_str() == Some(id));
    if let Some(id) = explicit {
        if named(choices, id) {
            return Ok(id.to_owned());
        }
        return Err(if named(reviewers, id) {
            "preset_outside_tier"
        } else {
            "review_model_not_allowed"
        });
    }
    choices
        .first()
        .and_then(|choice| choice["id"].as_str())
        .map(str::to_owned)
        .ok_or("preset_unavailable")
}

/// Fresh-review presets for a declared risk: the automatic choice first, then
/// the other presets the tier admits (docs/review-routing.md). A Provider or
/// generation the routing table does not cover keeps the legacy order.
#[cfg(test)]
pub(super) fn candidates(
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
) -> Vec<Value> {
    ranked(presets, presets, provider, preference, high_risk).list
}

/// `candidates` for a caller: `runnable` is the whole generation (coverage),
/// filtered here by the caller's own model.
pub(super) fn candidates_for(
    runnable: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
    (same_vendor, caller): (bool, Option<Option<&str>>),
) -> routing::Ranked {
    let presets = exclude_caller_model(runnable, same_vendor, caller);
    ranked(&presets, runnable, provider, preference, high_risk)
}

fn ranked(
    presets: &Value,
    coverage: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
) -> routing::Ranked {
    use super::super::agent_tools::ReviewEffort;
    let policy = if provider == "codex" {
        preference.gpt_effort
    } else {
        preference.claude_effort
    };
    let level = if high_risk {
        routing::Level::High
    } else if policy == ReviewEffort::Standard {
        routing::Level::Economy
    } else {
        routing::Level::Routine
    };
    let pool = legacy(presets, provider, preference, true);
    let coverage = coverage.as_array().map_or(&[][..], Vec::as_slice);
    routing::TABLE
        .as_ref()
        .and_then(|table| routing::order(table, provider, level, &pool, coverage))
        .unwrap_or_else(|| {
            let list = legacy(presets, provider, preference, high_risk);
            routing::Ranked {
                automatic: !list.is_empty(),
                list,
            }
        })
}

/// What a session's current model is, for excluding it from its reviews:
/// `None` when unknown, `Some(None)` for a known model that is no reviewer
/// family (for example Sonnet), `Some(Some(family))` otherwise.
pub(super) fn caller_family(configuration: Option<&Value>) -> Option<Option<&'static str>> {
    configuration?
        .as_array()?
        .iter()
        .find(|option| option["id"] == "model")?
        .get("currentValue")?
        .as_str()
        .filter(|model| !model.is_empty() && *model != "default")
        .map(family)
}

/// Whether a reviewer of `reviewer` family would review the caller with its
/// own model. Any Provider counts (another vendor's package can run the same
/// model); an unknown caller model conflicts with every same-vendor reviewer.
pub(super) fn same_model(
    same_vendor: bool,
    caller: Option<Option<&str>>,
    reviewer: Option<&str>,
) -> bool {
    match caller {
        Some(Some(caller)) => reviewer == Some(caller),
        Some(None) => false,
        None => same_vendor,
    }
}

/// Whether a parent's live configuration shows it now runs `reviewer`'s
/// model. An unknown model is no evidence for a fresh review (admission
/// checked it), but a same-vendor continuation was never checked, so there
/// it counts as the same model.
pub(super) fn runs_now(
    configuration: Option<&Value>,
    reviewer: Option<&str>,
    unchecked_same_vendor: bool,
) -> bool {
    match caller_family(configuration) {
        Some(caller) => same_model(false, Some(caller), reviewer),
        None => unchecked_same_vendor,
    }
}

/// Runnable presets without the caller's own model: a review by the same
/// model shares its blind spots.
pub(super) fn exclude_caller_model(
    presets: &Value,
    same_vendor: bool,
    caller: Option<Option<&str>>,
) -> Value {
    Value::Array(
        presets
            .as_array()
            .into_iter()
            .flatten()
            .filter(|preset| {
                let own = preset["values"]["model"].as_str().and_then(family);
                !same_model(same_vendor, caller, own)
            })
            .cloned()
            .collect(),
    )
}

/// Whether another vendor's package (Antigravity's Claude Opus) lost its
/// reviewer only to the caller's own model: of the tier's `eligible` choices
/// before that exclusion, the named one runs it, or the tier is now empty
/// although an eligible choice ran it.
fn cross_vendor_same_model(
    eligible: &[Value],
    choices: &[Value],
    explicit: Option<&str>,
    caller: Option<Option<&str>>,
) -> bool {
    let presets = eligible;
    let own = |preset: &Value| {
        same_model(
            false,
            caller,
            preset["values"]["model"].as_str().and_then(family),
        )
    };
    match explicit {
        Some(id) => presets
            .iter()
            .any(|preset| preset["id"].as_str() == Some(id) && own(preset)),
        None => choices.is_empty() && presets.iter().any(own),
    }
}

/// A fresh review's preset at admission, or the refusal to return. A high
/// tier needs a reason the caller wrote; the CLI fills an omitted reason
/// with its own default, which is no reason.
pub(super) fn admit(
    request: &crate::managed_calls::Request,
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    explicit: Option<&str>,
    (same_vendor, caller): (bool, Option<Option<&str>>),
) -> Result<String, Value> {
    let runnable = presets;
    let presets = &exclude_caller_model(runnable, same_vendor, caller);
    let risk = request
        .labels
        .get(crate::managed_calls::review::RISK)
        .map_or("undeclared", String::as_str);
    let high_risk = risk == "high";
    let decided = |outcome: &str, preset: Option<&str>| {
        // Journal-retained (not evidence-only): routing is low volume and its
        // refusals leave no ledger row, so this is their only record.
        tracing::info!(
            event_name = "cowboy.review.routed",
            request_id = %request.request_id,
            purpose = ?request.purpose,
            provider,
            same_vendor,
            risk,
            explicit_preset = explicit.unwrap_or(""),
            preset = preset.unwrap_or(""),
            outcome,
            "review routed"
        );
    };
    let refuse = |code: &str, hint: String| {
        decided(code, None);
        let mut refused = super::coordinator::error(code, "not_submitted");
        if !hint.is_empty() {
            refused["error"]["hint"] = json!(hint);
        }
        refused
    };
    if high_risk && missing_reason(request) {
        return Err(refuse(
            "risk_reason_required",
            "risk:'high' needs a reason naming the risk (for example persistence, permissions or a risk area in the review root's guidance)".into(),
        ));
    }
    let tier = candidates_for(
        runnable,
        provider,
        preference,
        high_risk,
        (same_vendor, caller),
    );
    let choices = tier.list;
    let all = reviewers(presets, provider, preference);
    let unnamed = explicit.is_none() && !tier.automatic;
    if same_vendor && (all.is_empty() || choices.is_empty() || unnamed) {
        return Err(refuse(
            "same_model",
            "a review never runs the caller's own model and this tier has no other model of this vendor; use another vendor's reviewer, or declare risk:'high' with a reason to name a stronger model of this vendor".into(),
        ));
    }
    // The tier before the caller-model exclusion (a known non-reviewer
    // caller excludes nothing).
    let eligible = candidates_for(
        runnable,
        provider,
        preference,
        high_risk,
        (false, Some(None)),
    );
    // Unnamed, the exclusion may also remove only the tier's automatic choice.
    let lost_automatic = explicit.is_none() && eligible.automatic && !tier.automatic;
    // Named, the exclusion removed that preset; unnamed, it removed only the
    // tier's automatic choice (an empty explicit-only tier is no evidence).
    let lost = match explicit {
        Some(_) => cross_vendor_same_model(&eligible.list, &choices, explicit, caller),
        None => lost_automatic && cross_vendor_same_model(&eligible.list, &[], None, caller),
    };
    if !same_vendor && lost {
        return Err(refuse(
            "same_model",
            "a review never runs the caller's own model, even through another vendor's package"
                .into(),
        ));
    }
    if unnamed {
        return Err(refuse("preset_unavailable", String::new()));
    }
    match review_preset(&choices, &all, explicit) {
        Ok(preset) => {
            decided(
                if explicit.is_some() {
                    "explicit"
                } else {
                    "automatic"
                },
                Some(&preset),
            );
            Ok(preset)
        }
        Err(code) => {
            let hint = if code == "preset_outside_tier" {
                format!(
                    "this preset is stronger than a routine review needs; omit preset (automatic: {}), name a cheaper one, or declare risk:'high' with a reason",
                    choices
                        .first()
                        .and_then(|choice| choice["id"].as_str())
                        .unwrap_or("none")
                )
            } else {
                String::new()
            };
            Err(refuse(code, hint))
        }
    }
}

/// A continued review keeps its child's preset, but the round still has a
/// tier: the risk it declares now (an undeclared confirmation round is
/// routine). A known reviewer stronger than that tier is refused so the
/// caller starts a fresh review on the tier's choice, which reuses the
/// captured input; an unrouted or custom preset keeps running.
pub(super) fn admit_continuation(
    request: &crate::managed_calls::Request,
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    (child_preset, rounds): (Option<&str>, u64),
    same_vendor: bool,
) -> Result<(), Value> {
    let risk = request
        .labels
        .get(crate::managed_calls::review::RISK)
        .map_or("undeclared", String::as_str);
    let high_risk = risk == "high";
    let decided = |outcome: &str| {
        tracing::info!(
            event_name = "cowboy.review.routed",
            request_id = %request.request_id,
            purpose = ?request.purpose,
            provider,
            same_vendor,
            risk,
            explicit_preset = "",
            preset = child_preset.unwrap_or(""),
            rounds,
            outcome,
            "review routed"
        );
    };
    let refuse = |code: &str, hint: String| {
        decided(code);
        let mut refused = super::coordinator::error(code, "not_submitted");
        refused["error"]["hint"] = json!(hint);
        refused
    };
    if rounds >= MAX_REVIEW_ROUNDS {
        return Err(refuse(
            "review_rounds_exhausted",
            format!(
                "this review child already completed {rounds} rounds; stop and redesign the mechanism its findings keep hitting instead of another round, then start a fresh routine review of the redesigned change"
            ),
        ));
    }
    if high_risk && missing_reason(request) {
        return Err(refuse(
            "risk_reason_required",
            "risk:'high' needs a reason naming the risk (for example persistence, permissions or a risk area in the review root's guidance)".into(),
        ));
    }
    let tier = candidates_for(
        presets,
        provider,
        preference,
        high_risk,
        (false, Some(None)),
    )
    .list;
    let named =
        |list: &[Value], id: &str| list.iter().any(|choice| choice["id"].as_str() == Some(id));
    if let Some(id) = child_preset
        && !named(&tier, id)
        && named(&reviewers(presets, provider, preference), id)
    {
        return Err(refuse(
            "preset_outside_tier",
            format!(
                "this review child runs {id}, stronger than a routine confirmation round needs; start a fresh review on {} (the same task and base reuse the captured input), or continue with risk:'high' and a reason when the fixes changed a high-risk area",
                tier.first()
                    .and_then(|choice| choice["id"].as_str())
                    .unwrap_or("the routine tier")
            ),
        ));
    }
    decided("continued");
    Ok(())
}

/// Completed rounds one review child may run before a further continuation
/// is refused: findings that survive three rounds point at a design to
/// rework, not another confirmation pass.
const MAX_REVIEW_ROUNDS: u64 = 3;

/// A high tier needs a reason the caller wrote; the CLI fills an omitted
/// reason with its own default, which is no reason.
fn missing_reason(request: &crate::managed_calls::Request) -> bool {
    request
        .labels
        .get("selection_reason")
        .is_none_or(|reason| reason.trim().is_empty() || reason == "Session Tools selection")
}

/// Every reviewer preset the session's family lists admit, strongest first.
pub(super) fn reviewers(
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
) -> Vec<Value> {
    legacy(presets, provider, preference, true)
}

/// What the caller sees of the tiers: each tier's automatic preset and the
/// cheaper presets a routine review may name, with measured costs.
pub(super) fn routing_view(
    runnable: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    caller: (bool, Option<Option<&str>>),
) -> Value {
    let Some(table) = routing::TABLE.as_ref() else {
        return Value::Null;
    };
    let entry =
        |preset: &Value| json!({"preset": preset["id"], "cost": routing::cost(table, preset)});
    let high = candidates_for(runnable, provider, preference, true, caller);
    let high = if high.automatic {
        high.list
    } else {
        Vec::new()
    };
    let routine = candidates_for(runnable, provider, preference, false, caller);
    let routine = if routine.automatic {
        routine.list
    } else {
        Vec::new()
    };
    json!({
        "evidence": table.evidence,
        "high": high.first().map(entry),
        "routine": routine.first().map(entry),
        "cheaper": routine.iter().skip(1).map(entry).collect::<Vec<_>>(),
    })
}

/// The family/effort order that predates the routing table.
/// The reviewer families `provider` may review with, in the session's order.
/// Antigravity reviews only with Claude Opus (its named account's Claude
/// quota), whatever else a signed generation's presets offer.
pub(super) fn reviewer_families(
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
) -> Vec<String> {
    match provider {
        "codex" => preference.gpt.clone(),
        "antigravity" => preference
            .claude
            .iter()
            .filter(|family| *family == "opus")
            .cloned()
            .collect(),
        _ => preference.claude.clone(),
    }
}

fn legacy(
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
) -> Vec<Value> {
    use super::super::agent_tools::ReviewEffort;
    let order = reviewer_families(provider, preference);
    let effort_policy = if provider == "codex" {
        preference.gpt_effort
    } else {
        preference.claude_effort
    };
    // High-risk reviews keep the strongest-first order whatever the policy.
    let standard = !high_risk && effort_policy == ReviewEffort::Standard;
    let mut choices: Vec<_> = presets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|preset| {
            let family = family(preset["values"]["model"].as_str()?)?;
            let rank = order.iter().position(|v| v == family)?;
            // Codex, Claude and Antigravity name the reasoning option apart.
            let effort = preset["values"]["reasoning_effort"]
                .as_str()
                .or(preset["values"]["effort"].as_str())
                .or(preset["values"]["thought_level"].as_str())
                .unwrap_or("default");
            let effort_rank = match (standard, effort) {
                (false, "max") | (true, "medium") => 0,
                (false, "xhigh") | (true, "default") => 1,
                (_, "high") => 2,
                (false, "default") | (true, "xhigh") => 3,
                (false, "medium") | (true, "max") => 4,
                _ => 5,
            };
            Some((rank, effort_rank, preset.clone()))
        })
        .collect();
    choices.sort_by_key(|(rank, effort, _)| (*rank, *effort));
    choices.into_iter().map(|(_, _, preset)| preset).collect()
}

pub(super) fn quota(snapshot: &UsageSnapshot, accounts: &[String], now: i64) -> Value {
    let missing: Vec<_> = accounts
        .iter()
        .filter(|account| {
            !snapshot
                .providers
                .iter()
                .any(|usage| usage.provider == account.as_str())
        })
        .collect();
    let accounts: Vec<_> = snapshot.providers.iter().filter(|usage| accounts.iter().any(|account| account == usage.provider)).take(20).map(|usage| {
        let limits = usage.rate_limits.as_ref();
        let bounded = limits.filter(|value| serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 8192));
        json!({
            "account": usage.provider,
            "status": usage.status,
            "observed_at_ms": (usage.observed_at_ms > 0).then_some(usage.observed_at_ms),
            "stale": usage.observed_at_ms <= 0 || now.saturating_sub(usage.observed_at_ms) > snapshot.refresh_interval_ms.max(60_000) * 2 || usage.refresh.as_ref().is_none_or(|refresh| refresh.stale),
            "limits": bounded,
            "needs_read": limits.is_some() && bounded.is_none(),
        })
    }).collect();
    json!({"observed_now_ms":now,"accounts":accounts,"unavailable_accounts":missing})
}

/// The quota evidence of a named account, from its owner-scoped usage read
/// (`provider_accounts::usage::read_one`). A child runs on this account, so
/// the Provider-wide legacy collector never stands in for it.
pub(super) fn named_quota(read: &Value, now: i64, interval_ms: i64) -> Value {
    let usage = &read["usage"];
    let observed = usage["observed_at_ms"].as_i64().unwrap_or(0);
    let limits = usage.get("rate_limits").filter(|value| !value.is_null());
    let bounded =
        limits.filter(|value| serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 8192));
    json!({
        "account": read["account_id"],
        "provider_id": read["provider_id"],
        "status": usage["status"].as_str().unwrap_or("unavailable"),
        "observed_at_ms": (observed > 0).then_some(observed),
        "stale": observed <= 0
            || now.saturating_sub(observed) > interval_ms.max(60_000) * 2
            || usage["refresh"]["stale"].as_bool().unwrap_or(true),
        "limits": bounded,
        "needs_read": limits.is_some() && bounded.is_none(),
        "reason": read.get("reason"),
        "message": read.get("message"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{ProviderRefreshState, ProviderUsage};

    #[test]
    fn routing_tiers_choose_and_bound_reviewers_within_the_family_lists() {
        use super::super::super::agent_tools::{ReviewEffort, ReviewPreferences};
        let ids = |list: Vec<Value>| {
            list.iter()
                .map(|v| v["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        let claude = json!([
            {"id":"fable-max","values":{"model":"claude-fable-5-1[1m]","effort":"max"}},
            {"id":"sonnet-high","values":{"model":"sonnet","effort":"high"}},
            {"id":"opus-default","values":{"model":"opus","effort":"default"}},
            {"id":"opus-high","values":{"model":"opus","effort":"high"}}
        ]);
        let mut preference = ReviewPreferences::default();
        let routine = candidates(&claude, "claude-code", &preference, false);
        let all = reviewers(&claude, "claude-code", &preference);
        // Opus is the automatic Claude reviewer; Fable needs a high-risk call.
        assert_eq!(ids(routine.clone()), ["opus-high", "opus-default"]);
        assert_eq!(review_preset(&routine, &all, None).unwrap(), "opus-high");
        assert_eq!(
            review_preset(&routine, &all, Some("fable-max")),
            Err("preset_outside_tier")
        );
        let high = candidates(&claude, "claude-code", &preference, true);
        assert_eq!(ids(high.clone())[0], "opus-high");
        assert_eq!(
            review_preset(&high, &all, Some("fable-max")).unwrap(),
            "fable-max"
        );
        assert_eq!(
            review_preset(&high, &all, Some("sonnet-high")),
            Err("review_model_not_allowed")
        );
        assert_eq!(review_preset(&[], &[], None), Err("preset_unavailable"));
        // Family lists still filter: Fable only, or Opus only.
        preference.claude = vec!["fable".into()];
        assert!(
            candidates(&claude, "claude-code", &preference, false)
                .iter()
                .all(|v| v["id"] == "fable-max")
        );
        preference.claude = vec!["opus".into()];
        assert!(
            !ids(candidates(&claude, "claude-code", &preference, true))
                .iter()
                .any(|v| v.contains("fable"))
        );
        assert_eq!(family("gpt-6.1-sol"), Some("sol"));
        assert_eq!(family("opus[1m]"), Some("opus"));
        assert_eq!(family("claude-fable-5-1[1m]"), Some("fable"));

        let gpt = json!([
            {"id":"astra-max","values":{"model":"gpt-6.2-astra","reasoning_effort":"max"}},
            {"id":"astra-medium","values":{"model":"gpt-6.2-astra","reasoning_effort":"medium"}},
            {"id":"sol-max","values":{"model":"gpt-6.2-sol","reasoning_effort":"max"}},
            {"id":"sol-medium","values":{"model":"gpt-6.2-sol","reasoning_effort":"medium"}}
        ]);
        let mut preference = ReviewPreferences::default();
        let all = reviewers(&gpt, "codex", &preference);
        let routine = candidates(&gpt, "codex", &preference, false);
        assert_eq!(
            ids(routine.clone()),
            ["sol-max", "astra-medium", "sol-medium"]
        );
        assert_eq!(
            review_preset(&routine, &all, Some("astra-max")),
            Err("preset_outside_tier")
        );
        assert_eq!(
            review_preset(&routine, &all, Some("astra-medium")).unwrap(),
            "astra-medium"
        );
        // High risk also starts at Sol Max; Astra Max needs an explicit,
        // high-risk request.
        let high = candidates(&gpt, "codex", &preference, true);
        assert_eq!(
            ids(high.clone()),
            ["sol-max", "astra-max", "astra-medium", "sol-medium"]
        );
        assert_eq!(
            review_preset(&high, &all, Some("astra-max")).unwrap(),
            "astra-max"
        );
        preference.gpt_effort = ReviewEffort::Standard;
        assert_eq!(
            ids(candidates(&gpt, "codex", &preference, false)),
            ["sol-medium", "astra-medium", "sol-max"]
        );
        assert_eq!(
            ids(candidates(&gpt, "codex", &preference, true))[0],
            "sol-max"
        );
        // SOL only: the routine tier stays inside the list.
        preference.gpt = vec!["sol".into()];
        preference.gpt_effort = ReviewEffort::Max;
        assert_eq!(
            ids(candidates(&gpt, "codex", &preference, false)),
            ["sol-max", "sol-medium"]
        );
        assert_eq!(
            ids(candidates(&gpt, "codex", &preference, true))[0],
            "sol-max"
        );
        // A generation the table does not know keeps the legacy order.
        let older = json!([
            {"id":"astra-max","values":{"model":"gpt-6-astra","reasoning_effort":"max"}},
            {"id":"sol-max","values":{"model":"gpt-6.1-sol","reasoning_effort":"max"}}
        ]);
        let preference = ReviewPreferences::default();
        assert_eq!(
            ids(candidates(&older, "codex", &preference, false)),
            ["astra-max", "sol-max"]
        );
        assert_eq!(family("gpt-6-astra"), Some("astra"));
        assert_eq!(family("gpt-6-luna"), None);
        let view = routing_view(&gpt, "codex", &preference, (false, Some(None)));
        assert_eq!(view["routine"]["preset"], "sol-max");
        assert_eq!(view["routine"]["cost"]["n"], 4);
        assert_eq!(view["high"]["preset"], "sol-max");
    }

    #[test]
    fn a_same_vendor_review_excludes_the_callers_own_model() {
        let options =
            json!([{"id":"model","currentValue":"opus[1m]"},{"id":"effort","currentValue":"high"}]);
        assert_eq!(caller_family(Some(&options)), Some(Some("opus")));
        assert_eq!(
            caller_family(Some(&json!([{"id":"model","currentValue":"sonnet"}]))),
            Some(None)
        );
        assert_eq!(caller_family(None), None);
        let claude = json!([
            {"id":"fable-max","values":{"model":"claude-fable-5-1[1m]","effort":"max"}},
            {"id":"opus-high","values":{"model":"opus","effort":"high"}}
        ]);
        let ids = |v: Value| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|p| p["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(exclude_caller_model(&claude, true, Some(Some("opus")))),
            ["fable-max"]
        );
        // Another vendor's package can run the same model: still excluded.
        assert_eq!(
            ids(exclude_caller_model(&claude, false, Some(Some("opus")))),
            ["fable-max"]
        );
        // A known non-reviewer caller (Sonnet) keeps every reviewer.
        assert_eq!(
            ids(exclude_caller_model(&claude, true, Some(None))),
            ["fable-max", "opus-high"]
        );
        // An unknown caller model offers no same-vendor reviewer.
        assert!(ids(exclude_caller_model(&claude, true, None)).is_empty());
        assert_eq!(ids(exclude_caller_model(&claude, false, None)).len(), 2);
        let preference = super::super::super::agent_tools::ReviewPreferences::default();
        // An Opus caller's routine Claude review has no Opus reviewer left.
        let request: crate::managed_calls::Request = serde_json::from_value(json!({
            "schema":1,"request_id":"r","purpose":"review","instruction":"i",
            "context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}
        })).unwrap();
        let none = exclude_caller_model(&json!([claude[1].clone()]), true, Some(Some("opus")));
        let refused = admit(
            &request,
            &none,
            "claude-code",
            &preference,
            None,
            (true, Some(Some("opus"))),
        )
        .unwrap_err();
        assert_eq!(refused["error"]["code"], "same_model");
        let fable_only = exclude_caller_model(&claude, true, Some(Some("opus")));
        // Fable is not a routine Claude reviewer, so routine refuses it...
        assert_eq!(
            admit(
                &request,
                &fable_only,
                "claude-code",
                &preference,
                Some("fable-max"),
                (true, Some(Some("opus")))
            )
            .unwrap_err()["error"]["code"],
            "same_model"
        );
        // ...and a high-risk review never falls back to Fable unnamed, but
        // may name it with a reason.
        let mut high = request.clone();
        high.labels.insert("review_risk".into(), "high".into());
        high.labels
            .insert("selection_reason".into(), "persistence".into());
        let caller = (true, Some(Some("opus")));
        assert_eq!(
            admit(&high, &claude, "claude-code", &preference, None, caller).unwrap_err()["error"]["code"],
            "same_model"
        );
        assert_eq!(
            admit(
                &high,
                &claude,
                "claude-code",
                &preference,
                Some("fable-max"),
                caller
            )
            .unwrap(),
            "fable-max"
        );
        // A Sol caller's routine GPT review falls to the cheaper Astra preset.
        let gpt = json!([
            {"id":"astra-max","values":{"model":"gpt-6.2-astra","reasoning_effort":"max"}},
            {"id":"astra-medium","values":{"model":"gpt-6.2-astra","reasoning_effort":"medium"}},
            {"id":"sol-6-2-max","values":{"model":"gpt-6.2-sol","reasoning_effort":"max"}}
        ]);
        let without_sol = exclude_caller_model(&gpt, true, Some(Some("sol")));
        assert_eq!(
            admit(
                &request,
                &without_sol,
                "codex",
                &preference,
                None,
                (true, Some(Some("sol")))
            )
            .unwrap(),
            "astra-medium"
        );
    }

    #[test]
    fn a_continued_review_round_stays_inside_its_declared_tier() {
        let preference = super::super::super::agent_tools::ReviewPreferences::default();
        let gpt = json!([
            {"id":"astra-max","values":{"model":"gpt-6.2-astra","reasoning_effort":"max"}},
            {"id":"astra-medium","values":{"model":"gpt-6.2-astra","reasoning_effort":"medium"}},
            {"id":"sol-6-2-max","values":{"model":"gpt-6.2-sol","reasoning_effort":"max"}}
        ]);
        let round: crate::managed_calls::Request = serde_json::from_value(json!({
            "schema":1,"request_id":"r","purpose":"review","instruction":"Confirm the fixes",
            "context":{"scope":"current-worktree"},"access":"read-only",
            "conversation":{"mode":"continue","child_session_id":"sess-child"}
        }))
        .unwrap();
        let admit = |request: &crate::managed_calls::Request, preset: Option<&str>| {
            admit_continuation(request, &gpt, "codex", &preference, (preset, 1), false)
        };
        // An undeclared confirmation round is routine: an Astra Max child is
        // refused and pointed at a fresh routine review.
        let refused = admit(&round, Some("astra-max")).unwrap_err();
        assert_eq!(refused["error"]["code"], "preset_outside_tier");
        assert!(
            refused["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("sol-6-2-max")
        );
        assert!(admit(&round, Some("sol-6-2-max")).is_ok());
        assert!(admit(&round, Some("astra-medium")).is_ok());
        // A custom or unrouted preset keeps running.
        assert!(admit(&round, Some("custom:model=x")).is_ok());
        assert!(admit(&round, None).is_ok());
        // High keeps the strong child, but only with a reason.
        let mut high = round.clone();
        high.labels.insert("review_risk".into(), "high".into());
        assert_eq!(
            admit(&high, Some("astra-max")).unwrap_err()["error"]["code"],
            "risk_reason_required"
        );
        high.labels.insert(
            "selection_reason".into(),
            "the fixes rewrote the ledger CAS".into(),
        );
        assert!(admit(&high, Some("astra-max")).is_ok());
        // A child that already completed three rounds takes no fourth, at
        // any tier.
        for request in [&round, &high] {
            assert_eq!(
                admit_continuation(
                    request,
                    &gpt,
                    "codex",
                    &preference,
                    (Some("sol-6-2-max"), MAX_REVIEW_ROUNDS),
                    false
                )
                .unwrap_err()["error"]["code"],
                "review_rounds_exhausted"
            );
        }
        assert!(
            admit_continuation(
                &round,
                &gpt,
                "codex",
                &preference,
                (Some("sol-6-2-max"), MAX_REVIEW_ROUNDS - 1),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn antigravity_reviews_only_with_its_claude_opus_preset() {
        let presets = json!([
            {"id":"gemini-neon-high","values":{"model":"gemini-4-neon","thought_level":"high"}},
            {"id":"gemini-hydrogen-xhigh","values":{"model":"gemini-4-hydrogen","thought_level":"xhigh"}},
            {"id":"claude-opus-high","values":{"model":"claude-opus-5-5","thought_level":"high"}},
            {"id":"claude-sonnet-high","values":{"model":"claude-sonnet-5-5","thought_level":"high"}},
            {"id":"claude-fable-high","values":{"model":"claude-fable-5-2","thought_level":"high"}},
        ]);
        // Even a session that prefers Fable gets Opus only from Antigravity.
        let preference = super::super::super::agent_tools::ReviewPreferences {
            claude: vec!["fable".into(), "opus".into()],
            ..Default::default()
        };
        assert_eq!(reviewer_families("antigravity", &preference), ["opus"]);
        // An Opus caller loses Antigravity's only reviewer to its own model.
        let opus = vec![presets[2].clone()];
        let filtered = exclude_caller_model(&json!(opus), false, Some(Some("opus")));
        let caller = Some(Some("opus"));
        assert!(cross_vendor_same_model(&opus, &[], None, caller));
        assert!(cross_vendor_same_model(
            &opus,
            &[],
            Some("claude-opus-high"),
            caller
        ));
        assert!(!cross_vendor_same_model(
            &opus,
            &[],
            Some("gemini-neon-high"),
            caller
        ));
        // A disabled family is no same-model refusal: nothing was eligible.
        assert!(!cross_vendor_same_model(&[], &[], None, caller));
        assert!(filtered.as_array().unwrap().is_empty());
        let choices = candidates(&presets, "antigravity", &preference, true);
        let ids: Vec<_> = choices.iter().filter_map(|c| c["id"].as_str()).collect();
        assert_eq!(ids, ["claude-opus-high"]);
        // The routing table does not cover Antigravity: a routine review
        // keeps the same single Claude Opus candidate.
        let routine = candidates(&presets, "antigravity", &preference, false);
        assert_eq!(routine, choices);
        assert_eq!(
            review_preset(&choices, &choices, None).as_deref(),
            Ok("claude-opus-high")
        );
        assert_eq!(
            review_preset(&choices, &choices, Some("gemini-neon-high")),
            Err("review_model_not_allowed")
        );
    }

    #[test]
    fn quota_preserves_model_windows_and_marks_stale_missing_and_oversized_evidence() {
        let mut snapshot = UsageSnapshot {
            refreshed_at_ms: 1000,
            next_refresh_at_ms: 61000,
            refresh_interval_ms: 60000,
            reset_schedules: Default::default(),
            refreshing: vec![],
            providers: vec![ProviderUsage {
                provider: "anthropic",
                status: "ok",
                source: "test",
                observed_at_ms: 1000,
                account: None,
                activity: None,
                error: None,
                refresh: Some(ProviderRefreshState {
                    last_attempt_at_ms: 1000,
                    manual_refresh_after_ms: 0,
                    next_auto_refresh_at_ms: 61000,
                    stale: false,
                    failures: 0,
                }),
                rate_limits: Some(json!({"rateLimitsByLimitId": {
                    "claude-five_hour": {"primary":{"usedPercent":20,"resetsAt":42}},
                    "claude-model-Fable": {"primary":{"usedPercent":98,"resetsAt":100}}
                }})),
            }],
        };
        let accounts = vec!["anthropic".into(), "openai".into()];
        let view = quota(&snapshot, &accounts, 2000);
        assert_eq!(
            view["accounts"][0]["limits"]["rateLimitsByLimitId"]["claude-model-Fable"]["primary"]["usedPercent"],
            98
        );
        assert_eq!(view["accounts"][0]["stale"], false);
        assert_eq!(view["unavailable_accounts"][0], "openai");
        assert_eq!(
            quota(&snapshot, &accounts, 200000)["accounts"][0]["stale"],
            true
        );
        assert!(
            quota(&snapshot, &[], 2000)["accounts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        snapshot.providers[0].rate_limits = Some(json!({"large":"x".repeat(8193)}));
        let view = quota(&snapshot, &accounts, 2000);
        assert_eq!(view["accounts"][0]["needs_read"], true);
        assert!(view["accounts"][0]["limits"].is_null());
    }

    #[test]
    fn named_quota_reports_the_account_read_and_its_freshness() {
        let fresh = json!({"account_id":"a","provider_id":"claude-code","usage":{
            "status":"available","observed_at_ms":1000,
            "rate_limits":{"rateLimitsByLimitId":{"claude-seven_day":{"primary":{"usedPercent":62}}}},
            "refresh":{"stale":false}}});
        let view = named_quota(&fresh, 2000, 60_000);
        assert_eq!(view["account"], "a");
        assert_eq!(view["status"], "available");
        assert_eq!(view["stale"], false);
        assert_eq!(
            view["limits"]["rateLimitsByLimitId"]["claude-seven_day"]["primary"]["usedPercent"],
            62
        );
        // Old, refresh-stale or refused reads are never ample evidence.
        assert_eq!(named_quota(&fresh, 1000 + 200_000, 60_000)["stale"], true);
        let refused =
            json!({"account_id":"a","reason":"native_login_required","message":"Sign in"});
        let view = named_quota(&refused, 2000, 60_000);
        assert_eq!(view["status"], "unavailable");
        assert_eq!(view["stale"], true);
        assert_eq!(view["reason"], "native_login_required");
        assert!(view["limits"].is_null());
    }
}
