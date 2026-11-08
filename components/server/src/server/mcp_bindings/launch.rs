//! Session-local receipt ownership. Cached metadata never substitutes for policy.
use super::{AppState, Placement, Stored};
use crate::{
    core::SessionMeta,
    machine_control::{ConnectionToken, NativeMcpSessionPreparation},
};
use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};

struct Context<'a> {
    hub: &'a crate::core::Hub,
    policies: &'a super::Policies,
    control: &'a Arc<crate::machine_control::MachineControl>,
    runtimes: &'a crate::runtime_router::RuntimeRouter,
    service: &'a str,
    shutdown: &'a tokio::sync::watch::Receiver<bool>,
}

impl<'a> From<&'a AppState> for Context<'a> {
    fn from(state: &'a AppState) -> Self {
        Self {
            hub: &state.hub,
            policies: &state.extensions.bindings,
            control: &state.machine_control,
            runtimes: &state.runtime_router,
            service: &state.service_id,
            shutdown: &state.shutdown,
        }
    }
}

#[derive(Default)]
pub(in crate::server) struct Launches {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    preparation: tokio::sync::Mutex<()>,
}

struct Scope {
    session: String,
    stored: Stored,
    runtime: ConnectionToken,
    execution: ConnectionToken,
    provider: (
        String,
        String,
        Option<u64>,
        crate::provider_account::ProviderAccountId,
    ),
}

impl Scope {
    fn accepts(&self, meta: &SessionMeta) -> bool {
        self.session == meta.id
            && Placement::from_meta(meta).ok().as_ref() == Some(&self.stored.placement)
            && self.provider
                == (
                    meta.provider_version.clone(),
                    meta.provider_generation_digest.clone(),
                    meta.provider_auth_generation,
                    meta.provider_account_id.clone(),
                )
    }

    fn capture(state: &Context<'_>, session: &str, epoch: &str) -> Result<(Self, SessionMeta)> {
        let meta = state
            .hub
            .session_info(session)
            .context("MCP session unavailable")?
            .meta;
        let stored = Stored::load(state.hub, session).context("MCP policy unavailable")?;
        ensure!(
            stored.epoch == epoch && !stored.bindings.is_empty(),
            "MCP policy changed"
        );
        let scope = Scope {
            session: session.into(),
            runtime: state
                .control
                .operation_connection(&meta.machine_id)
                .map_err(anyhow::Error::msg)?,
            execution: state
                .control
                .operation_connection(&stored.placement.execution_machine)
                .map_err(anyhow::Error::msg)?,
            provider: (
                meta.provider_version.clone(),
                meta.provider_generation_digest.clone(),
                meta.provider_auth_generation,
                meta.provider_account_id.clone(),
            ),
            stored,
        };
        scope.current(state)?;
        Ok((scope, meta))
    }

    fn current(&self, state: &Context<'_>) -> Result<SessionMeta> {
        ensure!(!*state.shutdown.borrow(), "Controller is stopping");
        ensure!(
            state.control.is_current(&self.runtime) && state.control.is_current(&self.execution),
            "MCP connection changed"
        );
        let meta = state
            .hub
            .session_info(&self.session)
            .context("MCP session unavailable")?
            .meta;
        meta.require_runtime_launch().map_err(anyhow::Error::msg)?;
        ensure!(
            !state.hub.is_managed_child(&self.session),
            "managed MCP launch denied"
        );
        ensure!(
            Stored::load(state.hub, &self.session).as_ref() == Some(&self.stored)
                && self.accepts(&meta),
            "MCP launch policy or placement changed"
        );
        Ok(meta)
    }
}

struct Entry {
    scope: Scope,
    owner: NativeMcpSessionPreparation,
    worker: Mutex<Option<crate::managed_calls::authority::Authority>>,
}

impl Entry {
    fn current(&self, state: &Context<'_>) -> Result<()> {
        let meta = self.scope.current(state)?;
        let binding = meta
            .execution_binding
            .as_ref()
            .context("MCP execution binding missing")?
            .decode()
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.owner.is_current(state.control, &binding),
            "MCP execution identity changed"
        );
        let runtime = state
            .runtimes
            .runtime(&meta.machine_id)
            .context("MCP runtime unavailable")?;
        let current = runtime.mcp_launch_authority(state.service, &meta, self.owner.plan());
        let mut worker = self.worker.lock();
        if let Some(original) = worker.as_ref() {
            ensure!(
                current
                    .as_ref()
                    .is_ok_and(|current| original.accepts(current)),
                "MCP worker changed"
            );
        } else if let Ok(current) = current {
            *worker = Some(current);
        }
        Ok(())
    }
}

impl Launches {
    pub(super) async fn prepare(&self, state: &AppState, session: &str, epoch: &str) -> Result<()> {
        self.prepare_context(&Context::from(state), session, epoch)
            .await
    }

    fn ready_authority(
        &self,
        state: &Context<'_>,
        meta: &SessionMeta,
    ) -> Result<crate::managed_calls::authority::Authority> {
        let plan = self.resolve(state, meta)?;
        let runtime = state
            .runtimes
            .runtime(&meta.machine_id)
            .context("MCP runtime unavailable")?;
        let authority = runtime.mcp_launch_authority(state.service, meta, &plan)?;
        // Recheck the owner after reading the worker observation.
        ensure!(self.resolve(state, meta)? == plan, "MCP launch changed");
        Ok(authority)
    }

    pub(in crate::server) fn grant_epoch(
        &self,
        state: &AppState,
        meta: &SessionMeta,
        authority: &crate::managed_calls::authority::Authority,
    ) -> Option<String> {
        self.grant_epoch_context(&Context::from(state), meta, authority)
    }

    fn grant_epoch_context(
        &self,
        context: &Context<'_>,
        meta: &SessionMeta,
        authority: &crate::managed_calls::authority::Authority,
    ) -> Option<String> {
        let stored = Stored::load(context.hub, &meta.id)?;
        let ready = self.ready_authority(context, meta).ok()?;
        if !authority.accepts(&ready)
            || Stored::load(context.hub, &meta.id).as_ref() != Some(&stored)
        {
            return None;
        }
        Some(stored.epoch)
    }

    pub(super) fn observe(
        &self,
        state: &AppState,
        meta: &SessionMeta,
        value: &mut serde_json::Value,
    ) {
        value["runtime_ready"] = serde_json::Value::Bool(
            value["policy_current"] == true
                && super::super::managed_calls::mcp_ready(state, &meta.id),
        );
        value["worker_ready"] = serde_json::Value::Bool(
            value["policy_current"] == true
                && self.ready_authority(&Context::from(state), meta).is_ok(),
        );
    }

    fn resolve(
        &self,
        state: &Context<'_>,
        meta: &SessionMeta,
    ) -> Result<Box<crate::mcp_binding::NativeLaunchPlan>> {
        let entry = self
            .entries
            .lock()
            .get(&meta.id)
            .cloned()
            .context("MCP launch preparation is not ready for this session")?;
        entry.current(state)?;
        ensure!(entry.scope.accepts(meta), "MCP launch declaration changed");
        Ok(Box::new(entry.owner.plan().clone()))
    }

    async fn prepare_context(&self, state: &Context<'_>, session: &str, epoch: &str) -> Result<()> {
        let _preparation = self
            .preparation
            .try_lock()
            .context("MCP preparation busy")?;
        let (scope, meta) = Scope::capture(state, session, epoch)?;
        if self.resolve(state, &meta).is_ok() {
            return Ok(());
        }
        {
            let entries = self.entries.lock();
            ensure!(
                entries.len() < 64 || entries.contains_key(session),
                "MCP launch capacity reached"
            );
        }
        let binding = meta
            .execution_binding
            .as_ref()
            .context("MCP requires bound execution")?
            .decode()
            .map_err(anyhow::Error::msg)?;
        let admissions = scope
            .stored
            .bindings
            .iter()
            .map(|binding| {
                state
                    .policies
                    .admit(state.hub, session, &meta.machine_id, epoch, &binding.name)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(anyhow::Error::msg)?;
        let deadline = admissions
            .iter()
            .map(super::authority::Admission::deadline)
            .min()
            .context("MCP bindings missing")?;
        let profiles = admissions
            .iter()
            .map(|admission| admission.profile(state.service))
            .collect();
        let owner = {
            let preparation =
                state
                    .control
                    .prepare_session_native_mcp(&scope.execution, binding, profiles);
            tokio::pin!(preparation);
            let mut changes = state.policies.changes.subscribe();
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                scope.current(state)?;
                for admission in &admissions {
                    admission
                        .current(state.policies, state.hub)
                        .map_err(anyhow::Error::msg)?;
                }
                tokio::select! {
                    result = &mut preparation => break result?,
                    _ = tokio::time::sleep_until(deadline) => anyhow::bail!("MCP launch admission expired"),
                    _ = tick.tick() => {},
                    result = changes.changed() => { result.context("MCP policy owner stopped")?; },
                }
            }
        };
        scope.current(state)?;
        for admission in &admissions {
            admission
                .current(state.policies, state.hub)
                .map_err(anyhow::Error::msg)?;
        }
        let mut entries = self.entries.lock();
        scope.current(state)?;
        ensure!(
            entries.len() < 64 || entries.contains_key(session),
            "MCP launch capacity reached"
        );
        entries.insert(
            session.into(),
            Arc::new(Entry {
                scope,
                owner,
                worker: Mutex::new(None),
            }),
        );
        Ok(())
    }

    fn sweep(&self, state: &Context<'_>) {
        self.entries
            .lock()
            .retain(|_, entry| entry.current(state).is_ok());
    }
}

mod recovery;

pub(in crate::server) fn start(state: &Arc<AppState>) {
    recovery::start(state);
    let weak = Arc::downgrade(state);
    state.hub.set_mcp_launch_resolver(Arc::new(move |meta| {
        let state = weak.upgrade().context("Controller is stopping")?;
        state
            .extensions
            .launches
            .resolve(&Context::from(state.as_ref()), meta)
    }));
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            state
                .extensions
                .launches
                .sweep(&Context::from(state.as_ref()));
            if *state.shutdown.borrow() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests;
