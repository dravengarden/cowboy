//! Re-exported auth primitives and the owned code-buffers module.

pub use crate::core::{CodeReadScope, Hub, SessionOrigin};
pub use crate::machine_control::MachineControl;
pub use crate::store::Store;
pub use anyhow::Context as _;
pub use axum::Router;
pub use axum::extract::{DefaultBodyLimit, Extension, Json, Path, Query, State};
#[cfg(test)]
pub use axum::http::Method;
pub use axum::http::{HeaderMap, StatusCode, header};
pub use axum::middleware;
pub use axum::response::{IntoResponse, Response};
pub use axum::routing::{get, post, put};
pub use cowboy_server_auth::server::http_policy::*;
pub use cowboy_server_auth::server::product_auth_api::*;
pub use cowboy_server_auth::server::types::*;
pub use cowboy_server_auth::server::{
    AuthenticatedProductRequest, ProductRequestAuth, RouteAuth, http_policy, operator_approval,
    product_auth_api, product_continuation, types,
};
pub use serde::{Deserialize, Serialize};
pub use std::sync::Arc;
pub use tokio::sync::{mpsc, watch};

pub fn zed_language_target(
    machine_id: &str,
    cwd: &str,
    relative: &str,
) -> Option<(String, String)> {
    if machine_id != "local" {
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains(['\\', '\0'])
            || relative
                .split('/')
                .any(|component| matches!(component, "" | "." | ".."))
        {
            return None;
        }
        return Some((cwd.to_owned(), relative.to_owned()));
    }
    crate::code_review::LocalCodeProvider::new(std::path::Path::new(cwd))
        .language_buffer_key(relative)
        .map(|(worktree, path)| {
            (
                worktree.to_string_lossy().into_owned(),
                path.to_string_lossy().replace('\\', "/"),
            )
        })
}

pub mod code_buffers;
