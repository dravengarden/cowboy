//! Continuous convergence of the Controller's signed desired component set.
//!
//! The manifest is the only authority for what a Machine may run: it names an
//! exact version, digest, signature and readiness probe, and the Machine still
//! verifies every one of them before an activation. This module decides *when*
//! that already-authorized work is dispatched, so a deployed component reaches
//! a Machine that is already connected instead of waiting for its next welcome
//! or for somebody to press a button.
//!
//! Three properties keep unattended convergence honest:
//!
//! - only `automatic` components converge, and the Machine refuses an automatic
//!   component that carries no health probe;
//! - a component with live leases drains first, so an update never replaces
//!   runtime bytes under a session that is using them; and
//! - repeated failure backs off and then stops, leaving a reported reason
//!   rather than an endless retry against the same broken digest.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::machine_protocol::{
    ComponentConvergence, ComponentConvergenceState, ComponentId, ComponentState, DesiredComponent,
    MachineSummary,
};

/// One accepted revision of the desired component set.
#[derive(Debug, Default)]
pub struct DesiredComponents {
    /// Monotonic within one Controller process; 0 means "nothing configured".
    pub generation: u64,
    pub components: Vec<DesiredComponent>,
    pub loaded_at_ms: i64,
    /// Why the most recent reload was rejected, if it was. The components above
    /// remain the last accepted set.
    pub error: Option<String>,
}

impl DesiredComponents {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, DesiredComponent> {
        self.components.iter()
    }

    #[must_use]
    pub fn find(&self, id: &ComponentId) -> Option<&DesiredComponent> {
        self.components.iter().find(|component| component.id == *id)
    }

    /// The subset a Machine may reconcile without anyone asking for it.
    #[must_use]
    pub fn automatic(&self) -> Vec<DesiredComponent> {
        self.components
            .iter()
            .filter(|component| component.automatic)
            .cloned()
            .collect()
    }
}

/// The configured manifest, re-read in place so a published component reaches
/// the fleet without restarting the Controller.
pub struct DesiredComponentSource {
    path: Option<PathBuf>,
    current: parking_lot::RwLock<Arc<DesiredComponents>>,
}

impl DesiredComponentSource {
    /// Read the configured manifest once. A Controller without a manifest keeps
    /// an empty set and converges nothing.
    pub fn load(path: Option<&Path>, now_ms: i64) -> anyhow::Result<Self> {
        let source = Self {
            path: path.map(Path::to_path_buf),
            current: parking_lot::RwLock::new(Arc::new(DesiredComponents::default())),
        };
        if let Some(path) = path {
            let bytes = std::fs::read(path).map_err(|error| {
                anyhow::anyhow!(
                    "reading Machine component manifest {}: {error}",
                    path.display()
                )
            })?;
            let components = parse_manifest(&bytes)
                .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))?;
            *source.current.write() = Arc::new(DesiredComponents {
                generation: 1,
                components,
                loaded_at_ms: now_ms,
                error: None,
            });
        }
        Ok(source)
    }

    #[must_use]
    pub fn current(&self) -> Arc<DesiredComponents> {
        Arc::clone(&self.current.read())
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.path.is_some()
    }

    /// Re-read the manifest. Returns the new generation when the accepted set
    /// changed. A missing, unreadable or invalid manifest keeps the previous
    /// accepted set and records why: an operator error must never widen into an
    /// unmanaged fleet or a half-applied desired state.
    pub fn reload(&self, now_ms: i64) -> Result<Option<u64>, String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(None);
        };
        let outcome = std::fs::read(path)
            .map_err(|error| format!("reading {}: {error}", path.display()))
            .and_then(|bytes| parse_manifest(&bytes));
        let mut current = self.current.write();
        match outcome {
            Ok(components) => {
                if components == current.components {
                    if current.error.is_some() {
                        *current = Arc::new(DesiredComponents {
                            generation: current.generation,
                            components,
                            loaded_at_ms: now_ms,
                            error: None,
                        });
                    }
                    return Ok(None);
                }
                let generation = current.generation.saturating_add(1);
                *current = Arc::new(DesiredComponents {
                    generation,
                    components,
                    loaded_at_ms: now_ms,
                    error: None,
                });
                Ok(Some(generation))
            }
            Err(error) => {
                *current = Arc::new(DesiredComponents {
                    generation: current.generation,
                    components: current.components.clone(),
                    loaded_at_ms: current.loaded_at_ms,
                    error: Some(error.clone()),
                });
                Err(error)
            }
        }
    }
}

/// Accept a manifest only when every record is independently installable. The
/// Machine repeats each of these checks against the bytes it downloads; the
/// Controller refuses first so an unusable record cannot sit in the desired
/// state looking like pending work.
pub fn parse_manifest(bytes: &[u8]) -> Result<Vec<DesiredComponent>, String> {
    let components: Vec<DesiredComponent> =
        serde_json::from_slice(bytes).map_err(|error| format!("parsing manifest: {error}"))?;
    let mut seen: HashSet<ComponentId> = HashSet::new();
    for component in &components {
        let id = &component.id;
        if !seen.insert(id.clone()) {
            return Err(format!("{}: duplicate component", component_label(id)));
        }
        if component.version.trim().is_empty() {
            return Err(format!("{}: empty version", component_label(id)));
        }
        if component
            .signature
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(format!("{}: unsigned component", component_label(id)));
        }
        if component.digest.len() != 64 || !component.digest.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(format!(
                "{}: digest must be a SHA-256 hex digest",
                component_label(id)
            ));
        }
        let url = reqwest::Url::parse(&component.artifact_url)
            .map_err(|error| format!("{}: artifact URL: {error}", component_label(id)))?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(format!("{}: artifact must use HTTPS", component_label(id)));
        }
        if component.automatic && component.probe.is_none() {
            return Err(format!(
                "{}: automatic activation requires a health probe",
                component_label(id)
            ));
        }
    }
    Ok(components)
}

fn component_label(id: &ComponentId) -> String {
    if id.slot.is_empty() {
        format!("{:?}", id.kind)
    } else {
        format!("{:?}/{}", id.kind, id.slot)
    }
}

/// How hard the Controller tries before it stops and reports.
#[derive(Debug, Clone, Copy)]
pub struct ConvergencePolicy {
    pub first_backoff: Duration,
    pub max_backoff: Duration,
    /// Consecutive failures against one digest before convergence gives up.
    pub max_attempts: u32,
}

impl Default for ConvergencePolicy {
    fn default() -> Self {
        Self {
            first_backoff: Duration::from_secs(60),
            max_backoff: Duration::from_secs(30 * 60),
            max_attempts: 4,
        }
    }
}

/// What one Machine's convergence should do right now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Exact signed records to reconcile now.
    pub dispatch: Vec<DesiredComponent>,
    /// Everything not dispatched, with the reason, for the Machine snapshot.
    pub reported: Vec<ComponentConvergence>,
}

/// One (machine, component) convergence history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The digest this history is about. A newer digest starts over.
    pub digest: String,
    pub attempts: u32,
    pub next_attempt_at_ms: i64,
    pub blocked: bool,
    /// The last dispatch was acknowledged by the Machine. Convergence itself is
    /// still proved by the next inventory, never by the acknowledgement.
    pub acknowledged: bool,
    pub detail: Option<String>,
}

impl Attempt {
    fn first(component: &DesiredComponent, now_ms: i64) -> Self {
        Self {
            digest: component.digest.clone(),
            attempts: 0,
            next_attempt_at_ms: now_ms,
            blocked: false,
            acknowledged: false,
            detail: None,
        }
    }
}

/// Decide the next convergence step for one Machine. Pure: every input is an
/// observation, and the result is what the caller may dispatch plus what the
/// user should be told about the rest.
#[must_use]
pub fn plan_machine(
    summary: &MachineSummary,
    desired: &DesiredComponents,
    attempts: &HashMap<ComponentId, Attempt>,
    now_ms: i64,
) -> Plan {
    let mut plan = Plan::default();
    if !summary.connected {
        return plan;
    }
    for component in desired.iter().filter(|component| component.automatic) {
        let installed = summary
            .components
            .iter()
            .find(|current| current.id == component.id);
        let converged = installed.is_some_and(|current| {
            current.state == ComponentState::Active
                && current.digest.eq_ignore_ascii_case(&component.digest)
        });
        if converged {
            continue;
        }
        let attempt = attempts
            .get(&component.id)
            .filter(|attempt| attempt.digest.eq_ignore_ascii_case(&component.digest));
        if let Some(attempt) = attempt.filter(|attempt| attempt.blocked) {
            plan.reported.push(ComponentConvergence {
                id: component.id.clone(),
                state: ComponentConvergenceState::Blocked,
                attempts: attempt.attempts,
                next_attempt_at_ms: None,
                detail: attempt.detail.clone(),
            });
            continue;
        }
        // A leased generation is in use by a live session. Automatic
        // convergence waits for it to drain; replacing it is an explicit
        // maintenance decision, not something a background loop may take.
        if installed.is_some_and(|current| current.active_leases > 0) {
            plan.reported.push(ComponentConvergence {
                id: component.id.clone(),
                state: ComponentConvergenceState::Draining,
                attempts: attempt.map_or(0, |attempt| attempt.attempts),
                next_attempt_at_ms: None,
                detail: None,
            });
            continue;
        }
        if let Some(attempt) = attempt.filter(|attempt| attempt.next_attempt_at_ms > now_ms) {
            plan.reported.push(ComponentConvergence {
                id: component.id.clone(),
                state: if attempt.acknowledged {
                    ComponentConvergenceState::Verifying
                } else {
                    ComponentConvergenceState::Retrying
                },
                attempts: attempt.attempts,
                next_attempt_at_ms: Some(attempt.next_attempt_at_ms),
                detail: attempt.detail.clone(),
            });
            continue;
        }
        plan.reported.push(ComponentConvergence {
            id: component.id.clone(),
            state: ComponentConvergenceState::Pending,
            attempts: attempt.map_or(0, |attempt| attempt.attempts),
            next_attempt_at_ms: None,
            detail: None,
        });
        plan.dispatch.push(component.clone());
    }
    plan
}

/// Per-Machine convergence history and its in-flight guard. Shared by the
/// convergence loop and the Machine snapshot projection.
#[derive(Default)]
pub struct ConvergenceState {
    attempts: parking_lot::Mutex<HashMap<String, HashMap<ComponentId, Attempt>>>,
    in_flight: parking_lot::Mutex<HashSet<String>>,
}

impl ConvergenceState {
    #[must_use]
    pub fn attempts(&self, machine_id: &str) -> HashMap<ComponentId, Attempt> {
        self.attempts
            .lock()
            .get(machine_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Claim this Machine's convergence slot. One Reconcile at a time per
    /// Machine: a second dispatch could race the first one's activation.
    pub fn begin(&self, machine_id: &str) -> bool {
        self.in_flight.lock().insert(machine_id.to_owned())
    }

    pub fn finish(&self, machine_id: &str) {
        self.in_flight.lock().remove(machine_id);
    }

    #[must_use]
    pub fn is_in_flight(&self, machine_id: &str) -> bool {
        self.in_flight.lock().contains(machine_id)
    }

    /// Count a dispatch before it is sent. An acknowledged Reconcile that does
    /// not converge must still back off: the acknowledgement says the Machine
    /// handled the command, not that the component became active.
    pub fn record_dispatch(
        &self,
        machine_id: &str,
        dispatched: &[DesiredComponent],
        policy: ConvergencePolicy,
        now_ms: i64,
    ) {
        let mut attempts = self.attempts.lock();
        let machine = attempts.entry(machine_id.to_owned()).or_default();
        for component in dispatched {
            let entry = machine
                .entry(component.id.clone())
                .or_insert_with(|| Attempt::first(component, now_ms));
            if !entry.digest.eq_ignore_ascii_case(&component.digest) {
                *entry = Attempt::first(component, now_ms);
            }
            entry.attempts = entry.attempts.saturating_add(1);
            entry.blocked = entry.attempts >= policy.max_attempts;
            entry.acknowledged = false;
            entry.next_attempt_at_ms = now_ms.saturating_add(
                i64::try_from(backoff(policy, entry.attempts).as_millis()).unwrap_or(i64::MAX),
            );
        }
    }

    /// Record how the dispatch itself ended: `None` for an acknowledged
    /// command, otherwise why it failed. Neither outcome retires the attempt;
    /// only an inventory that shows the exact digest active does.
    pub fn record_outcome(
        &self,
        machine_id: &str,
        dispatched: &[DesiredComponent],
        detail: Option<&str>,
    ) {
        let mut attempts = self.attempts.lock();
        let Some(machine) = attempts.get_mut(machine_id) else {
            return;
        };
        for component in dispatched {
            let Some(entry) = machine.get_mut(&component.id) else {
                continue;
            };
            if entry.digest.eq_ignore_ascii_case(&component.digest) {
                entry.acknowledged = detail.is_none();
                entry.detail = detail.map(str::to_owned);
            }
        }
    }

    /// Forget the history of components that converged or left the desired set.
    /// Only a connected Machine may prune: an offline Machine reports nothing,
    /// and reconnecting must not become a way to clear a blocked digest.
    pub fn retain(&self, machine_id: &str, keep: &HashSet<ComponentId>) {
        let mut attempts = self.attempts.lock();
        let Some(machine) = attempts.get_mut(machine_id) else {
            return;
        };
        machine.retain(|id, _| keep.contains(id));
        if machine.is_empty() {
            attempts.remove(machine_id);
        }
    }

    /// Drop a Machine's history. Used when it is revoked; a reconnect keeps its
    /// history so a failing digest cannot be retried forever by reconnecting.
    pub fn forget(&self, machine_id: &str) {
        self.attempts.lock().remove(machine_id);
        self.in_flight.lock().remove(machine_id);
    }
}

#[must_use]
pub fn backoff(policy: ConvergencePolicy, attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(16);
    let scaled = policy
        .first_backoff
        .saturating_mul(2_u32.saturating_pow(exponent));
    scaled.min(policy.max_backoff)
}

#[cfg(test)]
mod tests;
