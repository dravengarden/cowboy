use super::*;

const FAILURES: [&str; 3] = [
    "observability_failed_log_batches",
    "observability_failed_metric_batches",
    "observability_failed_trace_batches",
];
const ZERO: [&str; 4] = [
    "observability_failed_file_batches",
    "observability_dropped_export_batches",
    "observability_dropped_batches",
    "observability_failed_incident_batches",
];

fn lane(signal: Signal) -> usize {
    match signal {
        Signal::Logs => 0,
        Signal::Metrics => 1,
        Signal::Traces => 2,
    }
}

fn number(value: &Value, key: &str) -> Result<u64, Failure> {
    value[key].as_u64().ok_or(Failure::WrongObservation)
}

fn delta(before: &Value, after: &Value, key: &str) -> Result<u64, Failure> {
    number(after, key)?
        .checked_sub(number(before, key)?)
        .ok_or(Failure::WrongObservation)
}

pub(super) async fn run(
    pair: &Pair<'_>,
    step: DeliveryStep,
    report: &mut DeliveryReport,
) -> Result<(), Failure> {
    let mut fixtures = crate::otlp::client_fixtures();
    if step.ack() != Ack::Forward {
        fixtures.retain(|(s, _)| *s == Signal::Logs);
        check(fixtures.len() == 1)?;
    }
    let destination = pair.destination.as_ref().ok_or(Failure::Setup)?;
    destination.mode(step.response());
    pair.proxy.export_ack(step.ack())?;
    let evidence = pair.evidence()?;
    let start = pair.http.get("/api/metrics").await?;
    let before_http = destination.records()?.len();
    let before_wire = pair.proxy.export_snapshot().len();
    let file = pair.root.join("controller/telemetry/telemetry.jsonl");
    let before = std::fs::read(&file).map_err(|_| Failure::LocalRecording)?;
    let serial = report.rounds.len();
    report.rounds.push(DeliveryRound {
        step,
        accepted: false,
        submitted_batches: fixtures.len(),
        elapsed_ms: 0,
        local_bytes_added: 0,
        local_records_added: 0,
        duplicate_batches_added: 0,
        remote_failures_added: [0; 3],
        rejected_items_added: 0,
        export_commands_added: 0,
        http_requests_added: 0,
    });
    let outcome = report.rounds.last_mut().ok_or(Failure::Setup)?;
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(22), async {
        let client = pair.http.recording_client()?;
        for (index, (signal, bytes)) in fixtures.iter().enumerate() {
            let signal = match signal {
                Signal::Logs => "logs",
                Signal::Metrics => "metrics",
                Signal::Traces => "traces",
            };
            let uri = format!(
                "http://{}/api/telemetry/v1/{signal}?batch_id=delivery-{serial}-{index}",
                pair.address
            );
            for _ in 0..2 {
                let response = client
                    .post(&uri)
                    .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
                    .body(bytes.clone())
                    .send()
                    .await
                    .map_err(|_| Failure::LocalRecording)?;
                check(response.status() == reqwest::StatusCode::OK)?;
            }
        }
        let expected: Vec<_> = fixtures
            .iter()
            .filter(|(signal, _)| step.exports(*signal))
            .map(|(s, _)| *s)
            .collect();
        let failures = std::array::from_fn::<_, 3, _>(|index| {
            if step.failed() {
                fixtures.iter().filter(|(s, _)| lane(*s) == index).count() as u64
            } else {
                0
            }
        });
        let rejected = if step == DeliveryStep::Partial {
            fixtures.len() as u64
        } else {
            0
        };
        loop {
            pair.proxy.counts()?;
            let current = pair.http.get("/api/metrics").await?;
            let http = destination.records()?;
            let wire = pair.proxy.export_snapshot();
            check(
                http.len() <= before_http + expected.len()
                    && wire.len() <= before_wire + expected.len(),
            )?;
            for key in ZERO {
                check(number(&current, key)? == 0)?;
            }
            for (index, key) in FAILURES.iter().enumerate() {
                check(delta(&start, &current, key)? <= failures[index])?;
            }
            check(delta(&start, &current, "observability_rejected_export_items")? <= rejected)?;
            let failures_done = FAILURES
                .iter()
                .enumerate()
                .all(|(i, key)| delta(&start, &current, key).ok() == Some(failures[i]));
            if number(&current, "observability_pending")? == 0
                && delta(&start, &current, "observability_accepted_batches")?
                    == fixtures.len() as u64
                && delta(&start, &current, "observability_duplicate_batches")?
                    == fixtures.len() as u64
                && failures_done
                && delta(&start, &current, "observability_rejected_export_items")? == rejected
                && http.len() == before_http + expected.len()
                && wire.len() == before_wire + expected.len()
                && wire[before_wire..].iter().all(|e| e.receipt.is_some())
            {
                for ((entry, received), signal) in wire[before_wire..]
                    .iter()
                    .zip(&http[before_http..])
                    .zip(&expected)
                {
                    check(
                        entry.signal == *signal
                            && received.signal == *signal
                            && entry.items == received.items
                            && entry.payload_sha256 == received.payload_sha256
                            && entry.receipt == Some(step.receipt())
                            && entry.ack == Some(step.ack())
                            && received.response == step.response(),
                    )?;
                }
                let bytes = std::fs::read(&file).map_err(|_| Failure::LocalRecording)?;
                let metadata = file
                    .symlink_metadata()
                    .map_err(|_| Failure::LocalRecording)?;
                check(
                    metadata.is_file()
                        && metadata.permissions().mode() & 0o077 == 0
                        && bytes.starts_with(&before)
                        && bytes.len() > before.len(),
                )?;
                let lines: Vec<_> = bytes[before.len()..]
                    .split(|b| *b == b'\n')
                    .filter(|l| !l.is_empty())
                    .collect();
                check(
                    lines.len() >= fixtures.len()
                        && lines
                            .iter()
                            .all(|l| serde_json::from_slice::<Value>(l).is_ok()),
                )?;
                outcome.local_bytes_added = (bytes.len() - before.len()) as u64;
                outcome.local_records_added = lines.len();
                outcome.duplicate_batches_added = fixtures.len() as u64;
                outcome.remote_failures_added = failures;
                outcome.rejected_items_added = rejected;
                outcome.export_commands_added = expected.len();
                outcome.http_requests_added = expected.len();
                // Give the old queue/transport an opportunity to expose a retry.
                // Every later phase and restart rechecks the cumulative one-to-one evidence too.
                tokio::time::sleep(Duration::from_millis(200)).await;
                check(destination.records()? == http && pair.proxy.export_snapshot() == wire)?;
                evidence.matches(pair.root)?;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    outcome.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    result?;
    if step == DeliveryStep::LostAck {
        check(outcome.elapsed_ms >= 14_000)?;
    }
    outcome.accepted = true;
    Ok(())
}
