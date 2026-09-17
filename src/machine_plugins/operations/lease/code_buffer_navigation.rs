//! A one-command invocation issued by the actual enrolled connection, never
//! decoded from a wire declaration, read lease or private navigation reference.
use super::{Arc, AtomicBool, Duration, Ordering, PluginExecutionScope};
use crate::machine_protocol::code_buffer_navigation::Request;
use anyhow::{Result, ensure};
use tokio::time::Instant;

const COMMAND_BUDGET: Duration = Duration::from_secs(15);

pub(crate) struct CodeNavigationInvocation {
    request: Request,
    connection: Arc<AtomicBool>,
    deadline: Instant,
}

pub(crate) struct CodeNavigationOwner {
    connection: Arc<AtomicBool>,
}

impl PluginExecutionScope {
    pub(crate) fn code_navigation(&self, request: Request) -> Result<CodeNavigationInvocation> {
        request.validate()?;
        ensure!(
            self.connected.load(Ordering::Acquire)
                && self.service.as_deref() == Some(request.service_id.as_str())
                && self.machine == request.machine_id,
            "navigation connection owner changed"
        );
        Ok(CodeNavigationInvocation {
            request,
            connection: Arc::clone(&self.connected),
            deadline: Instant::now() + COMMAND_BUDGET,
        })
    }
}

impl CodeNavigationInvocation {
    pub(crate) fn request(&self) -> &Request {
        &self.request
    }

    pub(crate) fn remaining(&self) -> Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        ensure!(
            self.connection.load(Ordering::Acquire) && !remaining.is_zero(),
            "navigation authorization ended"
        );
        Ok(remaining)
    }

    pub(crate) fn owner(&self) -> Result<CodeNavigationOwner> {
        self.remaining()?;
        Ok(CodeNavigationOwner {
            connection: Arc::clone(&self.connection),
        })
    }

    pub(crate) fn check_owner(&self, owner: &CodeNavigationOwner) -> Result<()> {
        self.remaining()?;
        ensure!(
            Arc::ptr_eq(&self.connection, &owner.connection),
            "navigation original connection changed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
