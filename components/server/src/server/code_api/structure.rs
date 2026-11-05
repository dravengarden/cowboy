//! Syntax structure (`outline@1`, `symbols@1`) from the bundled grammars.
//! It runs no project code, so restricted worktrees and Machines without the
//! language engine keep an outline. Answers are heuristic.

use super::*;
use crate::code_review::structure::{StructureSymbol, WorkspaceSymbols};

fn refused(_: anyhow::Error) -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "code structure is unavailable on this Machine".to_owned(),
    )
}

async fn local<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, (StatusCode, String)> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "structure failed".to_owned(),
            )
        })?
        .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error))
}

pub(in crate::server) async fn outline(
    state: &AppState,
    scope: &CodeReadScope,
    cwd: String,
    path: String,
) -> Result<Vec<StructureSymbol>, (StatusCode, String)> {
    match remote_code_request(state, scope, CodeOperation::Outline { path: path.clone() }).await {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::Outline(symbols))) => Ok(symbols),
        Ok(Some(_)) => Err(refused(anyhow::anyhow!("wrong response"))),
        Err(error) => Err(refused(error)),
        Ok(None) => {
            local(move || crate::code_review::structure::outline(std::path::Path::new(&cwd), &path))
                .await
        }
    }
}

pub(in crate::server) async fn workspace_symbols(
    state: &AppState,
    scope: &CodeReadScope,
    cwd: String,
    query: String,
    limit: usize,
) -> Result<WorkspaceSymbols, (StatusCode, String)> {
    let operation = CodeOperation::WorkspaceSymbols {
        query: query.clone(),
        limit,
    };
    match remote_code_request(state, scope, operation).await {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::WorkspaceSymbols(found))) => Ok(found),
        Ok(Some(_)) => Err(refused(anyhow::anyhow!("wrong response"))),
        Err(error) => Err(refused(error)),
        Ok(None) => {
            local(move || {
                crate::code_review::structure::workspace_symbols(
                    std::path::Path::new(&cwd),
                    &query,
                    limit,
                )
            })
            .await
        }
    }
}
