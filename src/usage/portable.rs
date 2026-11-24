//! Named portable account usage: which released signed collector serves an
//! account scope, where it can run, and the Machine dispatch. The Controller
//! never runs it (no bootstrap fallback) and never sees a credential.

use super::{PluginUsageSpec, UsageService};
use crate::machine_control::{MachineControl, Reply, ReplyKind, RequestBinding};
use crate::machine_protocol::portable_usage::{
    PortableRenewalRequest, PortableSidecar, PortableUsageRequest, RENEWAL_OUTCOMES,
    portable_usage_error_code,
};
use crate::machine_protocol::{
    MachineCommand, PluginHostOperation, PluginInstallationState, PluginInventory,
};
use serde_json::Value;

/// A little above the Machine's admission wait plus its collector timeout.
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// The signed collector serving one portable account scope.
pub struct PortableUsageHost {
    /// Presentation binding: usage account, product and parser.
    pub binding: PluginUsageSpec,
    /// Scope member whose host owns the collector.
    pub plugin_id: String,
    /// Connected Machines holding an exact released active generation of it,
    /// and whether that release's collector declares it never redeems a
    /// refresh token on request.
    pub candidates: Vec<(String, PluginInventory, bool)>,
}

impl UsageService {
    /// Whether exactly this installed release's collector declares that its
    /// banked reset never redeems a refresh token.
    #[must_use]
    pub fn refresh_free_reset(&self, plugin: &PluginInventory) -> bool {
        let catalog = match &self.bindings {
            super::UsageBindingSource::Catalog(catalog) => catalog,
            #[cfg(any(test, feature = "test-support"))]
            super::UsageBindingSource::Static(_) => return false,
        };
        catalog.runtime().is_some_and(|runtime| {
            runtime
                .exact_host(
                    &plugin.plugin_id,
                    &plugin.plugin_version,
                    &plugin.generation_digest,
                )
                .is_some_and(|host| host.refresh_free_reset)
        })
    }

    /// Whether exactly this installed release's collector declares an idle
    /// renewal (`PortableRenewalRequest`).
    #[must_use]
    pub fn idle_renewal(&self, plugin: &PluginInventory) -> bool {
        let catalog = match &self.bindings {
            super::UsageBindingSource::Catalog(catalog) => catalog,
            #[cfg(any(test, feature = "test-support"))]
            super::UsageBindingSource::Static(_) => return false,
        };
        catalog.runtime().is_some_and(|runtime| {
            runtime
                .exact_host(
                    &plugin.plugin_id,
                    &plugin.plugin_version,
                    &plugin.generation_digest,
                )
                .is_some_and(|host| host.idle_renewal)
        })
    }

    /// The first scope member (in `members` order) whose released host
    /// declares a usage collector. Source-bundled bootstrap hosts never
    /// qualify: they would run on the Controller.
    #[must_use]
    pub fn portable_usage_host(&self, members: &[String]) -> Option<PortableUsageHost> {
        let catalog = match &self.bindings {
            super::UsageBindingSource::Catalog(catalog) => catalog,
            #[cfg(any(test, feature = "test-support"))]
            super::UsageBindingSource::Static(_) => return None,
        };
        let runtime = catalog.runtime()?;
        let installed = self.machine_control.connected_plugin_inventory();
        members.iter().find_map(|member| {
            let presentation = runtime
                .default_host(member)
                .and_then(|host| host.usage.clone())
                .filter(|usage| !usage.collector_argv.is_empty())?;
            let hosts: Vec<_> = runtime
                .exact_usage_hosts(&presentation.account)
                .into_iter()
                .filter(|host| {
                    &host.id == member
                        && host
                            .usage
                            .as_ref()
                            .is_some_and(|usage| !usage.collector_argv.is_empty())
                })
                .collect();
            let candidates = installed
                .iter()
                .filter(|installed| installed.plugin.state == PluginInstallationState::Active)
                .filter_map(|installed| {
                    let host = hosts.iter().find(|host| {
                        host.id == installed.plugin.plugin_id
                            && host.plugin_version.as_deref()
                                == Some(installed.plugin.plugin_version.as_str())
                            && host.artifact_digest.as_deref()
                                == Some(installed.plugin.generation_digest.as_str())
                    })?;
                    Some((
                        installed.machine_id.clone(),
                        installed.plugin.clone(),
                        host.refresh_free_collector,
                    ))
                })
                .collect();
            Some(PortableUsageHost {
                binding: presentation,
                plugin_id: member.clone(),
                candidates,
            })
        })
    }

    /// The sidecar Plugin generations the exact `collector` release starts on
    /// `machine_id`: for each sidecar adapter slot its signed usage binding
    /// declares, the scope member installed there whose exact release fills
    /// that slot. `None` when a declared slot has no such member. Nothing is
    /// taken from the catalog's presentation default.
    #[must_use]
    pub fn exact_sidecars(
        &self,
        machine_id: &str,
        collector: &PluginInventory,
        members: &[String],
    ) -> Option<Vec<PortableSidecar>> {
        let catalog = match &self.bindings {
            super::UsageBindingSource::Catalog(catalog) => catalog,
            #[cfg(any(test, feature = "test-support"))]
            super::UsageBindingSource::Static(_) => return None,
        };
        let runtime = catalog.runtime()?;
        let usage = runtime
            .exact_host(
                &collector.plugin_id,
                &collector.plugin_version,
                &collector.generation_digest,
            )?
            .usage
            .clone()?;
        let installed = self.machine_control.connected_plugin_inventory();
        let mut sidecars = Vec::new();
        for slot in usage
            .collector_sidecars
            .iter()
            .map(|sidecar| sidecar.adapter_slot.as_str())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let found = installed.iter().find_map(|candidate| {
                let plugin = &candidate.plugin;
                (candidate.machine_id == machine_id
                    && plugin.state == PluginInstallationState::Active
                    && members.contains(&plugin.plugin_id))
                .then(|| {
                    runtime.exact_host(
                        &plugin.plugin_id,
                        &plugin.plugin_version,
                        &plugin.generation_digest,
                    )
                })
                .flatten()
                .filter(|host| host.adapter_slot.as_deref() == Some(slot))
                .map(|_| PortableSidecar {
                    plugin_id: plugin.plugin_id.clone(),
                    generation_digest: plugin.generation_digest.clone(),
                })
            })?;
            sidecars.push(found);
        }
        Some(sidecars)
    }

    /// The usage binding of exactly this installed release: what its signed
    /// collector supports (resets, claim policy), never the presentation
    /// default of a newer or older release.
    #[must_use]
    pub fn exact_usage_binding(&self, plugin: &PluginInventory) -> Option<PluginUsageSpec> {
        let catalog = match &self.bindings {
            super::UsageBindingSource::Catalog(catalog) => catalog,
            #[cfg(any(test, feature = "test-support"))]
            super::UsageBindingSource::Static(_) => return None,
        };
        catalog
            .runtime()?
            .exact_host(
                &plugin.plugin_id,
                &plugin.plugin_version,
                &plugin.generation_digest,
            )?
            .usage
            .clone()
    }
}

/// Run the account's collector on `machine_id` in the account's replica
/// scope at `auth_generation`. Errors are closed reason codes.
pub async fn collect(
    machine_control: &MachineControl,
    machine_id: &str,
    plugin: &PluginInventory,
    request: PortableUsageRequest,
) -> Result<Value, &'static str> {
    begin_collect(machine_control, machine_id, plugin, request)?.await
}

/// Enqueue a collection (synchronously, so a caller can hold an admission
/// lock across exactly the enqueue); the future awaits its reply.
pub fn begin_collect<'a>(
    machine_control: &'a MachineControl,
    machine_id: &str,
    plugin: &PluginInventory,
    request: PortableUsageRequest,
) -> Result<impl Future<Output = Result<Value, &'static str>> + 'a, &'static str> {
    let binding = machine_control
        .bind_plugin_host(machine_id, plugin, PluginHostOperation::CollectUsage)
        .map_err(|_| "portable_usage_machine_unavailable")?;
    let request_id = format!("portable-usage-{}", uuid::Uuid::new_v4());
    let (reply, pending) = machine_control
        .begin_request(
            machine_id,
            &request_id,
            MachineCommand::CollectPortableUsage {
                request_id: request_id.clone(),
                request: Box::new(request),
            },
            ReplyKind::PluginHost,
            Some(RequestBinding::Plugin(&binding)),
        )
        .map_err(|error| dispatch_error(&error))?;
    Ok(async move {
        let _pending = pending;
        match tokio::time::timeout(REPLY_TIMEOUT, reply).await {
            Ok(Ok(Reply::PluginHost {
                accepted: true,
                payload: Some(value),
                ..
            })) if !value.is_null() => Ok(value),
            Ok(Ok(Reply::PluginHost {
                detail: Some(code), ..
            })) => Err(portable_usage_error_code(&code)),
            Ok(Err(_)) => Err("portable_usage_machine_unavailable"),
            Err(_) => Err("portable_usage_temporarily_unavailable"),
            Ok(Ok(_)) => Err("portable_usage_collector_failed"),
        }
    })
}

/// Enqueue an idle renewal (synchronously, like [`begin_collect`]); the
/// future awaits its closed outcome (`renewed` or `unchanged`).
pub fn begin_renewal<'a>(
    machine_control: &'a MachineControl,
    machine_id: &str,
    plugin: &PluginInventory,
    request: PortableRenewalRequest,
) -> Result<impl Future<Output = Result<&'static str, &'static str>> + 'a, &'static str> {
    let binding = machine_control
        .bind_plugin_host(machine_id, plugin, PluginHostOperation::RenewCredential)
        .map_err(|_| "portable_usage_machine_unavailable")?;
    let request_id = format!("portable-renewal-{}", uuid::Uuid::new_v4());
    let (reply, pending) = machine_control
        .begin_request(
            machine_id,
            &request_id,
            MachineCommand::RenewPortableCredential {
                request_id: request_id.clone(),
                request: Box::new(request),
            },
            ReplyKind::PluginHost,
            Some(RequestBinding::Plugin(&binding)),
        )
        .map_err(|error| dispatch_error(&error))?;
    Ok(async move {
        let _pending = pending;
        match tokio::time::timeout(REPLY_TIMEOUT, reply).await {
            Ok(Ok(Reply::PluginHost {
                accepted: true,
                payload: Some(value),
                ..
            })) => RENEWAL_OUTCOMES
                .into_iter()
                .find(|outcome| value["outcome"].as_str() == Some(outcome))
                .ok_or("portable_usage_collector_failed"),
            Ok(Ok(Reply::PluginHost {
                detail: Some(code), ..
            })) => Err(portable_usage_error_code(&code)),
            Ok(Err(_)) => Err("portable_usage_machine_unavailable"),
            Err(_) => Err("portable_usage_temporarily_unavailable"),
            Ok(Ok(_)) => Err("portable_usage_collector_failed"),
        }
    })
}

/// Closed codes for refusals before the command left the Controller.
#[must_use]
pub fn dispatch_error(error: &str) -> &'static str {
    match error {
        "Machine pending request budget exceeded" => "machine_usage_busy",
        "Machine is not connected" | "Machine disconnected" => "portable_usage_machine_unavailable",
        "Machine operation connection is no longer current" => "machine_connection_changed",
        _ if error.starts_with("Machine negotiated protocol") => "machine_usage_update_required",
        _ => "portable_usage_dispatch_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_refusals_are_closed() {
        for (error, code) in [
            (
                "Machine negotiated protocol 48, but this command requires 50",
                "machine_usage_update_required",
            ),
            (
                "Machine is not connected",
                "portable_usage_machine_unavailable",
            ),
            (
                "Machine pending request budget exceeded",
                "machine_usage_busy",
            ),
            (
                "Plugin host installation changed before dispatch",
                "portable_usage_dispatch_failed",
            ),
        ] {
            assert_eq!(dispatch_error(error), code);
        }
    }

    fn plugin() -> PluginInventory {
        serde_json::from_value(serde_json::json!({
            "plugin_id": "codex", "plugin_version": "1.0.0",
            "generation_digest": "sha256:x", "contract_fingerprint": "c", "state": "active",
            "auth_generation": 1,
        }))
        .unwrap()
    }

    fn request() -> PortableUsageRequest {
        serde_json::from_value(serde_json::json!({
            "account_id": "11111111-1111-4111-8111-111111111111", "plugin_id": "codex",
            "plugin_version": "1.0.0", "generation_digest": "sha256:x",
            "auth_generation": 3, "payload": {"operation": "collect"},
        }))
        .unwrap()
    }

    fn connect(
        control: &MachineControl,
        protocol: u16,
    ) -> (
        crate::machine_control::ConnectionToken,
        tokio::sync::mpsc::UnboundedReceiver<MachineCommand>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = control.install("ovh".into(), "epoch".into(), false, protocol, tx);
        control.record_remote(
            &connection,
            crate::machine_protocol::MachineEvent::PluginInventory {
                plugins: vec![plugin()],
                observed_at_ms: 1,
            },
        );
        (connection, rx)
    }

    #[tokio::test]
    async fn an_old_or_absent_machine_is_refused_before_the_command_leaves() {
        let control = MachineControl::default();
        assert_eq!(
            collect(&control, "ovh", &plugin(), request()).await,
            Err("portable_usage_machine_unavailable")
        );
        let (_connection, mut commands) = connect(
            &control,
            crate::machine_protocol::ACCOUNT_FENCE_PROTOCOL_VERSION,
        );
        assert_eq!(
            collect(&control, "ovh", &plugin(), request()).await,
            Err("machine_usage_update_required")
        );
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_account_scoped_request_is_sent_and_only_closed_codes_return() {
        let control = std::sync::Arc::new(MachineControl::default());
        for (reply, expected) in [
            (
                (true, Some(serde_json::json!({"status": "available"})), None),
                Ok(serde_json::json!({"status": "available"})),
            ),
            (
                (false, None, Some("portable_usage_replica_pending")),
                Err("portable_usage_replica_pending"),
            ),
            (
                (false, None, Some("collector stderr with /home/x/auth.json")),
                Err("portable_usage_collector_failed"),
            ),
        ] {
            let (connection, mut commands) = connect(
                &control,
                crate::machine_protocol::PORTABLE_USAGE_PROTOCOL_VERSION,
            );
            let requester = std::sync::Arc::clone(&control);
            let pending =
                tokio::spawn(async move { collect(&requester, "ovh", &plugin(), request()).await });
            let Some(MachineCommand::CollectPortableUsage {
                request_id,
                request: sent,
            }) = commands.recv().await
            else {
                panic!("portable usage command expected");
            };
            assert_eq!(*sent, request());
            let (accepted, payload, detail) = reply;
            control.record_remote(
                &connection,
                crate::machine_protocol::MachineEvent::PluginHostResponse {
                    request_id,
                    accepted,
                    started: true,
                    payload,
                    detail: detail.map(str::to_owned),
                },
            );
            assert_eq!(pending.await.unwrap(), expected);
        }
        let command = MachineCommand::CollectPortableUsage {
            request_id: "r".into(),
            request: Box::new(request()),
        };
        assert_eq!(
            command.minimum_protocol(),
            crate::machine_protocol::PORTABLE_USAGE_PROTOCOL_VERSION
        );
    }
}
