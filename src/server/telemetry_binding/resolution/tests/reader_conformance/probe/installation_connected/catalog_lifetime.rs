//! ABA at actual HTTP admission awaits, without altering any production state.
use super::*;
use reqwest::{Method, StatusCode};

async fn refresh(admin: &Http, count: u64) -> Result<(), Failure> {
    let reply = admin
        .call(Method::POST, "/api/plugins/catalog/refresh", None)
        .await?
        .ok()?;
    check(reply["external_releases"] == count)
}

async fn interrupt(
    pair: &Pair<'_>,
    kind: proxy::CatalogProbeKind,
    path: &str,
    body: Value,
) -> Result<(), Failure> {
    pair.http
        .denied(Method::POST, "/api/plugins/catalog/refresh", None)
        .await?;
    let admin = Http::catalog_admin(pair.address, &pair.fixture.catalog_password).await?;
    let gate = pair.proxy.hold_catalog_probe(kind)?;
    let marker = pair.root.join("catalog/victoria.release.json");
    let original = std::fs::read(&marker).map_err(|_| Failure::Setup)?;
    let (reply, changed) = tokio::join!(pair.http.call(Method::POST, path, Some(body)), async {
        let result = async {
            gate.held().await?;
            std::fs::remove_file(&marker).map_err(|_| Failure::Setup)?;
            refresh(&admin, 0).await?;
            std::fs::write(&marker, &original).map_err(|_| Failure::Setup)?;
            refresh(&admin, 1).await
        }
        .await;
        gate.release();
        result
    });
    changed?;
    check(reply?.status == StatusCode::CONFLICT)?;
    pair.proxy.finish_catalog_probe()?;
    Ok(())
}

pub(super) async fn install(pair: &Pair<'_>) -> Result<(), Failure> {
    interrupt(
        pair,
        proxy::CatalogProbeKind::Install,
        &endpoint(),
        pair.request("catalog-ended-install"),
    )
    .await?;
    let history = pair
        .http
        .get(&format!("{}/installation-operations", endpoint()))
        .await?;
    check(history["operations"] == json!([]))
}

pub(super) async fn uninstall(pair: &Pair<'_>) -> Result<(), Failure> {
    let preview = pair
        .http
        .call(
            Method::POST,
            &format!("{}/uninstall-plan", endpoint()),
            None,
        )
        .await?
        .ok()?;
    check(preview["affected_sessions"] == json!([]) && preview["plan_id"].is_string())?;
    interrupt(
        pair,
        proxy::CatalogProbeKind::Uninstall,
        &format!("{}/uninstall", endpoint()),
        json!({"plan_id": preview["plan_id"]}),
    )
    .await?;
    let history = pair.http.get(&format!("{}/operations", endpoint())).await?;
    check(history["operations"] == json!([]))
}
