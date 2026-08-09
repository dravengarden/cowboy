//! DeepSeek-owned model normalization and current list-price valuation.
//!
//! Account balances come from DeepSeek. Request telemetry comes from Columbus
//! gateways. This adapter is the only place where those measured token counts
//! are assigned a provider price; Web clients must not carry an independent
//! pricing table.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map, Value, json};

const PRICE_AS_OF: &str = "2026-08-06";
const PRICE_VERSION: &str = "deepseek-v4-cny-2026-08-06";
const PRICE_SOURCE_URL: &str = "https://api-docs.deepseek.com/zh-cn/quick_start/pricing";
const TOKENS_PER_MILLION: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelFamily {
    Flash,
    Pro,
}

impl ModelFamily {
    const fn name(self) -> &'static str {
        match self {
            Self::Flash => "flash",
            Self::Pro => "pro",
        }
    }

    const fn price(self) -> Price {
        match self {
            Self::Flash => Price {
                input_cache_hit_cny_per_million: 0.02,
                input_cache_miss_cny_per_million: 1.0,
                output_cny_per_million: 2.0,
            },
            Self::Pro => Price {
                input_cache_hit_cny_per_million: 0.025,
                input_cache_miss_cny_per_million: 3.0,
                output_cny_per_million: 6.0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Price {
    input_cache_hit_cny_per_million: f64,
    input_cache_miss_cny_per_million: f64,
    output_cny_per_million: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct CostEstimate {
    estimated_cny: f64,
    no_cache_cny: f64,
    all_hit_floor_cny: f64,
    cache_savings_cny: f64,
    cache_miss_premium_cny: f64,
    requests: u64,
    usage_observed_requests: u64,
    unknown_model_requests: u64,
    input_tokens: u64,
    priced_input_tokens: u64,
    unpriced_input_tokens: u64,
    output_tokens: u64,
    unpriced_output_tokens: u64,
    reasoning_tokens: u64,
    model_families: BTreeSet<String>,
}

#[derive(Clone, Copy)]
struct CostMetricKeys {
    requests: &'static str,
    usage_observations: &'static str,
    input_tokens: &'static str,
    output_tokens: &'static str,
    reasoning_tokens: &'static str,
    cache_hit_tokens: &'static str,
    cache_miss_tokens: &'static str,
}

const INTERACTIVE_COST_KEYS: CostMetricKeys = CostMetricKeys {
    requests: "requests",
    usage_observations: "usageObservations",
    input_tokens: "inputTokens",
    output_tokens: "outputTokens",
    reasoning_tokens: "reasoningTokens",
    cache_hit_tokens: "cacheHitTokens",
    cache_miss_tokens: "cacheMissTokens",
};

const CACHE_KEEPALIVE_COST_KEYS: CostMetricKeys = CostMetricKeys {
    requests: "cacheKeepaliveRequests",
    usage_observations: "cacheKeepaliveUsageObservations",
    input_tokens: "cacheKeepaliveInputTokens",
    output_tokens: "cacheKeepaliveOutputTokens",
    reasoning_tokens: "cacheKeepaliveReasoningTokens",
    cache_hit_tokens: "cacheKeepaliveHitTokens",
    cache_miss_tokens: "cacheKeepaliveMissTokens",
};

impl CostEstimate {
    fn for_model(model: &str, aggregate: &Value) -> Self {
        Self::for_model_metrics(model, aggregate, INTERACTIVE_COST_KEYS)
    }

    fn for_cache_keepalive_model(model: &str, aggregate: &Value) -> Self {
        Self::for_model_metrics(model, aggregate, CACHE_KEEPALIVE_COST_KEYS)
    }

    fn for_model_metrics(model: &str, aggregate: &Value, keys: CostMetricKeys) -> Self {
        let requests = metric(aggregate, keys.requests);
        let usage_observed_requests = metric(aggregate, keys.usage_observations);
        let input_tokens = metric(aggregate, keys.input_tokens);
        let output_tokens = metric(aggregate, keys.output_tokens);
        let reasoning_tokens = metric(aggregate, keys.reasoning_tokens);
        let Some(family) = model_family(model) else {
            return Self {
                requests,
                usage_observed_requests,
                unknown_model_requests: requests,
                input_tokens,
                unpriced_input_tokens: input_tokens,
                output_tokens,
                unpriced_output_tokens: output_tokens,
                reasoning_tokens,
                ..Self::default()
            };
        };
        let cache_hit_tokens = metric(aggregate, keys.cache_hit_tokens);
        let cache_miss_tokens = metric(aggregate, keys.cache_miss_tokens);
        let priced_input_tokens = cache_hit_tokens.saturating_add(cache_miss_tokens);
        let price = family.price();
        let hit = cache_hit_tokens as f64 / TOKENS_PER_MILLION;
        let miss = cache_miss_tokens as f64 / TOKENS_PER_MILLION;
        let output = output_tokens as f64 / TOKENS_PER_MILLION;
        let mut model_families = BTreeSet::new();
        model_families.insert(family.name().to_owned());
        Self {
            estimated_cny: hit * price.input_cache_hit_cny_per_million
                + miss * price.input_cache_miss_cny_per_million
                + output * price.output_cny_per_million,
            no_cache_cny: (hit + miss) * price.input_cache_miss_cny_per_million
                + output * price.output_cny_per_million,
            all_hit_floor_cny: (hit + miss) * price.input_cache_hit_cny_per_million
                + output * price.output_cny_per_million,
            cache_savings_cny: hit
                * (price.input_cache_miss_cny_per_million - price.input_cache_hit_cny_per_million),
            cache_miss_premium_cny: miss
                * (price.input_cache_miss_cny_per_million - price.input_cache_hit_cny_per_million),
            requests,
            usage_observed_requests,
            unknown_model_requests: 0,
            input_tokens,
            priced_input_tokens,
            unpriced_input_tokens: input_tokens.saturating_sub(priced_input_tokens),
            output_tokens,
            unpriced_output_tokens: 0,
            reasoning_tokens,
            model_families,
        }
    }

    fn add(&mut self, other: &Self) {
        self.estimated_cny += other.estimated_cny;
        self.no_cache_cny += other.no_cache_cny;
        self.all_hit_floor_cny += other.all_hit_floor_cny;
        self.cache_savings_cny += other.cache_savings_cny;
        self.cache_miss_premium_cny += other.cache_miss_premium_cny;
        self.requests = self.requests.saturating_add(other.requests);
        self.usage_observed_requests = self
            .usage_observed_requests
            .saturating_add(other.usage_observed_requests);
        self.unknown_model_requests = self
            .unknown_model_requests
            .saturating_add(other.unknown_model_requests);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.priced_input_tokens = self
            .priced_input_tokens
            .saturating_add(other.priced_input_tokens);
        self.unpriced_input_tokens = self
            .unpriced_input_tokens
            .saturating_add(other.unpriced_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.unpriced_output_tokens = self
            .unpriced_output_tokens
            .saturating_add(other.unpriced_output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.model_families
            .extend(other.model_families.iter().cloned());
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CostView {
    summary: CostEstimate,
    by_agent: BTreeMap<String, CostEstimate>,
    by_billing_model: BTreeMap<String, CostEstimate>,
    by_agent_billing_model: BTreeMap<String, BTreeMap<String, CostEstimate>>,
    by_low_hit_cause: BTreeMap<String, CostEstimate>,
    cache_protection: CacheProtectionCostView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheProtectionCostView {
    summary: CostEstimate,
    by_agent: BTreeMap<String, CostEstimate>,
    by_billing_model: BTreeMap<String, CostEstimate>,
    by_agent_billing_model: BTreeMap<String, BTreeMap<String, CostEstimate>>,
}

pub(crate) fn decorate_activity(activity: &mut Value) {
    let Some(object) = activity.as_object_mut() else {
        return;
    };
    let low_hit_cause_models = object
        .get("lowHit")
        .and_then(Value::as_object)
        .and_then(|low_hit| low_hit.get("byCauseModel"));
    let cost = cost_view(
        object.get("byBillingModel"),
        object.get("byAgentBillingModel"),
        low_hit_cause_models,
    );
    object.insert("pricing".to_owned(), pricing_view());
    object.insert(
        "cost".to_owned(),
        serde_json::to_value(cost).expect("DeepSeek cost view serializes"),
    );
    if let Some(rolling) = object.get_mut("last24Hours").and_then(Value::as_object_mut) {
        let cost = cost_view(
            rolling.get("byBillingModel"),
            rolling.get("byAgentBillingModel"),
            None,
        );
        rolling.insert(
            "cost".to_owned(),
            serde_json::to_value(cost).expect("DeepSeek rolling cost view serializes"),
        );
    }
}

fn cost_view(
    by_billing_model: Option<&Value>,
    by_agent_billing_model: Option<&Value>,
    by_low_hit_cause_model: Option<&Value>,
) -> CostView {
    let cache_protection = cache_protection_cost_view(by_billing_model, by_agent_billing_model);
    let by_billing_model = cost_map(by_billing_model);
    let mut summary = CostEstimate::default();
    for estimate in by_billing_model.values() {
        summary.add(estimate);
    }
    let mut agent_models = BTreeMap::new();
    let mut by_agent = BTreeMap::new();
    if let Some(agents) = by_agent_billing_model.and_then(Value::as_object) {
        for (agent, models) in agents {
            let models = cost_map(Some(models));
            let mut agent_total = CostEstimate::default();
            for estimate in models.values() {
                agent_total.add(estimate);
            }
            by_agent.insert(agent.clone(), agent_total);
            agent_models.insert(agent.clone(), models);
        }
    }
    let mut by_low_hit_cause = BTreeMap::new();
    if let Some(causes) = by_low_hit_cause_model.and_then(Value::as_object) {
        for (cause, models) in causes {
            let mut cause_total = CostEstimate::default();
            for estimate in cost_map(Some(models)).values() {
                cause_total.add(estimate);
            }
            by_low_hit_cause.insert(cause.clone(), cause_total);
        }
    }
    CostView {
        summary,
        by_agent,
        by_billing_model,
        by_agent_billing_model: agent_models,
        by_low_hit_cause,
        cache_protection,
    }
}

fn cache_protection_cost_view(
    by_billing_model: Option<&Value>,
    by_agent_billing_model: Option<&Value>,
) -> CacheProtectionCostView {
    let by_billing_model = cache_keepalive_cost_map(by_billing_model);
    let mut summary = CostEstimate::default();
    for estimate in by_billing_model.values() {
        summary.add(estimate);
    }
    let mut agent_models = BTreeMap::new();
    let mut by_agent = BTreeMap::new();
    if let Some(agents) = by_agent_billing_model.and_then(Value::as_object) {
        for (agent, models) in agents {
            let models = cache_keepalive_cost_map(Some(models));
            let mut agent_total = CostEstimate::default();
            for estimate in models.values() {
                agent_total.add(estimate);
            }
            by_agent.insert(agent.clone(), agent_total);
            agent_models.insert(agent.clone(), models);
        }
    }
    CacheProtectionCostView {
        summary,
        by_agent,
        by_billing_model,
        by_agent_billing_model: agent_models,
    }
}

fn cost_map(value: Option<&Value>) -> BTreeMap<String, CostEstimate> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::iter)
        .map(|(model, aggregate)| (model.clone(), CostEstimate::for_model(model, aggregate)))
        .collect()
}

fn cache_keepalive_cost_map(value: Option<&Value>) -> BTreeMap<String, CostEstimate> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::iter)
        .map(|(model, aggregate)| {
            (
                model.clone(),
                CostEstimate::for_cache_keepalive_model(model, aggregate),
            )
        })
        .collect()
}

fn pricing_view() -> Value {
    json!({
        "provider": "deepseek",
        "currency": "CNY",
        "unit": "per_million_tokens",
        "valuationBasis": "pinned_list_price_snapshot",
        "asOf": PRICE_AS_OF,
        "version": PRICE_VERSION,
        "sourceUrl": PRICE_SOURCE_URL,
        "models": {
            "flash": ModelFamily::Flash.price(),
            "pro": ModelFamily::Pro.price(),
        },
    })
}

fn metric(aggregate: &Value, key: &str) -> u64 {
    aggregate
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|v| v.try_into().ok()))
        })
        .unwrap_or(0)
}

fn model_family(model: &str) -> Option<ModelFamily> {
    let normalized = model
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    if normalized.starts_with("deepseek-v4-pro") {
        Some(ModelFamily::Pro)
    } else if normalized.starts_with("deepseek-v4-flash")
        || matches!(normalized.as_str(), "deepseek-chat" | "deepseek-reasoner")
    {
        Some(ModelFamily::Flash)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "got {actual}, want {expected}"
        );
    }

    #[test]
    fn normalizes_current_models_and_compatibility_names() {
        assert_eq!(model_family("deepseek-v4-flash"), Some(ModelFamily::Flash));
        assert_eq!(
            model_family("deepseek/deepseek-chat"),
            Some(ModelFamily::Flash)
        );
        assert_eq!(model_family("deepseek-reasoner"), Some(ModelFamily::Flash));
        assert_eq!(model_family("deepseek-v4-pro[1m]"), Some(ModelFamily::Pro));
        assert_eq!(model_family("unknown"), None);
    }

    #[test]
    fn values_reasoning_once_as_part_of_completion_tokens() {
        let aggregate = json!({
            "requests": 10,
            "usageObservations": 10,
            "inputTokens": 1_000_000,
            "outputTokens": 50_000,
            "reasoningTokens": 10_000,
            "cacheHitTokens": 900_000,
            "cacheMissTokens": 100_000,
        });
        let cost = CostEstimate::for_model("deepseek-v4-flash", &aggregate);
        assert_close(cost.estimated_cny, 0.9 * 0.02 + 0.1 * 1.0 + 0.05 * 2.0);
        assert_close(cost.no_cache_cny, 1.0 + 0.05 * 2.0);
        assert_close(cost.all_hit_floor_cny, 0.02 + 0.05 * 2.0);
        assert_close(cost.cache_savings_cny, 0.9 * (1.0 - 0.02));
        assert_close(cost.cache_miss_premium_cny, 0.1 * (1.0 - 0.02));
        assert_eq!(cost.output_tokens, 50_000);
        assert_eq!(cost.reasoning_tokens, 10_000);
    }

    #[test]
    fn unknown_models_remain_explicitly_unpriced() {
        let aggregate = json!({
            "requests": 2, "usageObservations": 2,
            "inputTokens": 100, "outputTokens": 20,
        });
        let cost = CostEstimate::for_model("future-model", &aggregate);
        assert_eq!(cost.unknown_model_requests, 2);
        assert_eq!(cost.unpriced_input_tokens, 100);
        assert_eq!(cost.unpriced_output_tokens, 20);
        assert_close(cost.estimated_cny, 0.0);
    }

    #[test]
    fn decorates_mixed_model_and_agent_aggregates_exactly() {
        let mut activity = json!({
            "byBillingModel": {
                "deepseek-v4-flash": { "requests": 1, "usageObservations": 1, "inputTokens": 10, "outputTokens": 1, "cacheHitTokens": 8, "cacheMissTokens": 2 },
                "deepseek-v4-pro": { "requests": 1, "usageObservations": 1, "inputTokens": 20, "outputTokens": 2, "cacheHitTokens": 10, "cacheMissTokens": 10 }
            },
            "byAgentBillingModel": {
                "codex": { "deepseek-v4-flash": { "requests": 1, "usageObservations": 1, "inputTokens": 10, "outputTokens": 1, "cacheHitTokens": 8, "cacheMissTokens": 2 } },
                "claude": { "deepseek-v4-pro": { "requests": 1, "usageObservations": 1, "inputTokens": 20, "outputTokens": 2, "cacheHitTokens": 10, "cacheMissTokens": 10 } }
            },
            "last24Hours": { "byBillingModel": {}, "byAgentBillingModel": {} }
        });
        decorate_activity(&mut activity);
        assert_eq!(activity["pricing"]["currency"], "CNY");
        assert_eq!(activity["cost"]["summary"]["requests"], 2);
        assert_eq!(
            activity["cost"]["byAgent"]["codex"]["modelFamilies"][0],
            "flash"
        );
        assert_eq!(
            activity["cost"]["byAgent"]["claude"]["modelFamilies"][0],
            "pro"
        );
        assert_eq!(activity["last24Hours"]["cost"]["summary"]["requests"], 0);
    }

    #[test]
    fn resolved_billing_model_wins_over_requested_alias() {
        let mut activity = json!({
            "byModel": {
                "deepseek-v4-flash": { "requests": 1, "usageObservations": 1, "inputTokens": 1_000_000, "outputTokens": 0, "cacheHitTokens": 0, "cacheMissTokens": 1_000_000 }
            },
            "byBillingModel": {
                "deepseek-v4-pro": { "requests": 1, "usageObservations": 1, "inputTokens": 1_000_000, "outputTokens": 0, "cacheHitTokens": 0, "cacheMissTokens": 1_000_000 }
            },
            "byAgentBillingModel": {
                "claude": { "deepseek-v4-pro": { "requests": 1, "usageObservations": 1, "inputTokens": 1_000_000, "outputTokens": 0, "cacheHitTokens": 0, "cacheMissTokens": 1_000_000 } }
            }
        });
        decorate_activity(&mut activity);
        assert_close(
            activity["cost"]["summary"]["estimatedCny"]
                .as_f64()
                .unwrap(),
            3.0,
        );
        assert_eq!(activity["cost"]["summary"]["modelFamilies"][0], "pro");
    }

    #[test]
    fn cache_protection_cost_is_valued_separately_from_interactive_spend() {
        let mut activity = json!({
            "byBillingModel": {
                "deepseek-v4-flash": {
                    "requests": 2,
                    "usageObservations": 2,
                    "inputTokens": 100,
                    "outputTokens": 10,
                    "cacheHitTokens": 80,
                    "cacheMissTokens": 20,
                    "cacheKeepaliveRequests": 1,
                    "cacheKeepaliveUsageObservations": 1,
                    "cacheKeepaliveInputTokens": 1_000_000,
                    "cacheKeepaliveOutputTokens": 1,
                    "cacheKeepaliveReasoningTokens": 0,
                    "cacheKeepaliveHitTokens": 1_000_000,
                    "cacheKeepaliveMissTokens": 0
                }
            },
            "byAgentBillingModel": {
                "claude": {
                    "deepseek-v4-flash": {
                        "cacheKeepaliveRequests": 1,
                        "cacheKeepaliveUsageObservations": 1,
                        "cacheKeepaliveInputTokens": 1_000_000,
                        "cacheKeepaliveOutputTokens": 1,
                        "cacheKeepaliveReasoningTokens": 0,
                        "cacheKeepaliveHitTokens": 1_000_000,
                        "cacheKeepaliveMissTokens": 0
                    }
                }
            }
        });
        decorate_activity(&mut activity);
        assert_eq!(activity["cost"]["summary"]["requests"], 2);
        assert_eq!(
            activity["cost"]["cacheProtection"]["summary"]["requests"],
            1
        );
        assert_close(
            activity["cost"]["cacheProtection"]["summary"]["estimatedCny"]
                .as_f64()
                .unwrap(),
            0.02 + 0.000_002,
        );
    }
}
