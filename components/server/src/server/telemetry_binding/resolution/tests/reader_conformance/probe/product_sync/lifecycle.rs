//! Populate only disposable pre-effect evidence; exercise exact immutable HTTP
//! readers without dispatching a Plugin effect or starting any Machine/worker.
use super::*;
use crate::plugin_operation::resolution::{ResolutionIntent, ResolutionPermit};
use crate::plugin_operation::{Actor, Phase, Problem};
use anyhow::Context as _;

const OPERATION: &str = "operation-history-fixture";

pub(super) async fn seed(root: &Path) -> Result<()> {
    let store =
        crate::store::Store::connect(&database(root), root.join("controller/artifacts")).await?;
    let mut uninstall = crate::plugin_operation::fixture("history");
    uninstall.operation_id = OPERATION.into();
    uninstall.service_id = SERVICE.into();
    uninstall.machine_id = MACHINE.into();
    uninstall.actor = Actor::Product {
        user_id: "c".repeat(32),
    };
    store.begin_plugin_uninstall(&uninstall).await?;
    store
        .advance_plugin_uninstall(
            OPERATION,
            Phase::Prepared,
            Phase::NeedsAttention,
            Some(Problem::Interrupted),
        )
        .await?;
    let interrupted = store
        .plugin_uninstall_operation(OPERATION)
        .await?
        .context("fixture operation missing")?;
    let resolution = ResolutionIntent::new(
        "resolution-history-fixture".into(),
        uninstall.actor.clone(),
        &interrupted,
        uninstall.expires_at_ms,
    )?;
    store
        .resolve_plugin_uninstall(&ResolutionPermit::for_test(resolution))
        .await?;
    let mut install = crate::plugin_operation::installation::machine_fixture("history");
    install.operation_id = OPERATION.into();
    install.request_id = format!("plugin-install-{OPERATION}");
    install.service_id = SERVICE.into();
    install.machine_id = MACHINE.into();
    install.actor = uninstall.actor;
    store.begin_plugin_install(&install).await?;
    Ok(())
}

async fn fingerprint(root: &Path) -> Result<String, Failure> {
    let result = async {
        let store =
            crate::store::Store::connect(&database(root), root.join("controller/artifacts"))
                .await?;
        let installation = store.plugin_install_operation(OPERATION).await?;
        let uninstall = store.plugin_uninstall_operation(OPERATION).await?;
        let resolution = store.plugin_uninstall_resolution(OPERATION).await?;
        Ok::<_, anyhow::Error>(sha256(&serde_json::to_vec(&(
            installation,
            uninstall,
            resolution,
        ))?))
    }
    .await;
    result.map_err(|_| Failure::EvidenceChanged)
}

pub(super) async fn exercise(
    address: std::net::SocketAddr,
    password: &str,
    root: &Path,
) -> Result<(), Failure> {
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(DEADLINE)
        .build()
        .map_err(|_| Failure::Setup)?;
    let base = format!("http://{address}");
    let (operator, _) = login(&client, &base, "dataset-operator", password).await?;
    let (viewer, _) = login(&client, &base, "dataset-viewer", password).await?;
    let path = format!("/api/machines/{MACHINE}/plugins/victoria/lifecycle-history");
    let before = fingerprint(root).await?;
    for (cookie, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(viewer.as_str()), StatusCode::FORBIDDEN),
    ] {
        check(get(&client, &base, &path, cookie).await?.status() == status)?;
    }
    let mut previous = None;
    for _ in 0..3 {
        let response = get(&client, &base, &path, Some(&operator)).await?;
        check(
            response.status() == StatusCode::OK
                && response
                    .headers()
                    .get(header::CACHE_CONTROL)
                    .is_some_and(|value| value == "no-store"),
        )?;
        let value = body(response).await?;
        check(
            value["schema"] == "dravengarden.cowboy.plugin-lifecycle-history/v1"
                && value["machine_id"] == MACHINE
                && value["plugin_id"] == "victoria"
                && value["execution_authorized"] == false
                && value["observation"] == "independent_durable_reads"
                && value["limit_per_kind"] == 32,
        )?;
        let entries = value["entries"]
            .as_array()
            .ok_or(Failure::WrongObservation)?;
        check(
            entries.len() == 2
                && entries
                    .iter()
                    .all(|entry| entry["operation"]["operation_id"] == OPERATION),
        )?;
        let install = entries
            .iter()
            .find(|entry| entry["kind"] == "install")
            .ok_or(Failure::WrongObservation)?;
        check(
            install["operation"]["phase"] == "needs_attention"
                && install["operation"]["problem"] == "interrupted"
                && install["operation"]["machine_receipt"].is_null(),
        )?;
        let uninstall = entries
            .iter()
            .find(|entry| entry["kind"] == "uninstall")
            .ok_or(Failure::WrongObservation)?;
        check(
            uninstall["operation"]["phase"] == "aborted"
                && uninstall["resolution"]["resolution_id"] == "resolution-history-fixture"
                && uninstall["resolution"]["worker_restoration_performed"] == false,
        )?;
        if let Some(previous) = &previous {
            check(previous == &value)?;
        }
        previous = Some(value);
    }
    check(fingerprint(root).await? == before)
}
