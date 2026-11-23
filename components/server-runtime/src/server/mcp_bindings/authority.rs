//! Process-local policy observation. It supplements, never replaces, the
//! authenticated parent grant and enrolled Machine connection of the relay.
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::time::Instant;

use super::{Binding, Hub, Placement, Policies, Stored};

const ADMISSION_LIFETIME: Duration = Duration::from_secs(30);

pub struct Admission {
    owner: Weak<()>,
    session: String,
    runtime_machine: String,
    stored: Stored,
    binding: Binding,
    deadline: Instant,
    pub changes: tokio::sync::watch::Receiver<()>,
    /// A view scope's app projection (docs/widget-apps-mcp.md §3): replaces
    /// the agent's tools in the process profile; `None` for the agent scope.
    view_tools: Option<Vec<String>>,
}

impl Policies {
    pub fn admit(
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
            view_tools: None,
        })
    }
}

impl Admission {
    pub fn renewal(&self, policies: &Policies, hub: &Hub) -> Result<Self, &'static str> {
        self.current(policies, hub)?;
        let mut renewed = policies.admit(
            hub,
            &self.session,
            &self.runtime_machine,
            &self.stored.epoch,
            &self.binding.name,
        )?;
        renewed.view_tools.clone_from(&self.view_tools);
        Ok(renewed)
    }

    /// The session and binding this admission serves.
    pub fn names(&self) -> (&str, &str) {
        (&self.session, &self.binding.name)
    }

    pub fn is_view(&self) -> bool {
        self.view_tools.is_some()
    }

    /// The same admission for a view scope whose process admits only `tools`.
    pub fn for_view(mut self, tools: Vec<String>) -> Self {
        self.view_tools = Some(tools);
        self
    }

    pub fn commit_renewal(
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
            || self.view_tools != candidate.view_tools
        {
            return Err("binding_changed");
        }
        *self = candidate;
        Ok(())
    }

    pub fn matches(&self, binding: &str, epoch: &str) -> bool {
        self.binding.name == binding && self.stored.epoch == epoch
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn profile(&self, service: &str) -> crate::mcp_binding::ProcessProfile {
        crate::mcp_binding::ProcessProfile {
            schema: 1,
            service_id: service.into(),
            session_id: self.session.clone(),
            runtime_machine: self.runtime_machine.clone(),
            execution_machine: self.stored.placement.execution_machine.clone(),
            workspace_id: self.stored.placement.workspace.clone(),
            epoch: self.stored.epoch.clone(),
            binding: match &self.view_tools {
                Some(tools) => crate::mcp_binding::Binding {
                    tools: tools.clone(),
                    ..self.binding.clone()
                },
                None => self.binding.clone(),
            },
        }
    }

    pub fn current(&self, policies: &Policies, hub: &Hub) -> Result<(), &'static str> {
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
