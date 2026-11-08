//! Machine browser instances: owner-operated browsers that run on this
//! Machine for one Provider account each, isolated from every other account's
//! browser and from the Machine's own state (`docs/machine-browser.md`).
//!
//! The Plugin ships the bytes; everything that decides whether and how they
//! run is here: admission, the systemd unit, confinement verification from
//! outside the instance, attachment leases, profile snapshots, reset and
//! revocation fences, restart reconciliation and sign-in binding.

pub mod host;
pub mod launcher;
pub mod manager;
pub mod settings;
pub mod snapshot;
pub mod state_file;
pub mod stream;
pub mod unit;
pub mod verify;

pub use manager::{BrowserManager, UplinkGuard};
