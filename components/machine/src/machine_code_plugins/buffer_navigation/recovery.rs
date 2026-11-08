use super::*;
use crate::machine_code_plugins::observation;

fn pending(slot: &Slot) -> bool {
    matches!(slot.phase, Phase::Unknown | Phase::ReleaseUnknown)
}

impl Operations {
    pub(in crate::machine_code_plugins) async fn observe_pending(&self) {
        self.expire_inert();
        let entries = observation::candidates(
            &self.registry.lock().active,
            &mut self.observation_cursor.lock(),
            pending,
        );
        for (id, entry) in entries {
            let observation = self.observe_original(&id, &entry);
            let _ = tokio::time::timeout(observation::DEADLINE, observation).await;
        }
    }

    async fn observe_original(&self, id: &NavigationRef, entry: &Arc<Mutex<Slot>>) -> Result<()> {
        let Ok(mut slot) = entry.try_lock() else {
            return Ok(());
        };
        if !pending(&slot) {
            return Ok(());
        }
        let target = slot.target.as_ref().context("navigation target lost")?;
        let runtime = Arc::clone(&target.runtime);
        let route = Arc::clone(&target.route);
        let Ok(mut route) = route.try_lock() else {
            return Ok(());
        };
        ensure!(runtime.is_running()?, "original navigation runtime exited");
        let value = exchange(
            &runtime.socket,
            &json!({"type":"bufferNavigation","navigation":slot.native,"action":"query"}),
        )
        .await?;
        self.accept_observation(id, entry, &mut slot, &mut route, &value)
    }
}
