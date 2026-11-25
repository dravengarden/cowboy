//! Evidence and quota-pace-aware routing for managed reviews and analyses.
//! Simple/routine reviews stay on Sol Max / Opus; high-risk reviews dynamically
//! switch Codex priority between Astra Max and Sol Max by fresh quota headroom
//! relative to reset time, and agents may self-assess important changes to name
//! Astra Max with risk:'high' and a concrete reason.
use serde_json::{Value, json};

mod routing;

pub const GUIDANCE: &str = "Controller picks Review/Design Review preset from providers[].review_routing by declared risk and live quota; cost matters. Routine changes (UI, copy, docs, tests, mechanical tweaks, ordinary refactors, fix confirmations) use risk:'routine' (Sol Max / Claude Opus; stronger presets fail preset_outside_tier). High-risk changes (data loss/corruption, auth/permissions/isolation, billing/limits, routing/deployment/protocol/concurrency, or review root AGENTS.md Review risk areas) use risk:'high' + concrete reason. On Codex risk:'high', automatic priority switches between Astra Max (ample fresh quota) and Sol Max (conserve); check review_routing.high and quota_mode. You may explicitly name preset:'astra-max' with risk:'high' + reason when warranted (Fable is explicit-only with risk:'high' + reason). Prefer another vendor (Auto tries others first); own vendor is allowed, never own model (same_model). Fix confirmations are routine: continue a routine child, or for Astra Max start a fresh routine review with same task+base+root (reuses input); max 3 rounds (review_rounds_exhausted). Audits are analysis, not reviews. Tokens != quota; usedPercent is USED, resetsAt is epoch s. Large quota: cowboy.usage.read. Diagnose failures before retrying.";

pub fn family(model: &str) -> Option<&'static str> {
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
pub fn review_preset(
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
pub fn candidates(
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
) -> Vec<Value> {
    ranked(presets, presets, provider, preference, high_risk, None, 0).list
}

/// `candidates` for a caller: `runnable` is the whole generation (coverage),
/// filtered here by the caller's own model, with optional cached quota evidence
/// for dynamic high-risk priority switching.
pub fn candidates_for(
    runnable: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
    (same_vendor, caller): (bool, Option<Option<&str>>),
    quota: Option<(&Value, i64)>,
) -> routing::Ranked {
    let presets = exclude_caller_model(runnable, same_vendor, caller);
    let (quota_value, now_ms) = quota.map_or((None, 0), |(q, now)| (Some(q), now));
    ranked(
        &presets,
        runnable,
        provider,
        preference,
        high_risk,
        quota_value,
        now_ms,
    )
}

fn ranked(
    presets: &Value,
    coverage: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    high_risk: bool,
    quota: Option<&Value>,
    now_ms: i64,
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
        .and_then(|table| routing::order(table, provider, level, &pool, coverage, quota, now_ms))
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
pub fn caller_family(configuration: Option<&Value>) -> Option<Option<&'static str>> {
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
pub fn same_model(same_vendor: bool, caller: Option<Option<&str>>, reviewer: Option<&str>) -> bool {
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
pub fn runs_now(
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
pub fn exclude_caller_model(
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
pub fn admit(
    request: &crate::managed_calls::Request,
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    explicit: Option<&str>,
    (same_vendor, caller): (bool, Option<Option<&str>>),
    quota: Option<(&Value, i64)>,
) -> Result<String, Value> {
    let runnable = presets;
    let presets = &exclude_caller_model(runnable, same_vendor, caller);
    let risk = request
        .labels
        .get(crate::managed_calls::review::RISK)
        .map_or("undeclared", String::as_str);
    let high_risk = risk == "high";
    let quota_mode = high_risk
        .then(|| {
            let (q, now) = quota.map_or((None, 0), |(q, now)| (Some(q), now));
            routing::TABLE
                .as_ref()
                .and_then(|table| routing::high_quota_assessment(table, provider, q, now))
                .map(|a| a.mode)
        })
        .flatten()
        .unwrap_or("none");
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
            quota_mode,
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
        quota,
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
        quota,
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
pub fn admit_continuation(
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
        None,
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
pub fn reviewers(
    presets: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
) -> Vec<Value> {
    legacy(presets, provider, preference, true)
}

/// What the caller sees of the tiers: each tier's automatic preset and the
/// cheaper presets a routine review may name, with measured costs.
pub fn routing_view(
    runnable: &Value,
    provider: &str,
    preference: &super::super::agent_tools::ReviewPreferences,
    caller: (bool, Option<Option<&str>>),
    quota: Option<(&Value, i64)>,
) -> Value {
    let Some(table) = routing::TABLE.as_ref() else {
        return Value::Null;
    };
    let entry =
        |preset: &Value| json!({"preset": preset["id"], "cost": routing::cost(table, preset)});
    let high = candidates_for(runnable, provider, preference, true, caller, quota);
    let high = if high.automatic {
        high.list
    } else {
        Vec::new()
    };
    let routine = candidates_for(runnable, provider, preference, false, caller, quota);
    let routine = if routine.automatic {
        routine.list
    } else {
        Vec::new()
    };
    let (q_val, now_ms) = quota.map_or((None, 0), |(q, now)| (Some(q), now));
    let assessment = routing::high_quota_assessment(table, provider, q_val, now_ms);
    let mut out = json!({
        "evidence": table.evidence,
        "high": high.first().map(entry),
        "routine": routine.first().map(entry),
        "cheaper": routine.iter().skip(1).map(entry).collect::<Vec<_>>(),
    });
    if let Some(a) = assessment {
        out["quota_mode"] = json!(a.mode);
        out["quota_headroom"] = a.to_json();
    }
    out
}

/// Automatic preset for a fresh unnamed `analysis` (or `context_ask`) call,
/// from `models/routing.json` against the pinned generation's runnable presets.
pub fn analysis_preset(runnable: &Value, provider: &str, context_ask: bool) -> Option<String> {
    let table = routing::TABLE.as_ref()?;
    routing::analysis_choice(table, provider, runnable, context_ask)
        .and_then(|preset| preset["id"].as_str().map(str::to_owned))
}

/// Route an unnamed fresh `analysis` (or `context_ask`) call to the cheap
/// preset in `models/routing.json` when no explicit preset or Tools pin is set.
pub fn admit_analysis(
    request: &crate::managed_calls::Request,
    runnable: &Value,
    target: &mut super::super::agent_tools::CallTarget,
    preset: Option<&str>,
) {
    if !matches!(request.purpose, crate::managed_calls::Purpose::Analysis)
        || !matches!(
            request.conversation,
            crate::managed_calls::Conversation::Fresh {}
        )
        || preset.is_some()
        || target.preset.is_some()
        || target.options.is_some()
    {
        return;
    }
    let context_ask = !request.context.sessions.is_empty();
    if let Some(chosen) = analysis_preset(runnable, &target.agent, context_ask) {
        tracing::info!(
            event_name = "cowboy.analysis.routed",
            request_id = %request.request_id,
            provider = %target.agent,
            context_ask,
            preset = %chosen,
            outcome = "automatic",
            "analysis routed"
        );
        target.preset = Some(chosen);
    }
}

/// Capabilities view of `provider`'s automatic analysis / context-ask routing.
pub fn analysis_routing_view(runnable: &Value, provider: &str) -> Value {
    routing::TABLE.as_ref().map_or(Value::Null, |table| {
        routing::analysis_view(table, provider, runnable)
    })
}

/// The family/effort order that predates the routing table.
/// The reviewer families `provider` may review with, in the session's order.
/// Antigravity reviews only with Claude Opus (its named account's Claude
/// quota), whatever else a signed generation's presets offer.
pub fn reviewer_families(
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

/// The quota evidence of a named account, from its owner-scoped usage read
/// (`provider_accounts::usage::read_one`). A child runs on this account.
pub fn named_quota(read: &Value, now: i64, interval_ms: i64) -> Value {
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
        let view = routing_view(&gpt, "codex", &preference, (false, Some(None)), None);
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
            None,
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
                (true, Some(Some("opus"))),
                None
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
            admit(
                &high,
                &claude,
                "claude-code",
                &preference,
                None,
                caller,
                None
            )
            .unwrap_err()["error"]["code"],
            "same_model"
        );
        assert_eq!(
            admit(
                &high,
                &claude,
                "claude-code",
                &preference,
                Some("fable-max"),
                caller,
                None
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
                (true, Some(Some("sol"))),
                None
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
    fn analysis_routing_picks_cheap_presets_without_widening_reviewers() {
        let codex = json!([
            {"id":"astra-max","values":{"model":"gpt-6.2-astra","reasoning_effort":"max"}},
            {"id":"sol-6-2-max","values":{"model":"gpt-6.2-sol","reasoning_effort":"max"}},
            {"id":"sol-6-2-medium","values":{"model":"gpt-6.2-sol","reasoning_effort":"medium"}},
            {"id":"luna-6-max","values":{"model":"gpt-6-luna","reasoning_effort":"max"}}
        ]);
        assert_eq!(
            analysis_preset(&codex, "codex", false).as_deref(),
            Some("sol-6-2-medium")
        );
        assert_eq!(
            analysis_preset(&codex, "codex", true).as_deref(),
            Some("sol-6-2-medium")
        );
        // Falls back to luna-6-max for context asks when sol-6-2-medium is absent.
        let luna_only = json!([
            {"id":"astra-max","values":{"model":"gpt-6.2-astra","reasoning_effort":"max"}},
            {"id":"luna-6-max","values":{"model":"gpt-6-luna","reasoning_effort":"max"}}
        ]);
        assert_eq!(
            analysis_preset(&luna_only, "codex", true).as_deref(),
            Some("luna-6-max")
        );
        let claude = json!([
            {"id":"sonnet-xhigh","values":{"model":"sonnet","effort":"xhigh"}},
            {"id":"opus-high","values":{"model":"opus","effort":"high"}},
            {"id":"sonnet-medium","values":{"model":"sonnet","effort":"medium"}}
        ]);
        assert_eq!(
            analysis_preset(&claude, "claude-code", false).as_deref(),
            Some("sonnet-medium")
        );
        let agy = json!([
            {"id":"gemini-neon-high","values":{"model":"gemini-4-neon","thought_level":"high"}},
            {"id":"gemini-hydrogen-xhigh","values":{"model":"gemini-4-hydrogen","thought_level":"xhigh"}},
            {"id":"gemini-hydrogen-high","values":{"model":"gemini-4-hydrogen","thought_level":"high"}},
            {"id":"claude-opus-high","values":{"model":"claude-opus-5-5","thought_level":"high"}}
        ]);
        assert_eq!(
            analysis_preset(&agy, "antigravity", false).as_deref(),
            Some("gemini-hydrogen-high")
        );
        assert_eq!(
            analysis_preset(&agy, "antigravity", true).as_deref(),
            Some("gemini-hydrogen-high")
        );
        let view = analysis_routing_view(&agy, "antigravity");
        assert_eq!(view["automatic"]["preset"], "gemini-hydrogen-high");
        assert_eq!(view["context_ask"]["preset"], "gemini-hydrogen-high");
        // Reviewer allowlist remains strictly unaffected by analysis families.
        assert_eq!(family("gemini-4-hydrogen"), None);
        assert_eq!(family("gpt-6-luna"), None);
        assert_eq!(family("sonnet"), None);
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

    #[test]
    fn codex_high_risk_switches_between_astra_and_sol_by_quota_pace() {
        use super::super::super::agent_tools::ReviewPreferences;
        let gpt = json!([
            {"id":"astra-max","values":{"model":"gpt-6.2-astra","reasoning_effort":"max"}},
            {"id":"sol-max","values":{"model":"gpt-6.2-sol","reasoning_effort":"max"}},
            {"id":"astra-medium","values":{"model":"gpt-6.2-astra","reasoning_effort":"medium"}},
            {"id":"sol-medium","values":{"model":"gpt-6.2-sol","reasoning_effort":"medium"}}
        ]);
        let preference = ReviewPreferences::default();
        let now_ms: i64 = 1_795_515_292_000;
        let now_s = now_ms / 1000;
        let mk_quota = |used: f64, hours_left: f64, stale: bool| {
            json!({
                "status": "available",
                "stale": stale,
                "limits": {
                    "rateLimitsByLimitId": {
                        "codex": {
                            "primary": {
                                "usedPercent": used,
                                "windowDurationMins": 10080,
                                "resetsAt": now_s + (hours_left * 3600.0) as i64
                            }
                        }
                    }
                }
            })
        };

        // 1. Ahead of linear pace (57% used => 43% left, 2.14d/7d => 30.6% time left): high -> astra-max, routine stays sol-max.
        let ample = mk_quota(57.0, 51.4, false);
        let view = routing_view(
            &gpt,
            "codex",
            &preference,
            (false, Some(None)),
            Some((&ample, now_ms)),
        );
        assert_eq!(view["high"]["preset"], "astra-max");
        assert_eq!(view["routine"]["preset"], "sol-max");
        assert_eq!(view["quota_mode"], "ample");

        // 2. Early-window abundance (10% used => 90% left >= 70% comfort even with 6.9d left): high -> astra-max.
        let fresh_week = mk_quota(10.0, 165.0, false);
        let view = routing_view(
            &gpt,
            "codex",
            &preference,
            (false, Some(None)),
            Some((&fresh_week, now_ms)),
        );
        assert_eq!(view["high"]["preset"], "astra-max");
        assert_eq!(view["quota_mode"], "ample");

        // 3. Behind linear pace (45% used => 55% left, 5.5d/7d => 78.6% time left): conserve -> sol-max.
        let burning_fast = mk_quota(45.0, 132.0, false);
        let view = routing_view(
            &gpt,
            "codex",
            &preference,
            (false, Some(None)),
            Some((&burning_fast, now_ms)),
        );
        assert_eq!(view["high"]["preset"], "sol-max");
        assert_eq!(view["quota_mode"], "conserve");
        assert_eq!(view["quota_headroom"]["reason"], "behind_pace");

        // 4. Below 20% safety floor even near reset (85% used => 15% left, 6h left): conserve -> sol-max.
        let low_floor = mk_quota(85.0, 6.0, false);
        let view = routing_view(
            &gpt,
            "codex",
            &preference,
            (false, Some(None)),
            Some((&low_floor, now_ms)),
        );
        assert_eq!(view["high"]["preset"], "sol-max");
        assert_eq!(view["quota_mode"], "conserve");
        assert_eq!(view["quota_headroom"]["reason"], "below_safety_floor");

        // 5. Stale quota fails safe to conserve -> sol-max.
        let stale = mk_quota(10.0, 24.0, true);
        let view = routing_view(
            &gpt,
            "codex",
            &preference,
            (false, Some(None)),
            Some((&stale, now_ms)),
        );
        assert_eq!(view["high"]["preset"], "sol-max");
        assert_eq!(view["quota_mode"], "conserve");
        assert_eq!(view["quota_headroom"]["reason"], "quota_stale");

        // 6. Tight secondary 5h burst window overrides ample 7d window -> conserve.
        let burst_tight = json!({
            "status": "available",
            "stale": false,
            "limits": {
                "rateLimitsByLimitId": {
                    "codex": {
                        "primary": {"usedPercent": 85.0, "windowDurationMins": 300, "resetsAt": now_s + 3600},
                        "secondary": {"usedPercent": 20.0, "windowDurationMins": 10080, "resetsAt": now_s + 86400}
                    }
                }
            }
        });
        let view = routing_view(
            &gpt,
            "codex",
            &preference,
            (false, Some(None)),
            Some((&burst_tight, now_ms)),
        );
        assert_eq!(view["high"]["preset"], "sol-max");
        assert_eq!(view["quota_mode"], "conserve");

        // 7. Same-model exclusion under ample quota: Astra caller falls back to sol-max; Sol caller gets astra-max.
        let high_req: crate::managed_calls::Request = serde_json::from_value(json!({
            "schema":1,"request_id":"r-high","purpose":"review","instruction":"i",
            "context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"},
            "labels":{"review_risk":"high","selection_reason":"critical state machine"}
        })).unwrap();
        assert_eq!(
            admit(
                &high_req,
                &gpt,
                "codex",
                &preference,
                None,
                (true, Some(Some("sol"))),
                Some((&ample, now_ms))
            )
            .unwrap(),
            "astra-max"
        );
        assert_eq!(
            admit(
                &high_req,
                &gpt,
                "codex",
                &preference,
                None,
                (true, Some(Some("astra"))),
                Some((&ample, now_ms))
            )
            .unwrap(),
            "sol-max"
        );
        // Even under conserve, agent can still explicitly choose astra-max with risk:high + reason.
        assert_eq!(
            admit(
                &high_req,
                &gpt,
                "codex",
                &preference,
                Some("astra-max"),
                (false, Some(None)),
                Some((&burning_fast, now_ms))
            )
            .unwrap(),
            "astra-max"
        );
        // Routine review ALWAYS refuses astra-max even when quota is ample.
        let routine_req: crate::managed_calls::Request = serde_json::from_value(json!({
            "schema":1,"request_id":"r-routine","purpose":"review","instruction":"i",
            "context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"},
            "labels":{"review_risk":"routine"}
        })).unwrap();
        let refused = admit(
            &routine_req,
            &gpt,
            "codex",
            &preference,
            Some("astra-max"),
            (false, Some(None)),
            Some((&ample, now_ms)),
        )
        .unwrap_err();
        assert_eq!(refused["error"]["code"], "preset_outside_tier");
    }
}
