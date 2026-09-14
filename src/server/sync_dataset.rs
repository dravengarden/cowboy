//! A stable browser dataset identity, not a credential or a serialized grant.
//! Core derives it from the actual Service and authenticated principal. Socket
//! binding closes the cookie-change race between discovery and connection.

use super::*;

pub(super) const SUBPROTOCOL: &str = "cowboy-sync-v1";

#[derive(Serialize)]
pub(super) struct Descriptor {
    schema: &'static str,
    dataset_id: String,
    user_id: String,
    database_version: u16,
    outbox_contract: &'static str,
}

fn identity(service: &str, user: &str) -> String {
    let bytes = serde_json::to_vec(&("cowboy.product-sync.v1", service, user))
        .expect("fixed string tuple is serializable");
    format!("dataset-{}", crate::admin::hex_sha256(&bytes))
}

pub(super) fn descriptor(service: &str, principal: &ProductPrincipal) -> Descriptor {
    Descriptor {
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: identity(service, &principal.user_id),
        user_id: principal.user_id.clone(),
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
    }
}

pub(super) async fn get_dataset(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(descriptor(&state.service_id, &authenticated.principal)),
    )
        .into_response()
}

pub(super) fn matches(service: &str, principal: &ProductPrincipal, supplied: &str) -> bool {
    supplied == identity(service, &principal.user_id)
}

pub(super) fn offers_protocol(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .any(|value| {
            value
                .to_str()
                .is_ok_and(|value| value.split(',').any(|token| token.trim() == SUBPROTOCOL))
        })
}

pub(super) fn socket_check(
    service: &str,
    principal: &ProductPrincipal,
    browser: bool,
    supplied: Option<&str>,
) -> Result<(), StatusCode> {
    match supplied {
        Some(value) if matches(service, principal, value) => Ok(()),
        Some(_) => Err(StatusCode::CONFLICT),
        // The dataset-aware Web and compatible recovery floor are release
        // prerequisites. Never infer an old browser's data identity from the
        // cookie it happens to present now, or reopen that path with a toggle.
        None if browser => Err(StatusCode::UPGRADE_REQUIRED),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_identity_is_stable_but_service_and_principal_never_alias() {
        let mut principal = crate::product_auth::local_product_principal();
        principal.user_id = "user-one".to_owned();
        let expected = descriptor("service-one", &principal);
        assert!(matches("service-one", &principal, &expected.dataset_id));
        principal.username = "renamed".to_owned();
        principal.role = crate::admin::AdminRole::Viewer;
        assert!(matches("service-one", &principal, &expected.dataset_id));
        assert!(!matches("service-two", &principal, &expected.dataset_id));
        principal.user_id = "user-two".to_owned();
        assert!(!matches("service-one", &principal, &expected.dataset_id));
        assert_ne!(identity("a:b", "c"), identity("a", "b:c"));
        assert!(!matches("service-one", &principal, ""));
        assert!(!matches("service-one", &principal, "dataset-untrusted"));
    }

    #[test]
    fn socket_identity_is_not_optional_when_supplied_and_never_grants_a_role() {
        let mut principal = crate::product_auth::local_product_principal();
        principal.role = crate::admin::AdminRole::Viewer;
        let dataset = descriptor("service", &principal);
        for browser in [false, true] {
            assert_eq!(
                socket_check("service", &principal, browser, Some(&dataset.dataset_id)),
                Ok(())
            );
            assert_eq!(
                socket_check("other", &principal, browser, Some(&dataset.dataset_id)),
                Err(StatusCode::CONFLICT)
            );
            assert_eq!(
                socket_check("service", &principal, browser, Some("")),
                Err(StatusCode::CONFLICT)
            );
        }
        assert_eq!(socket_check("service", &principal, false, None), Ok(()));
        assert_eq!(
            socket_check("service", &principal, true, None),
            Err(StatusCode::UPGRADE_REQUIRED)
        );
        assert!(!principal.can_reorder());
        assert_eq!(
            super::super::classify_route(&Method::GET, "/api/sync/dataset"),
            super::super::RouteAuth::Product
        );
    }
}
