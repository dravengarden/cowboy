//! A synthetic systemd: a unit inventory the test edits, observations that
//! can fail or wait on a gate, and terminations bound to invocations exactly
//! as `unit::terminate` binds them.

use super::*;
use crate::machine_browser::manager::units::UnitFuture;

/// One synthetic unit.
#[derive(Clone)]
pub(super) struct FakeUnit {
    invocation: String,
    live: bool,
    /// Whether a termination confirms its exit.
    stoppable: bool,
    /// Whether `show` answers for it.
    observable: bool,
}

impl FakeUnit {
    pub(super) fn live(invocation: &str) -> Self {
        Self {
            invocation: invocation.to_owned(),
            live: true,
            stoppable: true,
            observable: true,
        }
    }

    pub(super) fn unstoppable(mut self) -> Self {
        self.stoppable = false;
        self
    }

    pub(super) fn unobservable(mut self) -> Self {
        self.observable = false;
        self
    }
}

/// Holds one unit's `show` until released, announcing that it was reached.
pub(super) struct Gate {
    pub(super) reached: tokio::sync::Notify,
    pub(super) release: tokio::sync::Notify,
}

#[derive(Default)]
struct State {
    /// `None` makes the listing fail.
    listing: Option<Vec<String>>,
    units: HashMap<String, FakeUnit>,
    gates: HashMap<String, Arc<Gate>>,
    terminations: Vec<(String, Option<String>)>,
}

#[derive(Default)]
pub(super) struct FakeUnits {
    state: parking_lot::Mutex<State>,
}

impl FakeUnits {
    pub(super) fn set(&self, unit: &str, fake: FakeUnit) {
        self.state.lock().units.insert(unit.to_owned(), fake);
    }

    /// Make `unit`'s exit confirmable from now on.
    pub(super) fn make_stoppable(&self, unit: &str) {
        if let Some(fake) = self.state.lock().units.get_mut(unit) {
            fake.stoppable = true;
        }
    }

    pub(super) fn list(&self, units: Option<&[&str]>) {
        self.state.lock().listing =
            units.map(|units| units.iter().map(|unit| (*unit).to_owned()).collect());
    }

    pub(super) fn gate(&self, unit: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate {
            reached: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        self.state
            .lock()
            .gates
            .insert(unit.to_owned(), Arc::clone(&gate));
        gate
    }

    /// Every termination asked for, with the invocation it was bound to.
    pub(super) fn terminations(&self) -> Vec<(String, Option<String>)> {
        self.state.lock().terminations.clone()
    }

    pub(super) fn exists(&self, unit: &str) -> bool {
        self.state.lock().units.contains_key(unit)
    }
}

impl UnitControl for FakeUnits {
    fn list(&self) -> UnitFuture<'_, Result<Vec<String>>> {
        let listing = self.state.lock().listing.clone();
        Box::pin(async move { listing.context("synthetic unit listing failed") })
    }

    fn show<'a>(&'a self, unit: &'a str) -> UnitFuture<'a, Result<unit::UnitStatus>> {
        Box::pin(async move {
            let gate = self.state.lock().gates.remove(unit);
            if let Some(gate) = gate {
                gate.reached.notify_one();
                gate.release.notified().await;
            }
            let fake = self.state.lock().units.get(unit).cloned();
            match fake {
                None => Ok(unit::UnitStatus {
                    load_state: "not-found".into(),
                    ..unit::UnitStatus::default()
                }),
                Some(fake) if !fake.observable => bail!("synthetic user bus failure"),
                Some(fake) => Ok(unit::UnitStatus {
                    load_state: "loaded".into(),
                    active_state: if fake.live { "active" } else { "failed" }.into(),
                    invocation_id: fake.invocation,
                    ..unit::UnitStatus::default()
                }),
            }
        })
    }

    fn terminate<'a>(
        &'a self,
        unit: &'a str,
        _cgroup_dir: Option<&'a Path>,
        invocation_id: Option<&'a str>,
    ) -> UnitFuture<'a, bool> {
        Box::pin(async move {
            let mut state = self.state.lock();
            state
                .terminations
                .push((unit.to_owned(), invocation_id.map(ToOwned::to_owned)));
            let Some(fake) = state.units.get(unit).cloned() else {
                return true;
            };
            if !fake.observable {
                return false;
            }
            if invocation_id.is_some_and(|bound| bound != fake.invocation) {
                // Another invocation runs under the name: the bound one ended.
                return true;
            }
            if fake.stoppable {
                state.units.remove(unit);
            }
            fake.stoppable
        })
    }
}
