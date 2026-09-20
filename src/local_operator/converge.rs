//! Fleet convergence for installed Plugins.
//!
//! Every individual step already had a command, but converging a fleet meant
//! reading the Catalog, comparing versions by eye and hand-writing an exact
//! version and digest per Plugin per Machine. That is where a release stops
//! being reproducible: the digest is the whole trust claim, and typing it is
//! the one place to get it wrong. This resolves each target from the Catalog it
//! just read, so no digest is ever authored.
//!
//! It converges to what the Catalog already advertises; it does not publish,
//! sign, or decide what "latest" should be. Publication stays the separate,
//! explicitly authorized act, and this command runs under the same host
//! delegation and the same durable installation transaction as one upgrade
//! submitted by hand.
//!
//! It fails closed. A dry run is the default. A Plugin holding an active
//! session lease is reported, never recycled under a live worker. A release
//! that is not `ready`, or that does not declare the Machine's platform, is
//! not a target. An installed version ahead of the Catalog is reported rather
//! than downgraded. Each upgrade gets a deterministic identity, so a retry
//! after a lost response reuses it and the Controller returns the saved result
//! instead of installing twice. The next Machine is attempted only after the
//! previous one's inventory proves it converged.

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CatalogRelease {
    pub plugin_id: String,
    pub plugin_version: String,
    #[serde(default)]
    pub artifact_digest: String,
    #[serde(default)]
    pub release_state: String,
    #[serde(default)]
    pub supported_platforms: Vec<SupportedPlatform>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SupportedPlatform {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub architecture: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct InstalledPlugin {
    pub plugin_id: String,
    #[serde(default)]
    pub plugin_version: String,
    #[serde(default)]
    pub active_session_leases: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct MachineTarget {
    pub id: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepKind {
    /// The Machine should run this Plugin and does not.
    Install,
    /// The Machine runs an older release than the Catalog's newest ready one.
    Upgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ConvergeStep {
    pub kind: StepKind,
    pub plugin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    pub digest: String,
    pub operation_id: String,
}

/// An installed Plugin the Service-side document does not declare, on a Machine
/// whose policy is to remove those.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct RemovalStep {
    pub plugin: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SkippedPlugin {
    pub plugin: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(super) struct MachinePlan {
    pub machine: String,
    pub steps: Vec<ConvergeStep>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removals: Vec<RemovalStep>,
    pub skipped: Vec<SkippedPlugin>,
}

/// Ordinal comparison over dot-separated numeric versions. A non-numeric part
/// compares as 0 rather than failing: an unexpected version string must not be
/// able to stop a fleet mid-convergence.
#[must_use]
pub(super) fn compare_versions(left: &str, right: &str) -> Ordering {
    let mut left = left.split('.');
    let mut right = right.split('.');
    loop {
        let (a, b) = (left.next(), right.next());
        if a.is_none() && b.is_none() {
            return Ordering::Equal;
        }
        let a: u64 = a.unwrap_or("0").parse().unwrap_or(0);
        let b: u64 = b.unwrap_or("0").parse().unwrap_or(0);
        match a.cmp(&b) {
            Ordering::Equal => {}
            other => return other,
        }
    }
}

/// The newest `ready` release per Plugin that declares this Machine's exact
/// platform. A release in another state, or without a matching platform, is
/// not a target: the Catalog is the authority on what may be installed, and an
/// undeclared platform is not evidence of compatibility.
#[must_use]
pub(super) fn latest_ready<'a>(
    releases: &'a [CatalogRelease],
    platform: &str,
    architecture: &str,
) -> BTreeMap<&'a str, &'a CatalogRelease> {
    let mut latest: BTreeMap<&str, &CatalogRelease> = BTreeMap::new();
    for release in releases {
        if release.release_state != "ready" || release.artifact_digest.is_empty() {
            continue;
        }
        if !release
            .supported_platforms
            .iter()
            .any(|supported| supported.os == platform && supported.architecture == architecture)
        {
            continue;
        }
        latest
            .entry(release.plugin_id.as_str())
            .and_modify(|current| {
                if compare_versions(&release.plugin_version, &current.plugin_version)
                    == Ordering::Greater
                {
                    *current = release;
                }
            })
            .or_insert(release);
    }
    latest
}

/// One stable identity per (Machine, Plugin, target version). Reusing it after
/// a lost response is observation, never a second installation.
#[must_use]
pub(super) fn operation_id(machine: &str, plugin: &str, version: &str) -> String {
    let sanitize = |value: &str| -> String {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect()
    };
    let id = format!(
        "{}-{}-{}-converge",
        sanitize(machine),
        sanitize(plugin),
        sanitize(version)
    );
    id.chars().take(128).collect()
}

/// Decide what one Machine needs. Pure: every input is an observation.
///
/// `declared` is the Service-side membership for this Machine. Without one the
/// Machine is unmanaged: its installed Plugins are kept current and nothing is
/// added or removed, because "which Plugins belong here" was never stated.
#[must_use]
pub(super) fn plan_machine(
    machine: &MachineTarget,
    releases: &[CatalogRelease],
    installed: &[InstalledPlugin],
    only: &[String],
    declared: Option<&crate::machine_convergence::MachinePlugins>,
) -> MachinePlan {
    let latest = latest_ready(releases, &machine.platform, &machine.architecture);
    let mut plan = MachinePlan {
        machine: machine.id.clone(),
        ..MachinePlan::default()
    };
    let selected = |plugin: &str| only.is_empty() || only.iter().any(|id| id == plugin);
    for plugin in installed {
        if !selected(&plugin.plugin_id) {
            continue;
        }
        if let Some(declared) =
            declared.filter(|declared| !declared.plugins.contains(&plugin.plugin_id))
        {
            match declared.unlisted {
                crate::machine_convergence::UnlistedPolicy::Uninstall => {
                    plan.removals.push(RemovalStep {
                        plugin: plugin.plugin_id.clone(),
                        version: plugin.plugin_version.clone(),
                    });
                }
                crate::machine_convergence::UnlistedPolicy::Keep => {
                    plan.skipped.push(SkippedPlugin {
                        plugin: plugin.plugin_id.clone(),
                        reason: "not declared for this Machine; unlisted Plugins are kept"
                            .to_owned(),
                    });
                }
            }
            continue;
        }
        let Some(target) = latest.get(plugin.plugin_id.as_str()) else {
            plan.skipped.push(SkippedPlugin {
                plugin: plugin.plugin_id.clone(),
                reason: format!(
                    "no ready {}/{} release in the Catalog",
                    machine.platform, machine.architecture
                ),
            });
            continue;
        };
        match compare_versions(&plugin.plugin_version, &target.plugin_version) {
            Ordering::Equal => continue,
            // Installed ahead of the Catalog is a real condition: a release was
            // withdrawn, or this Machine was installed from elsewhere.
            // Downgrading is never implied by "converge".
            Ordering::Greater => {
                plan.skipped.push(SkippedPlugin {
                    plugin: plugin.plugin_id.clone(),
                    reason: format!(
                        "installed {} is ahead of Catalog {}",
                        plugin.plugin_version, target.plugin_version
                    ),
                });
                continue;
            }
            Ordering::Less => {}
        }
        if plugin.active_session_leases > 0 {
            plan.skipped.push(SkippedPlugin {
                plugin: plugin.plugin_id.clone(),
                reason: format!(
                    "holds {} active session lease(s)",
                    plugin.active_session_leases
                ),
            });
            continue;
        }
        plan.steps.push(ConvergeStep {
            kind: StepKind::Upgrade,
            plugin: plugin.plugin_id.clone(),
            from: Some(plugin.plugin_version.clone()),
            to: target.plugin_version.clone(),
            digest: target.artifact_digest.clone(),
            operation_id: operation_id(&machine.id, &plugin.plugin_id, &target.plugin_version),
        });
    }
    let Some(declared) = declared else {
        return plan;
    };
    for plugin in &declared.plugins {
        if !selected(plugin) || installed.iter().any(|current| current.plugin_id == *plugin) {
            continue;
        }
        let Some(target) = latest.get(plugin.as_str()) else {
            plan.skipped.push(SkippedPlugin {
                plugin: plugin.clone(),
                reason: format!(
                    "declared but has no ready {}/{} release in the Catalog",
                    machine.platform, machine.architecture
                ),
            });
            continue;
        };
        plan.steps.push(ConvergeStep {
            kind: StepKind::Install,
            plugin: plugin.clone(),
            from: None,
            to: target.plugin_version.clone(),
            digest: target.artifact_digest.clone(),
            operation_id: operation_id(&machine.id, plugin, &target.plugin_version),
        });
    }
    plan
}

/// Order the fleet. An explicit `--machine` list is the rollout order and its
/// first entry is the canary; without one, every connected Machine converges
/// in registry order.
#[must_use]
pub(super) fn rollout_order(
    machines: &[MachineTarget],
    requested: &[String],
) -> (Vec<MachineTarget>, Vec<SkippedPlugin>) {
    let mut ordered = Vec::new();
    let mut skipped = Vec::new();
    if requested.is_empty() {
        for machine in machines {
            if machine.connected {
                ordered.push(machine.clone());
            } else {
                skipped.push(SkippedPlugin {
                    plugin: machine.id.clone(),
                    reason: "Machine is not connected".to_owned(),
                });
            }
        }
        return (ordered, skipped);
    }
    for id in requested {
        match machines.iter().find(|machine| machine.id == *id) {
            Some(machine) if machine.connected => ordered.push(machine.clone()),
            Some(_) => skipped.push(SkippedPlugin {
                plugin: id.clone(),
                reason: "Machine is not connected".to_owned(),
            }),
            None => skipped.push(SkippedPlugin {
                plugin: id.clone(),
                reason: "no such registered Machine".to_owned(),
            }),
        }
    }
    (ordered, skipped)
}

async fn read_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    segments: &[&str],
    body: Option<Value>,
) -> Result<(u16, Value)> {
    let mut url = reqwest::Url::parse("http://localhost/v1/")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|()| anyhow::anyhow!("invalid local endpoint"))?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request.send().await.context(
        "local Operator request failed; inspect the original operation before retrying a write",
    )?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context(
        "local Operator response was interrupted; inspect the original operation before retrying a write",
    )? {
        ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "local Operator response exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Ok((status, data))
}

fn decode<T: serde::de::DeserializeOwned>(value: Value, what: &str) -> Result<Vec<T>> {
    serde_json::from_value(value).with_context(|| format!("reading {what}"))
}

async fn catalog(client: &reqwest::Client) -> Result<Vec<CatalogRelease>> {
    let (status, data) = read_json(client, reqwest::Method::GET, &["plugins"], None).await?;
    ensure!(status == 200, "reading the Catalog returned HTTP {status}");
    decode(
        data.get("plugins").cloned().unwrap_or(Value::Null),
        "the Catalog",
    )
}

async fn machines(client: &reqwest::Client) -> Result<Vec<MachineTarget>> {
    let (status, data) = read_json(client, reqwest::Method::GET, &["machines"], None).await?;
    ensure!(
        status == 200,
        "reading the Machine registry returned HTTP {status}"
    );
    decode(data, "the Machine registry")
}

async fn inventory(client: &reqwest::Client, machine: &str) -> Result<Vec<InstalledPlugin>> {
    let (status, data) = read_json(
        client,
        reqwest::Method::GET,
        &["machines", machine, "plugins"],
        None,
    )
    .await?;
    ensure!(
        status == 200,
        "reading {machine}'s Plugin inventory returned HTTP {status}"
    );
    decode(data, "a Plugin inventory")
}

/// Remove one undeclared Plugin through the same durable transaction a
/// browser confirmation uses. The preview names every session the removal
/// would take with it; automation never confirms past one. A live conversation
/// is not ended to satisfy a list — the Plugin stays until it is idle.
async fn remove(
    client: &reqwest::Client,
    machine: &str,
    removal: &RemovalStep,
) -> Result<serde_json::Value> {
    let (status, preview) = read_json(
        client,
        reqwest::Method::POST,
        &[
            "machines",
            machine,
            "plugins",
            &removal.plugin,
            "uninstall-plan",
        ],
        None,
    )
    .await?;
    if status != 200 {
        return Ok(json!({
            "plugin": removal.plugin,
            "removed": false,
            "http_status": status,
            "detail": preview,
        }));
    }
    let affected = preview
        .get("affected_sessions")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    if affected > 0 {
        return Ok(json!({
            "plugin": removal.plugin,
            "removed": false,
            "reason": format!("{affected} session(s) would be removed with it"),
        }));
    }
    let Some(plan_id) = preview.get("plan_id").and_then(serde_json::Value::as_str) else {
        return Ok(json!({
            "plugin": removal.plugin,
            "removed": false,
            "reason": "the uninstall preview carried no identity",
        }));
    };
    let (status, data) = read_json(
        client,
        reqwest::Method::POST,
        &["machines", machine, "plugins", &removal.plugin, "uninstall"],
        Some(json!({ "plan_id": plan_id, "confirm_active_sessions": false })),
    )
    .await?;
    Ok(json!({
        "plugin": removal.plugin,
        "version": removal.version,
        "removed": (200..300).contains(&status),
        "http_status": status,
        "detail": data,
    }))
}

/// Converge the requested Machines and report exactly what happened. Returns an
/// error — and therefore a non-zero exit — when an applied run leaves work
/// behind, so an unattended caller cannot read silence as success.
pub(super) async fn run(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
    requested: &[String],
    only: &[String],
    apply: bool,
) -> Result<()> {
    let releases = catalog(client).await?;
    let registry = machines(client).await?;
    let declared = crate::machine_convergence::PluginDesiredSource::new(data_dir).current();
    let (ordered, mut unreachable) = rollout_order(&registry, requested);
    let mut reports = Vec::new();
    let mut stopped_at: Option<String> = None;
    for machine in &ordered {
        if stopped_at.is_some() {
            reports.push(json!({
                "machine": machine.id,
                "attempted": false,
                "reason": "an earlier Machine in this rollout did not converge",
            }));
            continue;
        }
        let installed = inventory(client, &machine.id).await?;
        let membership = declared.value.machine(&machine.id);
        let plan = plan_machine(machine, &releases, &installed, only, membership);
        let mut applied = Vec::new();
        let mut removed = Vec::new();
        for step in &plan.steps {
            if !apply {
                continue;
            }
            let (status, data) = read_json(
                client,
                reqwest::Method::POST,
                &["machines", &machine.id, "plugins", &step.plugin, "install"],
                Some(json!({
                    "version": step.to,
                    "digest": step.digest,
                    "operation_id": step.operation_id,
                })),
            )
            .await?;
            applied.push(json!({
                "kind": step.kind,
                "plugin": step.plugin,
                "from": step.from,
                "to": step.to,
                "operation_id": step.operation_id,
                "http_status": status,
                "detail": data,
            }));
        }
        for removal in &plan.removals {
            if !apply {
                continue;
            }
            removed.push(remove(client, &machine.id, removal).await?);
        }
        // Re-read the inventory rather than trusting the submissions: an
        // accepted request and an installed generation are different facts.
        let remaining = if apply {
            plan_machine(
                machine,
                &releases,
                &inventory(client, &machine.id).await?,
                only,
                membership,
            )
        } else {
            plan.clone()
        };
        // A removal the Machine refused because a session would be affected is
        // reported, not a rollout failure: the next run removes it once that
        // conversation ends.
        if apply && !remaining.steps.is_empty() {
            stopped_at = Some(machine.id.clone());
        }
        reports.push(json!({
            "machine": machine.id,
            "attempted": true,
            "managed": membership.is_some(),
            "planned": plan.steps,
            "applied": applied,
            "planned_removals": plan.removals,
            "removed": removed,
            "skipped": plan.skipped,
            "remaining": remaining
                .steps
                .iter()
                .map(|step| match &step.from {
                    Some(from) => format!("{} {from}→{}", step.plugin, step.to),
                    None => format!("{} install {}", step.plugin, step.to),
                })
                .chain(
                    remaining
                        .removals
                        .iter()
                        .map(|removal| format!("{} remove {}", removal.plugin, removal.version)),
                )
                .collect::<Vec<_>>(),
        }));
    }
    unreachable.sort_by(|left, right| left.plugin.cmp(&right.plugin));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": 1,
            "applied": apply,
            "machines": reports,
            "unreachable": unreachable,
            "stopped_at": stopped_at,
        }))?
    );
    if let Some(machine) = stopped_at {
        bail!(
            "{machine} did not converge; inspect its receipts before repeating or publishing anything else"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
