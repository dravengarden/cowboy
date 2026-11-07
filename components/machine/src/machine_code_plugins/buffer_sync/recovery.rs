use super::*;
use crate::machine_code_plugins::observation;

fn pending(slot: &Slot) -> bool {
    matches!(slot.state, State::Pending {} | State::Unknown {})
        || slot.retire_attempted && slot.state != (State::Retired {})
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
            // Cancellation discards only a query, never an effect or its fence.
            let _ = tokio::time::timeout(observation::DEADLINE, observation).await;
        }
    }

    async fn observe_original(&self, id: &OperationRef, entry: &Arc<Mutex<Slot>>) -> Result<()> {
        let Ok(mut slot) = entry.try_lock() else {
            return Ok(());
        };
        if !pending(&slot) {
            return Ok(());
        }
        let target = slot
            .target
            .as_ref()
            .context("synchronization target lost")?;
        let runtime = Arc::clone(&target.runtime);
        let route = Arc::clone(&target.route);
        let Ok(_route) = route.try_lock() else {
            return Ok(());
        };
        ensure!(
            runtime.is_running()?,
            "original synchronization runtime exited"
        );
        let value = exchange(
            &runtime.socket,
            &json!({"type":"bufferSync","operation":slot.native,"action":"query"}),
        )
        .await?;
        self.accept_observation(id, entry, &mut slot, NativeAction::Query, &value)
    }
}
