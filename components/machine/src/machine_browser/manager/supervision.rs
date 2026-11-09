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
        let (phase, expired_attachment, grace_expired, snapshot_due) = {
            let live = instance.live.lock();
            (
                live.phase,
                live.attachment
                    .as_ref()
                    .filter(|attachment| attachment.lease_expires <= now)
                    .map(|attachment| attachment.id.clone()),
                live.detached_since
                    .filter(|since| *since + instance.settings.hibernate_after <= now),
                live.phase == Phase::Attached
                    && live.last_periodic + instance.settings.snapshot_interval <= now,
            )
        };
        if phase == Phase::Stopping {
            return true;
        }
        // A latch another path set (a stop that was then abandoned, or a
        // waiter that counted the last denial) is enforced here.
        let latched = *instance.latch.lock();
        if let Some(reason) = latched {
            let event = self.stop_instance(instance, reason, false, None).await;
            self.send_state(event);
            return true;
        }
        if now >= instance.deadline() {
            let event = self
                .stop_instance(instance, Reason::Deadline, false, None)
                .await;
            self.send_state(event);
            return true;
        }
        if let Some(attachment_id) = expired_attachment
            && expire_attachment(instance, &attachment_id, now)
        {
            tracing::info!(event_name = "browser.viewer.detached",
                browser_instance = %instance.instance_id, browser_attachment = %attachment_id,
                browser_cause = "lease_expired", "browser viewer lease expired");
            self.send_state(instance.state(None));
        }
        if let Some(since) = grace_expired
            && instance.live.lock().phase == Phase::Detached
        {
            // The graceful stop waits for a running copy and the lifecycle
            // lock; a viewer attaching meanwhile renews the instance, so the
            // detached interval that expired is re-checked atomically with
            // the transition to `Stopping`. The deadline is not idleness and
            // stops unconditionally above.
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
                    tracing::info!(event_name = "browser.grace.renewed", browser_instance = %instance.instance_id, "browser grace stop abandoned: the instance was attached meanwhile");
                }
            }
        }
        if claim_scan(instance, now)
            && let Err(reason) = self.recheck_periodic(instance)
        {
            let event = self.stop_instance(instance, reason, false, None).await;
            self.send_state(event);
            return true;
        }
        if let Ok(mut control) = instance.control.try_lock() {
            match control.next_event(Duration::from_millis(20)).await {
                Ok(Some(LauncherEvent::Exited { process, code })) => {
                    drop(control);
                    tracing::warn!(event_name = "browser.process.exited", browser_instance = %instance.instance_id, %process, exit_code = ?code, "browser process exited");
                    let event = self
                        .stop_instance(instance, Reason::Failed, false, None)
                        .await;
                    self.send_state(event);
                    return true;
                }
                Ok(Some(LauncherEvent::Error { kind, message })) => {
                    tracing::warn!(event_name = "browser.launcher.error", browser_instance = %instance.instance_id, browser_cause = ?kind, %message, "launcher reported an error");
                }
                Ok(_) => {}
                Err(error) => {
                    drop(control);
                    tracing::warn!(event_name = "browser.control.failed", %error, browser_instance = %instance.instance_id, "launcher control socket failed");
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
        false
    }

    /// Cheap confinement evidence: pins, socket scan and runtime directory.
    /// A scan the ptrace access check denied is [`Recheck::Unverified`]:
    /// operations refuse it, and only periodic supervision may wait for it.
    pub(super) fn recheck(&self, instance: &Arc<Instance>) -> Result<(), Recheck> {
        {
            let live = instance.live.lock();
            if !live.verified.pins_hold() {
                // Pinned processes exit during a deliberate stop (which
                // enters `Stopping` before it stops anything): that is not a
                // finding, and latching it would discard the stop's final
                // snapshot. Otherwise a lost pin is latched like any other.
                let stopping = live.phase == Phase::Stopping;
                drop(live);
                if !stopping {
                    latch_forced(instance, Reason::ConfinementUnverified);
                }
                return Err(Recheck::Stop(Reason::ConfinementUnverified));
            }
        }
        let checked = self.recheck_unlatched(instance);
        if let Err(Recheck::Stop(reason)) = checked {
            // A definite finding is latched like the denial limit, so no
            // renewed use can abandon the stop it requires and nothing is
            // published meanwhile, whichever caller found it.
            latch_forced(instance, reason);
        }
        checked
    }

    /// The socket scan and runtime directory checks.
    fn recheck_unlatched(&self, instance: &Arc<Instance>) -> Result<(), Recheck> {
        let denied = match verify::scan_sockets_periodic(&instance.cgroup_dir) {
            Ok(()) => false,
            Err(verify::ScanFailure::Violation(violation)) => {
                tracing::warn!(event_name = "browser.scan.violation",
                    browser_instance = %instance.instance_id, detail = %violation,
                    "browser socket scan violation");
                return Err(Recheck::Stop(Reason::ListenerDetected));
            }
            Err(verify::ScanFailure::Denied(violation)) => {
                tracing::info!(event_name = "browser.scan.denied",
                    browser_instance = %instance.instance_id, detail = %violation,
                    "browser socket scan denied");
                true
            }
        };
        // Checked even after a denial: an observable violation stops now.
        if let Err(violation) = verify::runtime_directory_clean(&instance.runtime_dir) {
            tracing::warn!(event_name = "browser.runtime_dir.violation",
                browser_instance = %instance.instance_id, detail = %violation,
                "browser runtime directory violation");
            return Err(Recheck::Stop(Reason::ConfinementUnverified));
        }
        if denied {
            return Err(Recheck::Unverified);
        }
        Ok(())
    }

    /// [`Self::recheck`] for the supervisor's periodic scan, also run by a
    /// graceful stop waiting for a copy (which blocks the supervisor): a
    /// denial is rescanned about a second later, and the instance stops
    /// once [`SCAN_DENIED_LIMIT`] consecutive denials were counted.
    pub(super) fn recheck_periodic(&self, instance: &Arc<Instance>) -> Result<(), Reason> {
        match self.recheck(instance) {
            Ok(()) => {
                instance.live.lock().scan_denied = 0;
                Ok(())
            }
            Err(Recheck::Stop(reason)) => Err(reason),
            Err(Recheck::Unverified) => {
                let attempt = {
                    let mut live = instance.live.lock();
                    live.scan_denied = live.scan_denied.saturating_add(1);
                    if let Some(soon) = Instant::now()
                        .checked_sub(SOCKET_SCAN_INTERVAL.saturating_sub(SCAN_DENIED_RETRY))
                    {
                        live.last_scan = soon;
                    }
                    live.scan_denied
                };
                if attempt >= SCAN_DENIED_LIMIT {
                    tracing::warn!(event_name = "browser.scan.unverified",
                        browser_instance = %instance.instance_id, attempt,
                        "browser socket scan denied repeatedly");
                    // Latched for every stop, including one already
                    // waiting for the lifecycle lock; a running copy is
                    // cancelled and no later copy is published.
                    latch_forced(instance, Reason::ConfinementUnverified);
                    return Err(Reason::ConfinementUnverified);
                }
                Ok(())
            }
        }
    }
}

/// Why a confinement recheck did not pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Recheck {
    /// A finding: the instance must stop for this reason.
    Stop(Reason),
    /// Only the ptrace access check denied some evidence: unverified, so
    /// an operation is refused, but nothing was found.
    Unverified,
}

/// Latch a stop that must be forced (the first reason wins) and cancel any
/// running copy.
pub(super) fn latch_forced(instance: &Instance, reason: Reason) {
    instance.latch.lock().get_or_insert(reason);
    instance.cancel_copy();
}

/// Claim the instance's next counted periodic scan: `true` (and the
/// schedule advanced) when one is due. The supervisor and every stop
/// waiting for a copy share this one schedule, so concurrent callers
/// never consume denial attempts faster than the retry interval.
pub(super) fn claim_scan(instance: &Instance, now: Instant) -> bool {
    let mut live = instance.live.lock();
    if live.last_scan + SOCKET_SCAN_INTERVAL <= now {
        live.last_scan = now;
        true
    } else {
        false
    }
}

/// Consecutive denied periodic scans tolerated before the instance stops.
const SCAN_DENIED_LIMIT: u8 = 3;
/// How soon a denied scan is repeated.
const SCAN_DENIED_RETRY: Duration = Duration::from_secs(1);

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
