//! Viewer attachments: streams, leases, detach, resize and navigation.

use super::*;

impl BrowserManager {
    pub(super) async fn attach_viewer(
        self: &Arc<Self>,
        request_id: &str,
        reference: InstanceRef,
        attachment_id: String,
        stream_token: String,
        viewport: Viewport,
    ) -> StateEvent {
        let instance = match self.current(&reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        if !matches!(
            instance.live.lock().phase,
            Phase::Detached | Phase::Attached
        ) {
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Busy);
        }
        if let Err(violation) = self.recheck(&instance) {
            return self
                .stop_instance(&instance, violation, false, Some(request_id))
                .await;
        }
        let endpoint = match self
            .uplink
            .lock()
            .as_ref()
            .map(|uplink| uplink.endpoint.clone())
        {
            Some(endpoint) => endpoint,
            None => {
                return instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Failed);
            }
        };
        let current = instance.live.lock().geometry;
        if (current.width, current.height) != (viewport.width, viewport.height) {
            if !instance
                .settings
                .geometry_allowed(viewport.width, viewport.height)
            {
                self.send_result(
                    request_id.to_owned(),
                    false,
                    Some("browser viewport exceeds this Device's bound".into()),
                );
                return instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Failed);
            }
            let resized = instance
                .control
                .lock()
                .await
                .resize(viewport.width, viewport.height, COMMAND_TIMEOUT)
                .await;
            match resized {
                Ok(()) => {
                    instance.live.lock().geometry = Geometry {
                        width: viewport.width,
                        height: viewport.height,
                    };
                }
                Err(error) => {
                    tracing::warn!(%error, "browser resize at attach failed");
                }
            }
        }
        let weak = Arc::downgrade(self);
        let end_instance = Arc::downgrade(&instance);
        let end_attachment = attachment_id.clone();
        let stream = Stream::start(
            endpoint,
            stream_token,
            instance
                .runtime_dir
                .join(crate::machine_browser::launcher::RFB_SOCKET),
            Box::new(move |end, _counters| {
                if let (Some(manager), Some(instance)) = (weak.upgrade(), end_instance.upgrade()) {
                    manager.stream_ended(&instance, &end_attachment, &end);
                }
            }),
        );
        // The phase was read before the resize was awaited; a stop (reset,
        // revocation, hibernation) may have begun meanwhile. Every stop sets
        // `Stopping` and takes the attachment under this lock, so checking
        // here, where the attachment is installed, is atomic with it.
        let refused = {
            let mut live = instance.live.lock();
            if matches!(live.phase, Phase::Detached | Phase::Attached) {
                if let Some(previous) = live.attachment.take()
                    && let Some(previous_stream) = previous.stream
                {
                    previous_stream.cancel();
                }
                live.attachment = Some(Attachment {
                    id: attachment_id.clone(),
                    lease_expires: Instant::now()
                        + Duration::from_millis(u64::try_from(LEASE_EXPIRY_MS).unwrap_or(30_000)),
                    stream: Some(stream),
                });
                live.phase = Phase::Attached;
                live.detached_since = None;
                None
            } else {
                Some(stream)
            }
        };
        if let Some(stream) = refused {
            stream.cancel();
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Busy);
        }
        if let Err(error) = instance
            .control
            .lock()
            .await
            .attach(&attachment_id, COMMAND_TIMEOUT)
            .await
        {
            tracing::debug!(%error, "launcher attach bookkeeping failed");
        }
        instance.state(Some(request_id.to_owned()))
    }

    pub(super) fn stream_ended(
        self: &Arc<Self>,
        instance: &Arc<Instance>,
        attachment_id: &str,
        end: &StreamEnd,
    ) {
        let detached = {
            let mut live = instance.live.lock();
            if live
                .attachment
                .as_ref()
                .is_some_and(|attachment| attachment.id == attachment_id)
            {
                live.attachment = None;
                if live.phase == Phase::Attached {
                    live.phase = Phase::Detached;
                    live.detached_since = Some(Instant::now());
                }
                true
            } else {
                false
            }
        };
        if detached {
            tracing::info!(?end, instance = %instance.instance_id, "browser viewer detached");
            self.send_state(instance.state(None));
        }
    }

    pub(super) fn lease(
        &self,
        request_id: &str,
        reference: &InstanceRef,
        attachment_id: &str,
        expires_at_ms: i64,
    ) -> Option<StateEvent> {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return Some((*event).with_request(request_id)),
        };
        let now = Instant::now();
        let renewed = {
            let mut live = instance.live.lock();
            match live.attachment.as_mut() {
                // Expiry is monotonic: a late or stale renewal cannot resurrect.
                Some(attachment)
                    if attachment.id == attachment_id && attachment.lease_expires > now =>
                {
                    let bound = now
                        + Duration::from_millis(u64::try_from(LEASE_EXPIRY_MS).unwrap_or(30_000));
                    let requested = now
                        + Duration::from_millis(
                            u64::try_from(expires_at_ms.saturating_sub(unix_ms())).unwrap_or(0),
                        );
                    attachment.lease_expires = bound.min(requested).max(now);
                    true
                }
                _ => false,
            }
        };
        // `state` takes the live lock itself: the guard above is released.
        (!renewed).then(|| {
            instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Superseded)
        })
    }

    pub(super) fn detach(
        &self,
        request_id: &str,
        reference: &InstanceRef,
        attachment_id: &str,
    ) -> StateEvent {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        let mut live = instance.live.lock();
        if live
            .attachment
            .as_ref()
            .is_some_and(|attachment| attachment.id == attachment_id)
        {
            if let Some(attachment) = live.attachment.take()
                && let Some(stream) = attachment.stream
            {
                stream.cancel();
            }
            live.phase = Phase::Detached;
            live.detached_since = Some(Instant::now());
            drop(live);
            return instance.state(Some(request_id.to_owned()));
        }
        drop(live);
        instance
            .state(Some(request_id.to_owned()))
            .with_reason(Reason::Superseded)
    }

    pub(super) async fn resize(
        self: &Arc<Self>,
        request_id: &str,
        reference: &InstanceRef,
        attachment_id: &str,
        width: u32,
        height: u32,
    ) -> StateEvent {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        let superseded = |instance: &Instance| {
            instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Superseded)
        };
        if !instance.live.lock().holds(attachment_id, Instant::now()) {
            return superseded(&instance);
        }
        if !instance.settings.geometry_allowed(width, height) {
            self.send_result(
                request_id.to_owned(),
                false,
                Some("browser geometry exceeds this Device's bound".into()),
            );
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Failed);
        }
        if let Err(violation) = self.recheck(&instance) {
            return self
                .stop_instance(&instance, violation, false, Some(request_id))
                .await;
        }
        let resized = {
            let mut control = instance.control.lock().await;
            // The control connection may have been held by a snapshot or a
            // navigation meanwhile: a takeover, a detach, a stop or the
            // lease's expiry ends this attachment's authority over the
            // geometry. Rechecked under the live lock, released before the
            // command is awaited; the control lock held from here on orders
            // this resize before any later one.
            if !instance.live.lock().holds(attachment_id, Instant::now()) {
                drop(control);
                return superseded(&instance);
            }
            control.resize(width, height, COMMAND_TIMEOUT).await
        };
        match resized {
            Ok(()) => {
                instance.live.lock().geometry = Geometry { width, height };
                instance.state(Some(request_id.to_owned()))
            }
            Err(error) => {
                tracing::warn!(%error, "browser resize failed");
                instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Failed)
            }
        }
    }

    pub(super) async fn navigate(
        self: &Arc<Self>,
        request_id: &str,
        reference: &InstanceRef,
        url: &str,
    ) -> Result<StateEvent, String> {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return Ok((*event).with_request(request_id)),
        };
        if instance.purpose() != Purpose::Tab {
            return Err("only tab browsers accept owner navigation".into());
        }
        match self.navigate_checked(&instance, &Purpose::Tab, url).await {
            Ok(()) => Ok(instance.state(Some(request_id.to_owned()))),
            Err(Navigation::Superseded) => Ok(instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Superseded)),
            Err(Navigation::Violation(reason)) => Ok(self
                .stop_instance(&instance, reason, false, Some(request_id))
                .await),
            Err(Navigation::Rejected) => Ok(instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Failed)),
        }
    }

    /// Full confinement verification, then navigation. A violation is
    /// returned, never acted on here, so a caller holding the account
    /// lifecycle lock stops through `stop_locked` and every other caller
    /// through `stop_instance`. A navigation the launcher rejects or does
    /// not answer in time is [`Navigation::Rejected`]: the browser keeps
    /// running (still verified) but shows a blank or earlier page, so a
    /// caller answering a command reports `failed`; the URL is remembered
    /// until a later navigation succeeds.
    ///
    /// A snapshot copy binds `/mnt/publish` in the instance's mount
    /// namespace, which verification rightly rejects. So no copy runs while
    /// this verifies and navigates: new copies are refused, and a running
    /// one is awaited (or cancelled when it overruns). A caller holding the
    /// lifecycle lock must quiesce copies before taking it (a running copy
    /// needs the lock to publish).
    ///
    /// `expected` is the binding the navigation was requested under. It is
    /// checked under the instance's navigation lock, which sign-in
    /// rebinding also takes, so a navigation for a login (or a tab) the
    /// instance no longer serves is [`Navigation::Superseded`] and never
    /// reaches the launcher.
    pub(super) async fn navigate_checked(
        &self,
        instance: &Arc<Instance>,
        expected: &Purpose,
        url: &str,
    ) -> Result<(), Navigation> {
        let _copies = self.quiesce_copies(instance).await;
        let _navigation = instance.navigation.lock().await;
        if instance.purpose() != *expected || !instance.live.lock().navigable() {
            return Err(Navigation::Superseded);
        }
        #[cfg(test)]
        let trusted = self
            .trust_confinement
            .load(std::sync::atomic::Ordering::SeqCst);
        #[cfg(not(test))]
        let trusted = false;
        if !trusted {
            let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
            let sealed = host::enforce_label(SEALED_PROFILE);
            let supervisor = instance.live.lock().verified.supervisor.pid;
            let verified = verify::verify(
                &self.expectations(instance, supervisor, &bootstrap, &sealed),
                &instance.report,
            )
            .map_err(|violation| {
                tracing::warn!(detail = %violation, "browser confinement failed before navigation");
                Navigation::Violation(Reason::ConfinementUnverified)
            })?;
            instance.live.lock().verified = verified;
        }
        let navigated = instance
            .control
            .lock()
            .await
            .navigate(url, NAVIGATE_TIMEOUT)
            .await;
        if let Err(error) = &navigated
            && error.downcast_ref::<LauncherRefusal>().is_none()
        {
            // No reply: the helper may still navigate later, under whatever
            // binding the instance has by then. The instance is not reused.
            tracing::warn!(%error, instance = %instance.instance_id, "browser navigation outcome unknown");
            // Fenced before the navigation lock is released, so a waiting
            // rebinding sees it; the caller then stops the instance.
            instance.live.lock().navigation_unsettled = true;
            return Err(Navigation::Violation(Reason::Failed));
        }
        let mut live = instance.live.lock();
        if let Err(error) = navigated {
            tracing::warn!(%error, instance = %instance.instance_id, "browser navigation failed");
            live.failed_navigation = Some((expected.clone(), url.to_owned()));
            return Err(Navigation::Rejected);
        }
        live.failed_navigation = None;
        Ok(())
    }
}

/// Why [`BrowserManager::navigate_checked`] did not navigate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Navigation {
    /// Confinement failed before the navigation, or the launcher never
    /// answered it (its effect is unsettled): the instance must stop.
    Violation(Reason),
    /// The launcher answered with a refusal (its helper ended or was
    /// killed); the instance is intact and a later navigation may succeed.
    Rejected,
    /// The instance no longer serves the binding the navigation was
    /// requested under, or is stopping: nothing was sent.
    Superseded,
}

impl Live {
    /// Whether the instance may navigate or be rebound to a sign-in: it is
    /// running and no earlier navigation's effect is still unsettled.
    pub(super) fn navigable(&self) -> bool {
        matches!(self.phase, Phase::Detached | Phase::Attached) && !self.navigation_unsettled
    }

    /// Whether `attachment_id` is the instance's current, unexpired
    /// attachment of an attached instance: the authority a viewer command
    /// such as resize acts under.
    pub(super) fn holds(&self, attachment_id: &str, now: Instant) -> bool {
        self.phase == Phase::Attached
            && self.attachment.as_ref().is_some_and(|attachment| {
                attachment.id == attachment_id && attachment.lease_expires > now
            })
    }
}
