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
        {
            let mut live = instance.live.lock();
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
        let attached = instance
            .live
            .lock()
            .attachment
            .as_ref()
            .is_some_and(|attachment| attachment.id == attachment_id);
        if !attached {
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Superseded);
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
        let resized = instance
            .control
            .lock()
            .await
            .resize(width, height, COMMAND_TIMEOUT)
            .await;
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
        if let Err(reason) = self.navigate_checked(&instance, url).await {
            return Ok(self
                .stop_instance(&instance, reason, false, Some(request_id))
                .await);
        }
        Ok(instance.state(Some(request_id.to_owned())))
    }

    /// Full confinement verification, then navigation. A violation is
    /// returned, never acted on here, so a caller holding the account
    /// lifecycle lock stops through `stop_locked` and every other caller
    /// through `stop_instance`.
    pub(super) async fn navigate_checked(
        &self,
        instance: &Arc<Instance>,
        url: &str,
    ) -> Result<(), Reason> {
        let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
        let sealed = host::enforce_label(SEALED_PROFILE);
        let supervisor = instance.live.lock().verified.supervisor.pid;
        let verified = verify::verify(
            &self.expectations(instance, supervisor, &bootstrap, &sealed),
            &instance.report,
        )
        .map_err(|violation| {
            tracing::warn!(detail = %violation, "browser confinement failed before navigation");
            Reason::ConfinementUnverified
        })?;
        instance.live.lock().verified = verified;
        let navigated = instance
            .control
            .lock()
            .await
            .navigate(url, COMMAND_TIMEOUT)
            .await;
        if let Err(error) = navigated {
            tracing::warn!(%error, "browser navigation failed");
        }
        Ok(())
    }
}
