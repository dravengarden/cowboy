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
        let (phase, expired_attachment, grace_expired, scan_due, snapshot_due) = {
            let live = instance.live.lock();
            (
                live.phase,
                live.attachment
                    .as_ref()
                    .filter(|attachment| attachment.lease_expires <= now)
                    .map(|attachment| attachment.id.clone()),
                live.detached_since
                    .filter(|since| *since + instance.settings.hibernate_after <= now),
                live.last_scan + SOCKET_SCAN_INTERVAL <= now,
                live.phase == Phase::Attached
                    && live.last_periodic + instance.settings.snapshot_interval <= now,
            )
        };
        if phase == Phase::Stopping {
            return true;
        }
        if now >= instance.deadline() {
            let event = self
                .stop_instance(instance, Reason::Deadline, false, None)
                .await;
            self.send_state(event);
            return true;
        }
        if let Some(reason) = self.login_binding_broken(instance) {
            // A login that ended keeps what it signed in; a binding that no
            // longer matches is an authority failure and stops at once.
            let graceful = reason == Reason::LoginEnded;
            let event = self.stop_instance(instance, reason, graceful, None).await;
            self.send_state(event);
            return true;
        }
        if let Some(attachment_id) = expired_attachment
            && expire_attachment(instance, &attachment_id, now)
        {
            self.send_state(instance.state(None));
        }
        if let Some(since) = grace_expired
            && instance.live.lock().phase == Phase::Detached
        {
            // The graceful stop waits for a running copy and the lifecycle
            // lock; a viewer attaching meanwhile renews the instance, so the
            // detached interval that expired is re-checked atomically with
            // the transition to `Stopping`. The deadline and an ended login
            // are not idleness and stop unconditionally above.
            let stopped = self
                .stop_instance_when(
                    instance,
                    Reason::Grace,
                    true,
                    None,
                    || Ok(()),
                    |live| {
                        if still_idle(live, instance.settings.hibernate_after, since) {
                            Ok(())
                        } else {
                            Err(IdleRenewed)
                        }
                    },
                )
                .await;
            match stopped {
                Ok(event) => {
                    self.send_state(event);
                    return true;
                }
                Err(IdleRenewed) => {
                    tracing::debug!(instance = %instance.instance_id, "browser grace stop abandoned: the instance was attached meanwhile");
                }
            }
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
        if snapshot_due && !instance.copy_running() {
            // The copy runs as its own job; a failure waits a full interval.
            instance.live.lock().last_periodic = now;
            self.spawn_snapshot(instance);
        }
        let pending = instance.live.lock().pending_navigation.take();
        if let Some(url) = pending
            && let Err(Navigation::Violation(reason)) = self.navigate_checked(instance, &url).await
        {
            let event = self.stop_instance(instance, reason, false, None).await;
            self.send_state(event);
            return true;
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

/// An idle stop abandoned because the instance was used while it waited.
#[derive(Debug)]
pub(super) struct IdleRenewed;

/// Whether the instance is still in the detached interval that began at
/// `since` and that interval still exceeds the hibernation grace: an attach
/// (and any later detach) replaces `detached_since`, and `Stopping` or
/// `Attached` are not idle. Evaluated under the lifecycle lock and the live
/// lock the grace stop changes the phase under.
pub(super) fn still_idle(live: &Live, hibernate_after: Duration, since: Instant) -> bool {
    live.phase == Phase::Detached
        && live.attachment.is_none()
        && live.detached_since == Some(since)
        && since + hibernate_after <= Instant::now()
}

/// Detach the attachment whose lease was seen expired, re-checked under the
/// live lock: only an attached instance whose current attachment is still
/// that one and still expired is detached. A stop that began meanwhile (and
/// took the attachment), a renewal or a takeover leaves everything as it is,
/// and `Stopping` never goes back to `Detached`. `true` when it detached.
pub(super) fn expire_attachment(instance: &Instance, attachment_id: &str, now: Instant) -> bool {
    let mut live = instance.live.lock();
    let still_expired = live.phase == Phase::Attached
        && live.attachment.as_ref().is_some_and(|attachment| {
            attachment.id == attachment_id && attachment.lease_expires <= now
        });
    if !still_expired {
        return false;
    }
    if let Some(attachment) = live.attachment.take()
        && let Some(stream) = attachment.stream
    {
        stream.cancel();
    }
    live.phase = Phase::Detached;
    live.detached_since = Some(now);
    true
}
