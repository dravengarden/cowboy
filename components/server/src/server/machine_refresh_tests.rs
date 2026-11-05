
use super::*;

#[test]
fn refresh_needs_an_operator_or_a_direct_same_host_caller() {
    let path = "/api/machines/hawk/refresh";
    assert_eq!(
        classify_route(&Method::POST, path),
        RouteAuth::ProductOrAdminOperator
    );
    let local: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let remote: SocketAddr = "192.0.2.1:50000".parse().unwrap();
    let direct = HeaderMap::new();
    assert!(internal_machine_refresh(
        &Method::POST,
        path,
        local,
        &direct
    ));
    assert!(!internal_machine_refresh(
        &Method::POST,
        path,
        remote,
        &direct
    ));
    assert!(!internal_machine_refresh(
        &Method::GET,
        path,
        local,
        &direct
    ));
    let mut proxied = HeaderMap::new();
    proxied.insert("x-forwarded-for", "192.0.2.1".parse().unwrap());
    assert!(!internal_machine_refresh(
        &Method::POST,
        path,
        local,
        &proxied
    ));
}

#[test]
fn a_plugin_named_refresh_is_not_a_machine_refresh() {
    assert!(!machine_refresh_path("/api/machines/hawk/plugins/refresh"));
    assert_eq!(
        classify_route(&Method::POST, "/api/machines/hawk/plugins/refresh"),
        RouteAuth::ProductOrAdminOperator
    );
}
