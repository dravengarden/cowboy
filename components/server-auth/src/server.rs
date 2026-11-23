//! Server auth, admin API, HTTP policy, secure transport, continuation, and approval modules.

pub use crate::agent_model::Status;
pub use crate::core::{Hub, Inbound, Outbound, PersistenceHealth};
pub use crate::product_auth::ProductPrincipal;
pub use crate::runtime::RuntimeHealth;
pub use crate::runtime_router::RuntimeRouter;
pub use crate::store::{ActiveClientRelease, Store};
pub use anyhow::{Context as _, Result};
pub use axum::{
    Extension, Form, Json, Router,
    body::Body,
    extract::{
        ConnectInfo, Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Redirect, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{any, delete, get, post, put},
};
pub use base64::Engine as _;
pub use futures::{SinkExt as _, StreamExt as _};
pub use serde::{Deserialize, Serialize};
pub use sha2::{Digest as _, Sha256};
pub use std::collections::HashMap;
pub use std::net::SocketAddr;
pub use std::path::PathBuf;
pub use std::sync::Arc;

pub mod admin_api;
pub mod cardea_grant;
pub mod http_policy;
pub mod operator_approval;
pub mod product_auth_api;
pub mod product_continuation;
pub mod secure_transport;
pub mod types;

pub use admin_api::*;
pub use http_policy::*;
pub use product_auth_api::*;
pub use types::*;

#[cfg(test)]
mod product_auth_api_tests;
