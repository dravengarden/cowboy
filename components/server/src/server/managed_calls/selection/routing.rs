//! Review tiers from `models/routing.json` (docs/review-routing.md): which
//! reviewer preset a review gets automatically, and which explicit presets a
//! routine review may name. Rows match resolved model values, never preset
//! IDs, because one signed ID can resolve to different models per generation.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Table {
    schema: u16,
    pub evidence: String,
    #[allow(dead_code, reason = "documentation for table editors")]
    note: String,
    providers: BTreeMap<String, Tiers>,
    costs: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tiers {
    high: Tier,
    routine: Tier,
    #[serde(default)]
    economy: Option<Tier>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tier {
    auto: Vec<Row>,
    #[serde(default)]
    allow: Vec<Row>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    family: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    efforts: Vec<String>,
}

pub(super) static TABLE: LazyLock<Option<Table>> = LazyLock::new(|| {
    let table: Table = serde_json::from_str(include_str!("../../../../../../models/routing.json"))
        .inspect_err(|error| tracing::error!(%error, "review routing table is invalid"))
        .ok()?;
    (table.schema == 1).then_some(table)
});

/// The model a preset resolves to, without a context suffix such as `[1m]`.
pub(super) fn model(preset: &Value) -> Option<&str> {
    let model = preset["values"]["model"].as_str()?;
    Some(model.split_once('[').map_or(model, |(base, _)| base))
}

pub(super) fn effort(preset: &Value) -> &str {
    preset["values"]["reasoning_effort"]
        .as_str()
        .or(preset["values"]["effort"].as_str())
        .unwrap_or("default")
}

impl Row {
    fn matches(&self, preset: &Value) -> bool {
        let Some(model) = model(preset) else {
            return false;
        };
        super::family(model) == Some(self.family.as_str())
            && self
                .model
                .as_deref()
                .is_none_or(|expected| expected == model)
            && (self.efforts.is_empty() || self.efforts.iter().any(|e| e == effort(preset)))
    }
}

/// Which tier a fresh review resolves to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    High,
    Routine,
    Economy,
}

/// The automatic preset first, then every other preset the tier admits, in
/// `pool` (strongest-first) order. `None` when the table has no row for this
/// Provider or none of its automatic rows matches a runnable preset; the
/// caller then keeps the legacy order.
pub(super) fn order(
    table: &Table,
    provider: &str,
    level: Level,
    pool: &[Value],
) -> Option<Vec<Value>> {
    let tiers = table.providers.get(provider)?;
    let tier = match level {
        Level::High => &tiers.high,
        Level::Routine => &tiers.routine,
        Level::Economy => tiers.economy.as_ref().unwrap_or(&tiers.routine),
    };
    let automatic = tier
        .auto
        .iter()
        .find_map(|row| pool.iter().find(|preset| row.matches(preset)))?;
    let bounds = if level == Level::High {
        None
    } else {
        Some(&tiers.routine)
    };
    let admitted = |preset: &&Value| {
        bounds.is_none_or(|routine| {
            routine.allow.is_empty()
                || routine
                    .allow
                    .iter()
                    .chain(&routine.auto)
                    .chain(tier.auto.iter())
                    .any(|row| row.matches(preset))
        })
    };
    let mut ordered = vec![automatic.clone()];
    ordered.extend(
        pool.iter()
            .filter(admitted)
            .filter(|preset| preset["id"] != automatic["id"])
            .cloned(),
    );
    Some(ordered)
}

/// Measured cost of a preset's resolved model and effort, or `unknown`.
pub(super) fn cost(table: &Table, preset: &Value) -> Value {
    model(preset)
        .and_then(|model| table.costs.get(&format!("{model}:{}", effort(preset))))
        .cloned()
        .unwrap_or_else(|| json!("unknown"))
}
