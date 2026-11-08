//! The systemd operations restart reconciliation and confirmed termination
//! depend on, behind one seam so tests can drive synthetic unit inventories.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use anyhow::Result;

use super::unit::{self, UnitStatus};

pub(super) type UnitFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Listing, observing and terminating `cowboy-browser-*` units.
pub(super) trait UnitControl: Send + Sync {
    /// Every loaded browser unit (see [`unit::list_units`]).
    fn list(&self) -> UnitFuture<'_, Result<Vec<String>>>;

    /// One unit's state (see [`unit::show`]).
    fn show<'a>(&'a self, unit: &'a str) -> UnitFuture<'a, Result<UnitStatus>>;

    /// Confirmed, invocation-bound termination (see [`unit::terminate`]).
    fn terminate<'a>(
        &'a self,
        unit: &'a str,
        cgroup_dir: Option<&'a Path>,
        invocation_id: Option<&'a str>,
    ) -> UnitFuture<'a, bool>;
}

/// The user systemd manager.
pub(super) struct Systemd;

impl UnitControl for Systemd {
    fn list(&self) -> UnitFuture<'_, Result<Vec<String>>> {
        Box::pin(unit::list_units())
    }

    fn show<'a>(&'a self, unit: &'a str) -> UnitFuture<'a, Result<UnitStatus>> {
        Box::pin(unit::show(unit))
    }

    fn terminate<'a>(
        &'a self,
        unit: &'a str,
        cgroup_dir: Option<&'a Path>,
        invocation_id: Option<&'a str>,
    ) -> UnitFuture<'a, bool> {
        Box::pin(unit::terminate(unit, cgroup_dir, invocation_id))
    }
}
