//! Only inert preparations expire. Uncertainty retains the exact runtime and
//! route; terminal tombstones retain bounded metadata, not a process or permit.
use super::super::WorktreeRoute;
use super::*;
use std::collections::VecDeque;

pub(super) struct Registry {
    instance: String,
    last_id: u64,
    pub active: BTreeMap<NavigationRef, Arc<Mutex<Slot>>>,
    retired: VecDeque<(NavigationRef, Arc<Mutex<Slot>>)>,
    pub native: BTreeMap<NativeRef, NavigationRef>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            instance: uuid::Uuid::new_v4().simple().to_string(),
            last_id: 0,
            active: BTreeMap::new(),
            retired: VecDeque::new(),
            native: BTreeMap::new(),
        }
    }
}

impl Registry {
    pub fn allocate(&mut self) -> Result<NavigationRef> {
        self.last_id = self
            .last_id
            .checked_add(1)
            .context("navigation identities exhausted")?;
        Ok(NavigationRef {
            instance: self.instance.clone(),
            id: format!("navigation:{:016x}", self.last_id),
        })
    }

    pub fn find(&self, id: &NavigationRef) -> Result<Arc<Mutex<Slot>>> {
        self.active
            .get(id)
            .or_else(|| {
                self.retired
                    .iter()
                    .find(|(key, _)| key == id)
                    .map(|(_, slot)| slot)
            })
            .cloned()
            .context("navigation is not retained by this Machine")
    }

    pub fn retire(
        &mut self,
        id: &NavigationRef,
        entry: &Arc<Mutex<Slot>>,
        slot: &mut Slot,
        route: &mut WorktreeRoute,
    ) {
        slot.phase = Phase::Released;
        slot.until = None;
        slot.target = None;
        slot.permit = None;
        if route.owned_navigations.remove(id) && route.is_idle() {
            *route = WorktreeRoute::default();
        }
        self.active.remove(id);
        if self.retired.len() == MAX_NAVIGATIONS
            && let Some((old, _)) = self.retired.pop_front()
        {
            self.native.retain(|_, owner| owner != &old);
        }
        self.retired.push_back((id.clone(), Arc::clone(entry)));
    }

    pub fn expire(&mut self) {
        let entries: Vec<_> = self
            .active
            .iter()
            .map(|(id, entry)| (id.clone(), Arc::clone(entry)))
            .collect();
        // No blocking or await under this registry. A busy route/operation
        // remains owned until a later pass, never removed before cleanup.
        for (id, entry) in entries {
            let Ok(mut slot) = entry.try_lock() else {
                continue;
            };
            if slot.phase != Phase::Prepared
                || !slot.until.is_some_and(|until| until <= Instant::now())
            {
                continue;
            }
            let route = Arc::clone(&slot.target.as_ref().expect("prepared target").route);
            if let Ok(mut route) = route.try_lock() {
                self.retire(&id, &entry, &mut slot, &mut route);
            }
        }
    }
}
