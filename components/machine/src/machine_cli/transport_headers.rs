//! Upgrade headers advertising host-layer transport and broker capabilities.

use std::sync::Arc;
use std::time::Duration;

use super::{ComponentKind, ComponentStore};
use crate::machine_broker::{ManagedResumeProbe, probe_worker_managed_resume};
use crate::machine_plugins::MachinePluginStore;

async fn probe_once(
    components: &ComponentStore,
    providers: &MachinePluginStore,
) -> ManagedResumeProbe {
    let worker = components
        .active()
        .ok()
        .and_then(|active| {
            active
                .into_iter()
                .find(|(component, _)| component.id.kind == ComponentKind::AcpRuntime)
                .map(|(_, executable)| executable)
        })
        .or_else(|| providers.worker_command());
    match worker {
        Some(worker) => probe_worker_managed_resume(&worker).await,
        None => ManagedResumeProbe::Unsupported,
    }
}

/// Probe the Machine's effective worker executable for `managed-resume-v1`,
/// retrying once quickly if the first probe is inconclusive.
pub(super) async fn probe_effective_managed_resume(
    components: &ComponentStore,
    providers: &MachinePluginStore,
) -> ManagedResumeProbe {
    let first = probe_once(components, providers).await;
    if first != ManagedResumeProbe::Inconclusive {
        return first;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    probe_once(components, providers).await
}

/// Retry an inconclusive worker probe on a live connection and signal once
/// `Supported` is confirmed so the Machine can reconnect and refresh its header.
pub(super) fn spawn_inconclusive_resume_watcher(
    components: Arc<ComponentStore>,
    providers: Arc<MachinePluginStore>,
) -> tokio::sync::oneshot::Receiver<()> {
    let (mut tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        for delay in [5, 15, 30, 60, 60] {
            tokio::select! {
                () = tx.closed() => return,
                () = tokio::time::sleep(Duration::from_secs(delay)) => {}
            }
            match probe_once(&components, &providers).await {
                ManagedResumeProbe::Supported => {
                    let _ = tx.send(());
                    return;
                }
                ManagedResumeProbe::Unsupported => return,
                ManagedResumeProbe::Inconclusive => {}
            }
        }
    });
    rx
}

pub(super) async fn wait_refresh(refresh: &mut Option<tokio::sync::oneshot::Receiver<()>>) {
    match refresh {
        Some(rx) => {
            if rx.await.is_err() {
                *refresh = None;
                std::future::pending::<()>().await;
            }
        }
        None => std::future::pending::<()>().await,
    }
}

pub(super) fn attach(
    request: &mut tokio_tungstenite::tungstenite::http::Request<()>,
    managed_resume: bool,
) -> anyhow::Result<()> {
    request.headers_mut().insert(
        crate::machine_transport::HEADER,
        crate::machine_transport::CHUNKED.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::COMPRESSION_HEADER,
        crate::machine_transport::DEFLATE.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::PACING_HEADER,
        crate::machine_transport::CREDIT.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::STREAM_HEADER,
        crate::machine_transport::STREAM.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_lanes::LANES_HEADER,
        crate::machine_lanes::LANES.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::HIBERNATION_EPOCH_HEADER,
        crate::machine_transport::HIBERNATION_EPOCH_V1.parse()?,
    );
    if managed_resume {
        request.headers_mut().insert(
            crate::machine_transport::MANAGED_RESUME_HEADER,
            crate::machine_transport::MANAGED_RESUME_V1.parse()?,
        );
    }
    Ok(())
}
