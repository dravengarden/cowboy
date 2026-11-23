pub use cowboy_server::server::{
    AuthenticatedProductRequest, PluginFenceState, PluginLifecycleFences, ProductRequestAuth,
    http_policy::auth_now_ms, operator_approval,
};

mod plugin_install_integration;
mod provider_auth_sync_integration;

pub mod plugin_install {
    pub use cowboy_server::server::plugin_install::*;
}

pub mod provider_auth_sync {
    pub use cowboy_server::server::provider_auth_sync::*;
}

pub mod telemetry_binding {
    pub use cowboy_server::server::telemetry_binding::{
        Effects, advance, background, coordinate, export, recovery, surface, test_fixtures,
    };

    pub mod tests {
        pub use cowboy_server::server::telemetry_binding::test_fixtures::FixtureEffects;
    }

    pub mod live {
        pub use super::*;
        pub use crate::machine_control::{ConnectionToken, MachineControl};
        pub use crate::machine_protocol::telemetry_binding::{BindingObservation, BindingOutcome};
        pub use crate::plugin_catalog::PluginCatalog;
        pub use crate::store::Store;
        pub use crate::telemetry_binding::{
            Intent, LegacyFence, Operation, Progress, writer::Change,
        };
        pub use crate::telemetry_plugin::writer_admission::WriteScope;
        pub use anyhow::Result;
        pub use cowboy_server::server::telemetry_binding::live::LiveEffects;
        pub use std::sync::{Arc, atomic::Ordering};

        pub mod tests;
    }

    pub mod resolution {
        pub use cowboy_server::server::telemetry_binding::resolution::{
            Observation, resolve_fixture,
        };
        pub mod surface {
            pub use cowboy_server::server::telemetry_binding::resolution::surface::view::OperationView;
            pub mod tests {
                pub use cowboy_server::server::telemetry_binding::test_fixtures::HttpFixture as Fixture;
            }
        }
        pub mod tests {
            use super::Observation;
            use crate::machine_protocol::telemetry_binding::{BindingObservation, BindingStep};
            use anyhow::Result;
            pub use cowboy_server::server::telemetry_binding::resolution::coordinate;
            pub use cowboy_server::server::telemetry_binding::test_fixtures::resolution_authority as authority;
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

            pub struct Observer {
                pub value: BindingObservation,
                pub queries: AtomicUsize,
                pub ended: AtomicBool,
                pub disconnect: bool,
                pub hang: bool,
            }
            impl Observer {
                pub fn new(value: BindingObservation) -> Self {
                    Self {
                        value,
                        queries: AtomicUsize::new(0),
                        ended: AtomicBool::new(false),
                        disconnect: false,
                        hang: false,
                    }
                }
            }
            impl Observation for Observer {
                fn current(&self) -> bool {
                    !self.ended.load(Ordering::Acquire)
                }
                async fn observe(&self, _step: &BindingStep) -> Result<BindingObservation> {
                    self.queries.fetch_add(1, Ordering::Relaxed);
                    if self.hang {
                        std::future::pending::<()>().await;
                    }
                    if self.disconnect {
                        self.ended.store(true, Ordering::Release);
                    }
                    Ok(self.value.clone())
                }
            }

            #[cfg(target_os = "linux")]
            mod reader_conformance;
        }
    }

    mod recovery_audit;
    mod surface_signed;
}
