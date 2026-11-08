//! Per-instance supervision: deadlines, grace, leases, periodic verification
//! and launcher events.

use super::*;

impl BrowserManager {
    pub(super) fn spawn_supervisor(self: &Arc<Self>, instance: &Arc<Instance>) {
        let manager = Arc::downgrade(self);
        let instance = Arc::clone(instance);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SUPERVISOR_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                if manager.supervise(&instance).await {
                    return;
                }
            }
        });
    }

    /// One supervision tick; `true` once the instance is gone.
    pub(super) async fn supervise(self: &Arc<Self>, instance: &Arc<Instance>) -> bool {
        let now = Instant::now();
        let (phase, lease_expired, grace_expired, scan_due, snapshot_due) = {
            let live = instance.live.lock();
            (
                live.phase,
                live.attachment
                    .as_ref()
                    .is_some_and(|attachment| attachment.lease_expires <= now),
                live.detached_since
                    .is_some_and(|since| since + instance.settings.hibernate_after <= now),
                live.last_scan + SOCKET_SCAN_INTERVAL <= now,
                live.phase == Phase::Attached
                    && live.last_periodic + instance.settings.snapshot_interval <= now,
            )
        };
        if phase == Phase::Stopping {
            return true;
        }
        if now >= instance.deadline {
            let event = self
                .stop_instance(instance, Reason::Deadline, false, None)
                .await;
            self.send_state(event);
            return true;
        }
        if let Some(reason) = self.login_binding_broken(instance) {
            let event = self.stop_instance(instance, reason, true, None).await;
            self.send_state(event);
            return true;
        }
        if lease_expired {
            let mut live = instance.live.lock();
            if let Some(attachment) = live.attachment.take()
                && let Some(stream) = attachment.stream
            {
                stream.cancel();
            }
            live.phase = Phase::Detached;
            live.detached_since = Some(now);
            drop(live);
            self.send_state(instance.state(None));
        }
        if grace_expired && instance.live.lock().phase == Phase::Detached {
            let event = self
                .stop_instance(instance, Reason::Grace, true, None)
                .await;
            self.send_state(event);
            return true;
        }
        if scan_due {
            instance.live.lock().last_scan = now;
            if let Err(reason) = self.recheck(instance) {
                let event = self.stop_instance(instance, reason, false, None).await;
                self.send_state(event);
                return true;
            }
        }
        if let Ok(mut control) = instance.control.try_lock() {
            match control.next_event(Duration::from_millis(20)).await {
                Ok(Some(LauncherEvent::Exited { process, code })) => {
                    drop(control);
                    tracing::warn!(instance = %instance.instance_id, %process, ?code, "browser process exited");
                    let event = self
                        .stop_instance(instance, Reason::Failed, false, None)
                        .await;
                    self.send_state(event);
                    return true;
                }
                Ok(Some(LauncherEvent::Error { kind, message })) => {
                    tracing::warn!(instance = %instance.instance_id, ?kind, %message, "launcher reported an error");
                }
                Ok(_) => {}
                Err(error) => {
                    drop(control);
                    tracing::warn!(%error, instance = %instance.instance_id, "launcher control socket failed");
                    let event = self
                        .stop_instance(instance, Reason::Failed, false, None)
                        .await;
                    self.send_state(event);
                    return true;
                }
            }
        }
        if snapshot_due {
            let slot = self.slot(&instance.account_id);
            let _lifecycle = slot.lifecycle.lock().await;
            if let Err(error) = self.frozen_snapshot(instance).await {
                tracing::warn!(%error, instance = %instance.instance_id, "periodic browser snapshot failed");
                instance.live.lock().last_periodic = now;
            }
        }
        let pending = instance.live.lock().pending_navigation.take();
        if let Some(url) = pending {
            self.navigate_verified(instance, &url).await;
        }
        false
    }

    /// Cheap periodic evidence: pins, socket scan and runtime directory.
    pub(super) fn recheck(&self, instance: &Arc<Instance>) -> Result<(), Reason> {
        if !instance.live.lock().verified.pins_hold() {
            return Err(Reason::ConfinementUnverified);
        }
        if let Err(violation) = verify::scan_sockets(&instance.cgroup_dir) {
            tracing::warn!(detail = %violation, "browser socket scan violation");
            return Err(Reason::ListenerDetected);
        }
        if let Err(violation) = verify::runtime_directory_clean(&instance.runtime_dir) {
            tracing::warn!(detail = %violation, "browser runtime directory violation");
            return Err(Reason::ConfinementUnverified);
        }
        Ok(())
    }
}
