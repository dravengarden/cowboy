//! Store-crate contracts that need the Hub or other modules above the store.
//!
//! `cowboy-store` cannot depend on this crate, so tests that drive a real
//! store through the Hub, the CLI or the Plugin runtime live here and reach
//! the store through its public (and `test-support`) surface.

mod core_passkeys;
mod core_security;
mod native_sessions;
mod subagent_events;
mod telemetry_bindings;
