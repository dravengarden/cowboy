//! Capture only the original retained process; navigation has no selector or
//! path fallback. Destination reservations join ordinary lease accounting.
use super::*;
use crate::machine_plugins::CodeNavigationInvocation;

impl Routes {
    /// Caller owns the original route lock. Preparation/Execute recheck after
    /// waiting; a released source or a synchronization reservation cannot pass.
    pub(in crate::machine_code_plugins) async fn check_navigation_source(
        &self,
        lease: &LeaseRef,
        target: &RetainedTarget,
    ) -> Result<()> {
        let registry = self.registry.lock().await;
        let entry = registry
            .entries
            .get(&("zed".into(), lease.clone()))
            .context("original navigation source is no longer retained")?;
        ensure!(
            entry.state == State::Open
                && entry.until.is_none()
                && Arc::ptr_eq(&entry.sync, &target.marker)
                && !entry.sync.load(Ordering::Acquire),
            "original navigation source changed or is reserved"
        );
        Ok(())
    }

    /// The reservation has no native effect. Record the original buffer route
    /// before publishing its reference; a lost response never needs a path open.
    pub(in crate::machine_code_plugins) async fn prepare_navigation(
        &self,
        reservation: Reservation,
        target: &RetainedTarget,
        route: &mut WorktreeRoute,
        payload: &Value,
        invocation: &CodeNavigationInvocation,
    ) -> Result<LeaseRef> {
        invocation.remaining()?;
        ensure!(
            reservation.until > Instant::now(),
            "destination preparation expired"
        );
        let response = exchange(&target.runtime.socket, payload).await?;
        let reply = Reply::parse(&response)?;
        ensure!(
            reply.state == State::Prepared,
            "navigation destination was not prepared"
        );
        let key = ("zed".to_owned(), reply.lease.clone());
        let mut registry = self.registry.lock().await;
        invocation.remaining()?;
        ensure!(
            reservation.until > Instant::now(),
            "destination preparation expired"
        );
        ensure!(
            !registry.entries.contains_key(&key) && !registry.released.contains(&key),
            "native buffer reference was reused"
        );
        registry.entries.insert(
            key,
            Entry {
                route: Arc::clone(&target.route),
                runtime: Arc::clone(&target.runtime),
                until: Some(reservation.until),
                state: State::Prepared,
                sync: Arc::default(),
                _permit: reservation.permit,
            },
        );
        route.owned_buffers.insert(reply.lease.clone());
        Ok(reply.lease)
    }
}
