//! A one-command grant constructed only by the authenticated connection owner.
//! A recorded request, read lease or native ticket cannot construct this type.

use super::{Arc, AtomicBool, Duration, Ordering, PluginExecutionScope};
use crate::machine_protocol::code_buffer_sync::Request;
use anyhow::{Result, ensure};
use tokio::time::Instant;

const COMMAND_BUDGET: Duration = Duration::from_secs(15);

pub(crate) struct CodeBufferSyncInvocation {
    request: Request,
    connection: Arc<AtomicBool>,
    deadline: Instant,
}

/// Kept with an operation, not with its transport observer. Reconnection cannot
/// renew an old synchronization's authority even for the same Service name.
pub(crate) struct CodeBufferSyncOwner {
    connection: Arc<AtomicBool>,
}

impl PluginExecutionScope {
    pub(crate) fn code_buffer_sync(&self, request: Request) -> Result<CodeBufferSyncInvocation> {
        request.validate()?;
        ensure!(
            self.connected.load(Ordering::Acquire)
                && self.service.as_deref() == Some(request.service_id.as_str())
                && self.machine == request.machine_id,
            "synchronization connection owner changed"
        );
        Ok(CodeBufferSyncInvocation {
            request,
            connection: Arc::clone(&self.connected),
            deadline: Instant::now() + COMMAND_BUDGET,
        })
    }
}

impl CodeBufferSyncInvocation {
    pub(crate) fn request(&self) -> &Request {
        &self.request
    }

    pub(crate) fn remaining(&self) -> Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        ensure!(
            self.connection.load(Ordering::Acquire) && !remaining.is_zero(),
            "synchronization authorization ended"
        );
        Ok(remaining)
    }

    pub(crate) fn owner(&self) -> Result<CodeBufferSyncOwner> {
        self.remaining()?;
        Ok(CodeBufferSyncOwner {
            connection: Arc::clone(&self.connection),
        })
    }

    pub(crate) fn check_owner(&self, owner: &CodeBufferSyncOwner) -> Result<()> {
        self.remaining()?;
        ensure!(
            Arc::ptr_eq(&self.connection, &owner.connection),
            "synchronization original connection changed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
