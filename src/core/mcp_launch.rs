//! A Controller-owned resolver shared by supervisor and runtime delivery.
use super::{Hub, SessionMeta};
use crate::mcp_binding::NativeLaunchPlan;
use std::sync::Arc;

pub type Resolver = dyn Fn(&SessionMeta) -> anyhow::Result<Box<NativeLaunchPlan>> + Send + Sync;

impl Hub {
    pub fn set_mcp_launch_resolver(&self, resolver: Arc<Resolver>) {
        *self.inner.mcp_launch.lock() = Some(resolver);
    }

    pub fn resolve_mcp_launch(
        &self,
        session: &SessionMeta,
    ) -> anyhow::Result<Box<NativeLaunchPlan>> {
        let resolver = self.inner.mcp_launch.lock().clone().ok_or_else(|| {
            anyhow::anyhow!("MCP launch preparation is not ready for this session")
        })?;
        resolver(session)
    }
}
