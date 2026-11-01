//! The Machine host on top of the shared cowboy crate.
//!
//! Every base module is re-exported at this crate's root so the moved code
//! keeps addressing them as crate::<module>.

pub use cowboy::*;

#[cfg(feature = "machine-host")]
pub mod execution_host;
#[cfg(feature = "machine-host")]
pub mod machine_broker;
#[cfg(feature = "machine-host")]
pub mod machine_cli;
#[cfg(feature = "machine-host")]
pub mod machine_code_plugins;
#[cfg(feature = "machine-host")]
pub mod machine_plugins;
#[cfg(feature = "machine-host")]
pub mod memory_relay;

#[cfg(all(test, target_os = "linux", feature = "machine-host"))]
mod cgroup_containment_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod machine_control_binding_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod machine_control_matrix_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod worker_execution_tests;
#[cfg(all(test, feature = "full", feature = "machine-host"))]
mod worker_trace_tests;
