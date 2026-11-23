//! Thin `AppState` route binding over `cowboy_server_auth::server::sync_dataset`.

use super::*;
pub(super) use cowboy_server_auth::server::sync_dataset::{
    SUBPROTOCOL, dataset_response, descriptor, matches, offers_protocol, socket_check,
};

pub(super) async fn get_dataset(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
) -> Response {
    dataset_response(&state.service_id, &authenticated.principal)
}
