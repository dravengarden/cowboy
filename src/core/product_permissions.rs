//! Process-local product permission observations, never credentials or grants.
//! The settings owner ends affected lifetimes at mutation, even with no polling.
use super::{Hub, HubInner};
use crate::admin::{AdminRole, PERMISSIONS_SETTING, PermissionPolicy};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

type Settings = HashMap<String, serde_json::Value>;
const MAX_OBSERVATIONS: usize = 1024;
const MAX_ACCOUNT_BYTES: usize = 64;

struct State {
    account: String,
    role: AdminRole,
    current: AtomicBool,
}

/// Clone shares one observation, not a fresh permission lifetime. No serde or
/// Debug; an account/role tuple cannot reconstruct this private core identity.
#[derive(Clone)]
pub(crate) struct ProductPermissionObservation {
    owner: Weak<HubInner>,
    state: Arc<State>,
}

impl ProductPermissionObservation {
    pub(crate) fn role(&self) -> AdminRole {
        self.state.role
    }

    pub(crate) fn current(&self, hub: &Hub, account: &str) -> bool {
        Weak::ptr_eq(&self.owner, &Arc::downgrade(&hub.inner))
            && account.len() <= MAX_ACCOUNT_BYTES
            && account.trim().eq_ignore_ascii_case(&self.state.account)
            && self.state.current.load(Ordering::Acquire)
    }
}

#[derive(Default)]
pub(super) struct Observations {
    current: HashMap<String, Weak<State>>,
    // Ended lifetimes still consume capacity until their actual holders drop.
    // Repeated policy changes cannot mint an unbounded retained history.
    retained: Vec<Weak<State>>,
}

impl Observations {
    fn reconcile(&mut self, settings: &Settings) {
        if self.current.is_empty() {
            return;
        }
        let policy = PermissionPolicy::from_setting(settings.get(PERMISSIONS_SETTING));
        self.current.retain(|account, state| {
            let Some(state) = state.upgrade() else {
                return false;
            };
            if policy.role_for(account) != state.role {
                state.current.store(false, Ordering::Release);
                return false;
            }
            true
        });
    }
}

impl Hub {
    pub(crate) fn observe_product_permissions(
        &self,
        account: &str,
    ) -> Option<ProductPermissionObservation> {
        if account.len() > MAX_ACCOUNT_BYTES || account.trim().is_empty() {
            return None;
        }
        let account = account.trim().to_ascii_lowercase();
        // Same lock order as mutation: settings, then observations. Capture
        // the effective role and its lifetime atomically, without an await.
        let settings = self.inner.settings.lock();
        let mut observations = self.inner.product_permissions.lock();
        let state = if let Some(state) = observations.current.get(&account).and_then(Weak::upgrade)
        {
            state
        } else {
            if observations.retained.len() >= MAX_OBSERVATIONS {
                observations
                    .retained
                    .retain(|state| state.strong_count() > 0);
                observations
                    .current
                    .retain(|_, state| state.strong_count() > 0);
            }
            if observations.retained.len() >= MAX_OBSERVATIONS {
                return None;
            }
            let state = Arc::new(State {
                role: PermissionPolicy::from_setting(settings.get(PERMISSIONS_SETTING))
                    .role_for(&account),
                account: account.clone(),
                current: AtomicBool::new(true),
            });
            observations.current.insert(account, Arc::downgrade(&state));
            observations.retained.push(Arc::downgrade(&state));
            state
        };
        Some(ProductPermissionObservation {
            owner: Arc::downgrade(&self.inner),
            state,
        })
    }
}

/// Reconcile before releasing the settings lock, including when a mutation
/// closure unwinds. No second mutable settings path may bypass this owner.
pub(super) fn mutate<R>(
    hub: &Hub,
    settings: &mut Settings,
    f: impl FnOnce(&mut Settings) -> R,
) -> R {
    struct Mutation<'a> {
        hub: &'a Hub,
        settings: &'a mut Settings,
    }
    impl Drop for Mutation<'_> {
        fn drop(&mut self) {
            self.hub
                .inner
                .product_permissions
                .lock()
                .reconcile(self.settings);
        }
    }
    let mutation = Mutation { hub, settings };
    f(mutation.settings)
}

#[cfg(test)]
mod tests;
