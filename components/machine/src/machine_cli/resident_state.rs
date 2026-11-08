//! Adapter projections of the resident broker's evidence.
use super::*;

/// Content-free counts and writer flags; no Session, lineage or path.
pub(super) fn durable_state_reply(
    payload: &serde_json::Value,
    view: &crate::machine_broker::DurableStateView,
) -> anyhow::Result<serde_json::Value> {
    anyhow::ensure!(
        payload.as_object().is_some_and(serde_json::Map::is_empty),
        "invalid durable state request"
    );
    let status = view
        .status()
        .context("durable state is not available yet")?;
    serde_json::to_value(status).context("encoding durable state")
}

/// Retention includes every resident declaration's exact Provider generation,
/// even when it differs from the Controller's current release. Without broker
/// evidence, refuse instead of guessing what a surviving worker uses.
pub(super) fn resident_provider_generations(
    view: &crate::machine_broker::DurableStateView,
) -> anyhow::Result<BTreeMap<String, BTreeSet<String>>> {
    view.resident_provider_generations()
        .context("generation retention needs the Machine broker's Session registry")
}
