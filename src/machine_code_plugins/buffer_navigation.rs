//! Connection-issued original-generation acquisition. No path lookup or
//! installation selection occurs here; ambiguous effects are never replayed.
use super::{buffer_leases, exchange};
use crate::machine_plugins::{CodeNavigationInvocation, CodeNavigationOwner};
use crate::machine_protocol::code_buffer_navigation::{
    Action, BufferRef, Destination, Location, NavigationRef, Phase, Snapshot,
};
use anyhow::{Context as _, Result, ensure};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

mod codec;
mod registry;
use codec::{NativeRef, NativeState, reply};
use registry::Registry;

const MAX_NAVIGATIONS: usize = 32;
const MAX_COMMANDS: usize = 64;
const PREPARE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Execute,
    Query,
    Release,
    Destination,
}

impl Step {
    fn native(self) -> Result<&'static str> {
        match self {
            Self::Execute => Ok("execute"),
            Self::Query => Ok("query"),
            Self::Release => Ok("release"),
            Self::Destination => {
                anyhow::bail!("destination preparation is not a navigation effect")
            }
        }
    }
}

struct Slot {
    owner: CodeNavigationOwner,
    target: Option<buffer_leases::RetainedTarget>,
    source: BufferRef,
    native: NativeRef,
    phase: Phase,
    locations: Vec<Location>,
    destinations: BTreeMap<u32, Destination>,
    until: Option<Instant>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Slot {
    fn snapshot(&self, id: &NavigationRef) -> Snapshot {
        Snapshot {
            api_version: 1,
            navigation: id.clone(),
            phase: self.phase,
            locations: self.locations.clone(),
            destinations: self.destinations.values().cloned().collect(),
        }
    }
}

pub(super) struct Operations {
    registry: parking_lot::Mutex<Registry>,
    capacity: Arc<Semaphore>,
    commands: Semaphore,
}

impl Default for Operations {
    fn default() -> Self {
        Self {
            registry: parking_lot::Mutex::default(),
            capacity: Arc::new(Semaphore::new(MAX_NAVIGATIONS)),
            commands: Semaphore::new(MAX_COMMANDS),
        }
    }
}

impl Operations {
    pub(super) fn expire_inert(&self) {
        self.registry.lock().expire();
    }

    pub(super) async fn execute(
        &self,
        invocation: CodeNavigationInvocation,
        leases: &buffer_leases::Routes,
    ) -> Result<Snapshot> {
        let _command = self
            .commands
            .try_acquire()
            .context("navigation command capacity reached")?;
        self.expire_inert();
        tokio::time::timeout(
            invocation.remaining()?,
            self.execute_inner(&invocation, leases),
        )
        .await
        .context("navigation command deadline ended")?
    }

    async fn prepare(
        &self,
        invocation: &CodeNavigationInvocation,
        leases: &buffer_leases::Routes,
    ) -> Result<Snapshot> {
        let Action::Prepare {
            lease,
            content,
            position,
            query,
        } = &invocation.request().action
        else {
            unreachable!()
        };
        let (id, permit) = {
            let mut registry = self.registry.lock();
            let permit = Arc::clone(&self.capacity)
                .try_acquire_owned()
                .context("navigation capacity reached")?;
            (registry.allocate()?, permit)
        };
        let until = Instant::now() + PREPARE_TTL;
        let target = leases.retained_target(lease).await?;
        let route = Arc::clone(&target.route);
        let mut route = route.lock().await;
        invocation.remaining()?;
        leases.check_navigation_source(lease, &target).await?;
        ensure!(
            target.runtime.is_running()?,
            "original navigation runtime exited"
        );
        let supported = exchange(
            &target.runtime.socket,
            &json!({"type":"bufferNavigationSupport"}),
        )
        .await?;
        ensure!(
            supported == json!({"type":"bufferNavigationSupport","api_version":1,"protocol":1}),
            "owned navigation unavailable"
        );
        invocation.remaining()?;
        let value = exchange(
            &target.runtime.socket,
            &json!({"type":"prepareBufferNavigation",
            "lease":lease,"content":content,"position":position,"kind":query}),
        )
        .await?;
        let observed = reply(&value)?;
        ensure!(
            matches!(observed.state, NativeState::Prepared {}),
            "navigation was not prepared"
        );
        let owner = invocation.owner()?;
        ensure!(until > Instant::now(), "navigation preparation expired");
        let mut registry = self.registry.lock();
        ensure!(
            !registry.native.contains_key(&observed.navigation),
            "native navigation reference reused"
        );
        let slot = Slot {
            owner,
            target: Some(target),
            source: lease.clone(),
            native: observed.navigation.clone(),
            phase: Phase::Prepared,
            locations: Vec::new(),
            destinations: BTreeMap::new(),
            until: Some(until),
            permit: Some(permit),
        };
        let snapshot = slot.snapshot(&id);
        registry.native.insert(observed.navigation, id.clone());
        registry
            .active
            .insert(id.clone(), Arc::new(Mutex::new(slot)));
        route.owned_navigations.insert(id);
        Ok(snapshot)
    }

    async fn execute_inner(
        &self,
        invocation: &CodeNavigationInvocation,
        leases: &buffer_leases::Routes,
    ) -> Result<Snapshot> {
        let (id, action) = match &invocation.request().action {
            Action::Prepare { .. } => return self.prepare(invocation, leases).await,
            Action::Execute { navigation } => (navigation, Step::Execute),
            Action::Query { navigation } => (navigation, Step::Query),
            Action::Release { navigation } => (navigation, Step::Release),
            Action::PrepareDestination { navigation, .. } => (navigation, Step::Destination),
        };
        let entry = self.registry.lock().find(id)?;
        let mut slot = entry.lock().await;
        invocation.check_owner(&slot.owner)?;
        if slot.phase == Phase::Released
            || action == Step::Execute && slot.phase != Phase::Prepared
            || action == Step::Query && matches!(slot.phase, Phase::Prepared | Phase::Retained)
            || action == Step::Release
                && matches!(slot.phase, Phase::Unknown | Phase::ReleaseUnknown)
        {
            return Ok(slot.snapshot(id));
        }
        if let Action::PrepareDestination {
            destination,
            content,
            ..
        } = &invocation.request().action
        {
            ensure!(
                slot.phase == Phase::Retained,
                "navigation destination is not retained"
            );
            let location = slot
                .locations
                .get(*destination as usize)
                .context("unknown navigation destination")?;
            ensure!(
                &location.content == content,
                "navigation destination content changed"
            );
            if slot.destinations.contains_key(destination) {
                return Ok(slot.snapshot(id));
            }
        }
        // Reserve BEFORE taking the route: ordinary lease expiry takes routes
        // itself. Capacity includes in-flight, cancelled preparations.
        let reservation = if action == Step::Destination {
            Some(leases.reserve().await?)
        } else {
            None
        };
        let target = slot.target.as_ref().context("navigation target lost")?;
        let runtime = Arc::clone(&target.runtime);
        let route = Arc::clone(&target.route);
        let mut route = route.lock().await;
        invocation.check_owner(&slot.owner)?;
        if slot.phase == Phase::Prepared
            && (action == Step::Release || slot.until.is_some_and(|until| until <= Instant::now()))
        {
            self.registry
                .lock()
                .retire(id, &entry, &mut slot, &mut route);
            return Ok(slot.snapshot(id));
        }
        ensure!(runtime.is_running()?, "original navigation runtime exited");
        if let Action::PrepareDestination {
            destination,
            content,
            ..
        } = &invocation.request().action
        {
            let payload = json!({"type":"prepareNavigationBuffer","navigation":slot.native,
                "destination":destination,"content":content});
            let lease = leases
                .prepare_navigation(
                    reservation.expect("destination reservation"),
                    slot.target.as_ref().expect("retained target"),
                    &mut route,
                    &payload,
                    invocation,
                )
                .await?;
            // No await between recording the ordinary route and this original
            // group's lookup. A lost observer discovers the same reservation.
            slot.destinations.insert(
                *destination,
                Destination {
                    destination: *destination,
                    lease,
                },
            );
            return Ok(slot.snapshot(id));
        }
        if action == Step::Execute {
            leases
                .check_navigation_source(
                    &slot.source,
                    slot.target.as_ref().expect("prepared target"),
                )
                .await?;
            invocation.check_owner(&slot.owner)?;
            slot.phase = Phase::Unknown;
            slot.until = None;
        } else if action == Step::Release {
            slot.phase = Phase::ReleaseUnknown;
        }
        let value = exchange(
            &runtime.socket,
            &json!({"type":"bufferNavigation",
            "navigation":slot.native,"action":action.native()?}),
        )
        .await?;
        let observed = reply(&value)?;
        ensure!(
            observed.navigation == slot.native,
            "navigation reply owner changed"
        );
        match (slot.phase, observed.state) {
            (Phase::Unknown, NativeState::Unknown {}) => {}
            (Phase::Unknown, NativeState::Retained { locations }) => {
                slot.phase = Phase::Retained;
                slot.locations = locations;
            }
            (Phase::ReleaseUnknown, NativeState::ReleaseUnknown {}) => {}
            (Phase::ReleaseUnknown, NativeState::Released {}) => {
                self.registry
                    .lock()
                    .retire(id, &entry, &mut slot, &mut route);
            }
            _ => anyhow::bail!("navigation outcome unavailable"),
        }
        Ok(slot.snapshot(id))
    }
}

#[cfg(test)]
mod tests;
