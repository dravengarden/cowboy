//! Real CLI, private HTTP boundary and enrolled Machine using disposable identities.
use super::*;

fn failure(error: Failure) -> anyhow::Error {
    anyhow::anyhow!("isolated local Operator check: {error:?}")
}

async fn cli(pair: &Pair<'_>, args: &[&str]) -> Result<Value> {
    let mut command = command(&pair.controller_artifact.executable, pair.root);
    command
        .arg("operator")
        .arg("--data-dir")
        .arg(pair.root.join("controller"))
        .args(args)
        .stdout(Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(110), command.output()).await??;
    ensure!(
        output.stdout.len() <= 128 * 1024,
        "oversized CLI fixture result"
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    // Failed HTTP outcomes must also produce a failed CLI exit status.
    if let Some(status) = value["http_status"].as_u64() {
        ensure!(
            output.status.success() == (200..300).contains(&status),
            "CLI exit status mismatch"
        );
    } else {
        ensure!(output.status.success(), "local policy command failed");
    }
    Ok(value)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "just plugin-local-operator-conformance; immutable releases and isolated network required"]
async fn immutable_local_operator_installation() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    manifest::require_isolation()?;
    let revision = manifest::clean_revision()?;
    let matrix: Matrix = serde_json::from_slice(&std::fs::read(std::env::var(
        "COWBOY_TEST_LOCAL_OPERATOR_MATRIX",
    )?)?)?;
    let receipt = PathBuf::from(std::env::var("COWBOY_TEST_LOCAL_OPERATOR_RECEIPT")?);
    ensure!(
        receipt.is_absolute() && !receipt.exists(),
        "new absolute receipt required"
    );
    let artifacts = matrix.resolve()?;
    let controller = artifacts
        .iter()
        .find(|a| a.lane == Lane::Controller && a.role == Role::Active)
        .unwrap();
    let machine = artifacts
        .iter()
        .find(|a| a.lane == Lane::Machine && a.role == Role::Active)
        .unwrap();
    let helper = manifest::ssh_keygen()?;
    let root = tempfile::tempdir()?;
    let fixture = InstallFixture::seed(root.path(), &helper.path).await?;
    let mut pair = Pair::prepare(
        root.path(),
        &fixture,
        controller,
        machine,
        Flow::InstallAndReinstall,
        None,
    )
    .await
    .map_err(failure)?;
    let result: Result<()> = async {
        pair.start_controller().await.map_err(failure)?;
        pair.start_machine().map_err(failure)?;
        pair.connected(1).await.map_err(failure)?;
        ensure!(
            cli(&pair, &["status"]).await?["http_status"] == 403,
            "delegation must default closed"
        );
        cli(&pair, &["enable"]).await?;
        let status = cli(&pair, &["status"]).await?;
        ensure!(
            status["http_status"] == 200
                && status["data"]["actor"]["account"]
                    == format!("unix-uid:{}", crate::local_operator::uid()),
            "kernel host identity required"
        );
        // The public router cannot receive host authority through request headers.
        let public = reqwest::Client::builder().no_proxy().build()?;
        let response = public
            .post(format!("http://{}{}", pair.address, endpoint()))
            .header("x-cowboy-operator", "root")
            .json(&pair.request(FIRST))
            .send()
            .await?;
        ensure!(
            matches!(response.status().as_u16(), 401 | 403),
            "public host-header bypass"
        );
        let response = public
            .get(format!("http://{}/v1/status", pair.address))
            .send()
            .await?;
        ensure!(
            response.status() != reqwest::StatusCode::OK,
            "private router leaked onto TCP"
        );
        let install = [
            "upgrade",
            "--machine",
            MACHINE,
            "--plugin",
            "victoria",
            "--version",
            &fixture.desired.release.plugin_version,
            "--digest",
            &fixture.desired.release.artifact_digest,
            "--operation-id",
            FIRST,
        ];
        ensure!(
            cli(&pair, &install).await?["http_status"] == 204,
            "local installation failed"
        );
        let counts = pair.proxy.counts().map_err(failure)?;
        ensure!(
            counts.steps_forwarded == 1 && counts.receipts_forwarded == 1,
            "one actual Machine effect required"
        );
        let duplicate = cli(&pair, &install).await?;
        ensure!(
            duplicate["http_status"] == 409
                && duplicate["data"]["operation"]["phase"] == "completed",
            "saved identity must return original evidence"
        );
        cli(&pair, &["disable"]).await?;
        ensure!(
            cli(&pair, &install).await?["http_status"] == 403,
            "revoked grant accepted"
        );
        ensure!(
            pair.proxy.counts().map_err(failure)? == counts,
            "duplicate or denied request contacted Machine"
        );
        pair.stop_processes().await.map_err(failure)?;
        let before = Evidence::read(root.path()).map_err(failure)?;
        let operations = before.operations().map_err(failure)?;
        ensure!(
            operations.len() == 1
                && operations[0].intent.actor
                    == crate::plugin_operation::Actor::Admin {
                        account: format!("unix-uid:{}", crate::local_operator::uid())
                    },
            "durable host actor missing"
        );
        pair.start_controller().await.map_err(failure)?;
        cli(&pair, &["enable"]).await?;
        ensure!(
            cli(&pair, &install).await?["http_status"] == 409,
            "restart replayed saved identity"
        );
        pair.stop_processes().await.map_err(failure)?;
        ensure!(
            Evidence::read(root.path()).map_err(failure)? == before,
            "restart changed durable installation evidence"
        );
        Ok(())
    }
    .await;
    let cleanup = pair.finish().await.map_err(failure);
    let wire = pair.proxy.snapshot();
    write_receipt(
        &receipt,
        &json!({
            "schema":1, "purpose":"isolated_local_operator_installation", "source_revision":revision,
            "controller":controller, "machine":machine, "wire":wire,
            "protocol":pair.proxy.protocol(),
            "accepted":result.is_ok() && cleanup.is_ok(),
            "checks":["default_closed", "kernel_uid", "public_router_separation", "exact_signed_install", "one_machine_effect", "saved_id_without_replay", "revocation", "durable_host_actor", "restart_without_replay"],
            "not_checked":["production_authority_and_installation", "provider_credentials_and_sessions", "distributed_power_loss"]
        }),
    )?;
    result.and(cleanup)
}
