use super::*;
use reqwest::{Method, StatusCode};

pub(super) struct Prepared {
    pub root: tempfile::TempDir,
    pub fixture: InstallFixture,
    pub http: Http,
    pub evidence: Evidence,
}

pub(super) async fn prepare(
    artifacts: &[Artifact],
    flow: Flow,
    helper: &Path,
    stage: &mut Stage,
    report: &mut WriterReport,
) -> Result<Prepared, Failure> {
    let controller = artifacts
        .iter()
        .find(|a| a.lane == Lane::Controller && a.role == Role::Active)
        .ok_or(Failure::Setup)?;
    let machine = artifacts
        .iter()
        .find(|a| a.lane == Lane::Machine && a.role == Role::Active)
        .ok_or(Failure::Setup)?;
    let root = tempfile::tempdir().map_err(|_| Failure::Setup)?;
    let fixture = InstallFixture::seed(root.path(), helper)
        .await
        .map_err(|_| Failure::Setup)?;
    report.package_sha256.clone_from(&fixture.package_sha256);
    report.release_sha256.clone_from(&fixture.release_sha256);
    let mut pair = Pair::prepare(root.path(), &fixture, controller, machine, flow, None).await?;
    let started = std::time::Instant::now();
    let result = async {
        *stage = Stage::Authentication;
        pair.start(true).await?;
        let history = pair
            .http
            .get(&format!("{}/installation-operations", endpoint()))
            .await?;
        check(history["admission_enabled"] == true && history["operations"] == json!([]))?;
        *stage = Stage::Installation;
        exercise(&mut pair, flow).await
    }
    .await;
    report.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    report.last_http = pair.http.last();
    let cleaned = pair.finish().await;
    report.wire = pair.proxy.snapshot();
    result.and(cleaned)?;
    *stage = Stage::WriterEvidence;
    verify_counts(&pair.proxy.counts()?, flow)?;
    let http = pair.http.at(pair.address)?;
    drop(pair);
    let evidence = Evidence::read(root.path())?;
    evidence.verify(&fixture, flow, false)?;
    let (service, machine) = evidence.hashes()?;
    report.service_sha256 = Some(service);
    report.machine_sha256 = Some(machine);
    Ok(Prepared {
        root,
        fixture,
        http,
        evidence,
    })
}

async fn exercise(pair: &mut Pair<'_>, flow: Flow) -> Result<(), Failure> {
    let body = pair.request(FIRST);
    let path = endpoint();
    if flow == Flow::ControllerCrashAfterApplied {
        let mut controller = pair.controller.take().ok_or(Failure::Setup)?;
        let (reply, killed) =
            tokio::join!(pair.http.call(Method::POST, &path, Some(body)), async {
                pair.proxy.applied().await?;
                controller.finish().await
            });
        killed?;
        return check(
            reply.is_err()
                && pair.http.last().is_some_and(|last| {
                    matches!(
                        last.result,
                        super::super::super::connected::HttpResult::Transport
                    )
                }),
        );
    }
    let started = std::time::Instant::now();
    let reply = pair.http.call(Method::POST, &path, Some(body)).await?;
    if flow == Flow::InstallAndReinstall {
        check(reply.status == StatusCode::NO_CONTENT)?;
        let reply = pair
            .http
            .call(Method::POST, &path, Some(pair.request(SECOND)))
            .await?;
        check(reply.status == StatusCode::NO_CONTENT)?;
    } else {
        check(reply.status == StatusCode::CONFLICT)?;
    }
    if flow == Flow::LostReceipt {
        // Exercise the real 90-second Machine command deadline, not a mocked
        // timer or a replacement grant. There is no receipt-query fallback.
        check(started.elapsed() >= Duration::from_secs(90))?;
    }
    if matches!(
        flow,
        Flow::DisconnectAfterApplied | Flow::DisconnectBeforeDelivery
    ) {
        pair.connected(2).await?;
    }
    Ok(())
}

fn verify_counts(counts: &WireCounts, flow: Flow) -> Result<(), Failure> {
    let attempts = u32::try_from(flow.attempts()).map_err(|_| Failure::Setup)?;
    let delivered = if flow == Flow::DisconnectBeforeDelivery {
        0
    } else {
        attempts
    };
    let forwarded = if flow == Flow::InstallAndReinstall {
        attempts
    } else {
        0
    };
    let disconnected = u32::from(matches!(
        flow,
        Flow::DisconnectAfterApplied | Flow::DisconnectBeforeDelivery
    ));
    check(
        counts.connections > disconnected
            && counts.runtime_configurations == counts.connections
            && counts.target_queries == attempts
            && counts.target_receipts == attempts
            && counts.steps_observed == attempts
            && counts.steps_forwarded == delivered
            && counts.receipts_observed == delivered
            && counts.receipts_forwarded == forwarded
            && counts.dropped_receipts == delivered - forwarded
            && counts.forced_disconnects == disconnected,
    )
}
