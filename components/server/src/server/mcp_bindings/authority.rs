//! Process-local policy observation. It supplements, never replaces, the
//! authenticated parent grant and enrolled Machine connection of the relay.
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::time::Instant;

use super::{Binding, Hub, Placement, Policies, Stored};

const ADMISSION_LIFETIME: Duration = Duration::from_secs(30);

pub(super) struct Admission {
    owner: Weak<()>,
    session: String,
    runtime_machine: String,
    stored: Stored,
    binding: Binding,
    deadline: Instant,
    pub(super) changes: tokio::sync::watch::Receiver<()>,
}

impl Policies {
    pub(super) fn admit(
        &self,
        hub: &Hub,
        session: &str,
        runtime_machine: &str,
        epoch: &str,
        name: &str,
    ) -> Result<Admission, &'static str> {
        // Subscribe before reading policy so a concurrent commit cannot be lost.
        let changes = self.changes.subscribe();
        let stored = current_policy(hub, session, runtime_machine)?;
        if stored.epoch != epoch {
            return Err("epoch_changed");
        }
        let binding = stored
            .bindings
            .iter()
            .find(|binding| binding.name == name)
            .ok_or("binding_not_admitted")?
            .clone();
        Ok(Admission {
            owner: Arc::downgrade(&self.owner),
            changes,
            session: session.into(),
            runtime_machine: runtime_machine.into(),
            stored,
            binding,
            deadline: Instant::now() + ADMISSION_LIFETIME,
        })
    }
}

impl Admission {
    pub(super) fn renewal(&self, policies: &Policies, hub: &Hub) -> Result<Self, &'static str> {
        self.current(policies, hub)?;
        policies.admit(
            hub,
            &self.session,
            &self.runtime_machine,
            &self.stored.epoch,
            &self.binding.name,
        )
    }

    pub(super) fn commit_renewal(
        &mut self,
        candidate: Self,
        policies: &Policies,
        hub: &Hub,
    ) -> Result<(), &'static str> {
        self.current(policies, hub)?;
        candidate.current(policies, hub)?;
        if self.stored != candidate.stored
            || self.binding != candidate.binding
            || self.session != candidate.session
            || self.runtime_machine != candidate.runtime_machine
        {
            return Err("binding_changed");
        }
        *self = candidate;
        Ok(())
    }

    pub(super) fn matches(&self, binding: &str, epoch: &str) -> bool {
        self.binding.name == binding && self.stored.epoch == epoch
    }
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(super) fn profile(&self, service: &str) -> crate::mcp_binding::ProcessProfile {
        crate::mcp_binding::ProcessProfile {
            schema: 1,
            service_id: service.into(),
            session_id: self.session.clone(),
            runtime_machine: self.runtime_machine.clone(),
            execution_machine: self.stored.placement.execution_machine.clone(),
            workspace_id: self.stored.placement.workspace.clone(),
            epoch: self.stored.epoch.clone(),
            binding: self.binding.clone(),
        }
    }

    pub(super) fn current(&self, policies: &Policies, hub: &Hub) -> Result<(), &'static str> {
        if !Weak::ptr_eq(&self.owner, &Arc::downgrade(&policies.owner))
            || self.owner.upgrade().is_none()
            || self.changes.has_changed().is_err()
            || Instant::now() >= self.deadline
        {
            return Err("admission_ended");
        }
        let current = current_policy(hub, &self.session, &self.runtime_machine)?;
        if current != self.stored || !current.bindings.contains(&self.binding) {
            return Err("binding_changed");
        }
        Ok(())
    }
}

fn current_policy(hub: &Hub, session: &str, runtime_machine: &str) -> Result<Stored, &'static str> {
    let meta = hub.session_info(session).ok_or("session_unavailable")?.meta;
    if hub.is_managed_child(session) {
        return Err("managed_session_denied");
    }
    if meta.machine_id != runtime_machine {
        return Err("wrong_runtime_machine");
    }
    let placement = Placement::from_meta(&meta)?;
    let stored = Stored::load(hub, session).ok_or("binding_not_configured")?;
    if stored.placement != placement {
        return Err("placement_changed");
    }
    Ok(stored)
}
