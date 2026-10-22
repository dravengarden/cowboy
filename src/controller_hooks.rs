//! Entry points that only the Controller server crate can provide.
//!
//! The server lives in its own crate on top of this one, so shared code that
//! must start it or ask it for per-session policy goes through these hooks.
//! `cowboy-server` installs them before parsing the command line.

use std::{future::Future, pin::Pin, sync::OnceLock};

pub type LocalFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pub struct ControllerHooks {
    pub serve: fn(crate::cli::ServeArgs) -> LocalFuture<anyhow::Result<()>>,
    pub init_tracing: fn(),
    pub launch_extensions:
        fn(&crate::core::Hub, &crate::core::SessionMeta) -> crate::runtime_wire::SessionExtensions,
}

static HOOKS: OnceLock<ControllerHooks> = OnceLock::new();

pub fn install(hooks: ControllerHooks) {
    let _ = HOOKS.set(hooks);
}

pub async fn serve(args: crate::cli::ServeArgs) -> anyhow::Result<()> {
    match HOOKS.get() {
        Some(hooks) => (hooks.serve)(args).await,
        None => anyhow::bail!(
            "this binary does not include the Controller server; run cowboy-controller (the cowboy-server package) instead"
        ),
    }
}

pub fn init_tracing() {
    if let Some(hooks) = HOOKS.get() {
        (hooks.init_tracing)();
    }
}

pub fn launch_extensions(
    hub: &crate::core::Hub,
    session: &crate::core::SessionMeta,
) -> crate::runtime_wire::SessionExtensions {
    match HOOKS.get() {
        Some(hooks) => (hooks.launch_extensions)(hub, session),
        None => crate::runtime_wire::SessionExtensions {
            schema: crate::runtime_wire::SESSION_EXTENSIONS_SCHEMA,
            memory_tools: false,
            memory_recall: false,
        },
    }
}
