//! The Controller server on top of the shared cowboy crate.
//!
//! Every base module is re-exported at this crate's root so the server code
//! keeps addressing them as crate::<module>.

pub use cowboy::*;

mod server;

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod controller_policy_tests;
#[cfg(test)]
mod plugin_catalog_tests;

/// Register the server entry points the shared CLI and supervisor call.
pub fn install_controller_hooks() {
    cowboy::controller_hooks::install(cowboy::controller_hooks::ControllerHooks {
        serve: |args| Box::pin(server::serve(args)),
        init_tracing: server::init_tracing,
        launch_extensions: server::launch_extensions,
    });
}
