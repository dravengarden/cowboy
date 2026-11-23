//! Server code reads, git/diff/file API, code context, and Zed session boundary.

pub use crate::code_adapter::{CodeAdapterRequest, CodeAdapterResponse, CodeOperation};
pub use crate::code_review::{CodeProvider as _, LocalCodeProvider};
pub use crate::core::{CodeReadScope, Hub, SessionOrigin, Status};
pub use crate::diff_snapshot::{DiffSnapshotCache, DiffSnapshotKey};
pub use crate::machine_control::MachineControl;
pub use crate::product_auth::ProductPrincipal;
pub use crate::store::Store;
pub use crate::supervisor::Supervisor;
pub use crate::usage::now_ms;
pub use anyhow::{Context as _, Result};
pub use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
pub use base64::Engine as _;
pub use cowboy_server_auth::server::{
    AuthenticatedProductRequest, ProductRequestAuth, no_store_json, operator_approval,
    product_continuation, web_security,
};
pub use cowboy_server_code_buffers::server::code_buffers;
pub use cowboy_server_code_buffers::server::zed_language_target;
pub use serde::{Deserialize, Serialize};
pub use sha2::{Digest as _, Sha256};
pub use std::collections::{BTreeSet, HashMap};
pub use std::path::{Path as FsPath, PathBuf};
pub use std::sync::Arc;
#[cfg(test)]
pub use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
#[cfg(test)]
pub use tokio::sync::mpsc;

pub mod code_api;
pub mod code_context;
pub mod code_reads;
pub mod routes;
pub mod types;
pub mod workspace_extensions;
pub mod zed_session;

#[cfg(test)]
mod workspace_code_context_tests;

#[cfg(test)]
mod zed_adapter_tests;

pub use code_api::*;
pub use code_context::*;
pub use types::*;

pub struct CodeState {
    pub service_id: String,
    pub hub: Hub,
    pub store: Option<Store>,
    pub supervisor: Arc<Supervisor>,
    pub machine_control: Arc<MachineControl>,
    pub code_cache: crate::code_cache::CodeCache,
    pub diff_snapshots: DiffSnapshotCache,
    pub file_cursors: code_reads::file_pages::PageCursors,
    pub zed_adapter_socket: Option<PathBuf>,
    pub colocated_machines: BTreeSet<String>,
    pub code_navigation_admission: bool,
    pub product_auth_enabled: bool,
    pub device_access: Arc<crate::client_auth::DeviceAccessSessions>,
    pub product_authentication: Arc<crate::auth_plugins::ProductAuthentication>,
}

impl<'a> From<&'a CodeState> for ProductRequestAuth<'a> {
    fn from(state: &'a CodeState) -> Self {
        Self {
            product_auth_enabled: state.product_auth_enabled,
            store: state.store.as_ref(),
            hub: &state.hub,
            device_access: &state.device_access,
            product_authentication: &state.product_authentication,
        }
    }
}

pub(crate) type AppState = CodeState;
