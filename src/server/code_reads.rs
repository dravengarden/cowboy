//! Controller-owned observation around a complete buffered code-reader response.
//!
//! Keep cache hits, conditional responses and errors inside the same boundary as
//! local/remote I/O. This rejects stale replies; it is neither authorization nor
//! a fence or compensation for work already dispatched by the reader.

use std::future::Future;

pub(super) mod file_pages;
pub(super) mod workspace;

use super::{
    AppState, IntoResponse as _, ResolvedCodeContext, Response, StatusCode,
    code_context_is_current, header, resolve_code_context,
};

pub(super) async fn scoped<F: Future<Output = Response>>(
    state: &AppState,
    id: &str,
    read: impl FnOnce(ResolvedCodeContext) -> F,
) -> Response {
    let Some(context) = resolve_code_context(state, id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let scope = context.scope.clone();
    guarded_response(
        || code_context_is_current(state, id, &scope),
        || read(context),
    )
    .await
}

async fn guarded_response<C: Future<Output = bool>, R: Future<Output = Response>>(
    current: impl Fn() -> C,
    read: impl FnOnce() -> R,
) -> Response {
    if !current().await {
        return stale_response();
    }
    let response = read().await;
    if !current().await {
        // Replace the entire response, including stale bytes, ETag and 304.
        return stale_response();
    }
    response
}

fn stale_response() -> Response {
    (
        StatusCode::GONE,
        [(header::CACHE_CONTROL, "no-store")],
        "code context changed",
    )
        .into_response()
}

#[cfg(test)]
mod tests;
