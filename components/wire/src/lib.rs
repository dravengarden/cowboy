//! Shared wire contracts between the Controller, Machine host and workers.
//!
//! Everything here is plain data plus validation; nothing depends on the
//! Controller's store, server or plugin runtime. Keeping it a separate crate
//! lets Cargo compile it once for every Cowboy feature slice.

pub mod agent_api;
pub mod execution_environment;
pub mod execution_protocol;
pub mod machine_auth;
pub mod machine_protocol;
pub mod managed_calls;
pub mod otlp;
pub mod provider_account;
pub mod runtime_trace;
pub mod runtime_wire;
pub mod service_identity;
