//! `AgentApiBridge` implementation wiring `cowboy-server-runtime` back into `agent_api`.

use super::*;

pub(in crate::server) struct Bridge;

impl super::super::agent_api_bridge::AgentApiBridge for Bridge {
    fn status(&self, state: &Arc<AppState>, meta: &SessionMeta, grant: &Grant) -> Value {
        status(state, meta, grant)
    }

    fn invoke<'a>(
        &'a self,
        state: &'a Arc<AppState>,
        meta: &'a SessionMeta,
        ledger: &'a Ledger,
        action: &'a str,
        arguments: Value,
        operation: Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + 'a>> {
        Box::pin(invoke(state, meta, ledger, action, arguments, operation))
    }

    fn read<'a>(
        &'a self,
        state: &'a Arc<AppState>,
        connection: &'a ConnectionToken,
        grant: &'a Grant,
        meta: &'a SessionMeta,
        ledger: &'a Ledger,
        program: super::super::agent_api_bridge::AgentApiProgram,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + 'a>> {
        Box::pin(read(
            state,
            connection,
            grant,
            meta,
            ledger,
            Program {
                code: program.code,
                typescript: program.typescript,
                module: program.module,
                timeout_ms: program.timeout_ms,
            },
        ))
    }

    fn recover_decisions<'a>(
        &'a self,
        state: &'a Arc<AppState>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(recover_decisions(state))
    }
}
