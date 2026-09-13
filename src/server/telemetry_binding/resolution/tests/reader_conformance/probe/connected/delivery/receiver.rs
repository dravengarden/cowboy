//! Isolated protocol fixture, not a Victoria database or a production endpoint.
use super::*;
use crate::otlp::{Request, Signal};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, OriginalUri},
    http::{HeaderMap, StatusCode},
};
use prost::Message as _;

struct State {
    mode: ResponseMode,
    received: Vec<HttpExport>,
    invalid: bool,
}

pub(in super::super) struct Destination {
    pub address: std::net::SocketAddr,
    state: Arc<parking_lot::Mutex<State>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Drop for Destination {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Destination {
    pub async fn start() -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Failure::Setup)?;
        let address = listener.local_addr().map_err(|_| Failure::Setup)?;
        let state = Arc::new(parking_lot::Mutex::new(State {
            mode: ResponseMode::Success,
            received: Vec::new(),
            invalid: false,
        }));
        let shared = state.clone();
        let app = axum::Router::new()
            .fallback(
                move |method: axum::http::Method,
                      OriginalUri(uri): OriginalUri,
                      headers: HeaderMap,
                      bytes: Bytes| {
                    let shared = shared.clone();
                    async move {
                        let mut state = shared.lock();
                        let result = receive(&method, &uri, &headers, &bytes, state.mode);
                        let Ok(record) = result else {
                            state.invalid = true;
                            return (StatusCode::BAD_REQUEST, HeaderMap::new(), Vec::new());
                        };
                        if state.received.len() == 96 {
                            state.invalid = true;
                            return (StatusCode::TOO_MANY_REQUESTS, HeaderMap::new(), Vec::new());
                        }
                        let response = response(state.mode, record.signal);
                        state.received.push(record);
                        response
                    }
                },
            )
            .layer(DefaultBodyLimit::max(crate::otlp::MAX_BYTES));
        let (shutdown, stop) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stop.await;
                })
                .await
        });
        Ok(Self {
            address,
            state,
            shutdown: Some(shutdown),
            task,
        })
    }

    pub fn mode(&self, mode: ResponseMode) {
        self.state.lock().mode = mode;
    }

    pub fn records(&self) -> Result<Vec<HttpExport>, Failure> {
        let state = self.state.lock();
        check(!state.invalid)?;
        Ok(state.received.clone())
    }

    pub async fn finish(&mut self) -> Result<(), Failure> {
        if let Some(stop) = self.shutdown.take() {
            let _ = stop.send(());
        }
        match tokio::time::timeout(Duration::from_secs(3), &mut self.task).await {
            Ok(Ok(Ok(()))) => Ok(()),
            _ => Err(Failure::Cleanup),
        }
    }
}

fn receive(
    method: &axum::http::Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
    bytes: &[u8],
    mode: ResponseMode,
) -> Result<HttpExport, Failure> {
    check(
        method == axum::http::Method::POST
            && uri.query().is_none()
            && headers.get("content-type").and_then(|h| h.to_str().ok())
                == Some("application/x-protobuf")
            && headers.get("authorization").and_then(|h| h.to_str().ok())
                == Some("Bearer isolated-fixture-only")
            && !headers.contains_key("content-encoding"),
    )?;
    let signal = match uri.path() {
        "/insert/opentelemetry/v1/logs" => Signal::Logs,
        "/opentelemetry/v1/metrics" => Signal::Metrics,
        "/insert/opentelemetry/v1/traces" => Signal::Traces,
        _ => return Err(Failure::WrongObservation),
    };
    let request = Request::decode(signal, bytes).map_err(|_| Failure::FrameDecode)?;
    check(request.count() > 0)?;
    if let Request::Metrics(metrics) = &request {
        check(
            metrics
                .resource_metrics
                .iter()
                .flat_map(|r| &r.scope_metrics)
                .flat_map(|s| &s.metrics)
                .all(|m| {
                    use opentelemetry_proto::tonic::metrics::v1::metric::Data;
                    match &m.data {
                        Some(Data::Sum(s)) => s.aggregation_temporality == 2 && s.is_monotonic,
                        Some(Data::Histogram(h)) => h.aggregation_temporality == 2,
                        _ => false,
                    }
                }),
        )?;
    }
    Ok(HttpExport {
        signal,
        payload_sha256: sha256(bytes),
        items: request.count(),
        response: mode,
    })
}

fn response(mode: ResponseMode, signal: Signal) -> (StatusCode, HeaderMap, Vec<u8>) {
    let status = match mode {
        ResponseMode::Success | ResponseMode::Partial => StatusCode::OK,
        ResponseMode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        ResponseMode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        ResponseMode::Redirect => StatusCode::TEMPORARY_REDIRECT,
    };
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/x-protobuf".parse().unwrap());
    headers.insert("location", "/must-not-follow".parse().unwrap());
    let body = if mode == ResponseMode::Partial {
        partial(signal)
    } else {
        Vec::new()
    };
    (status, headers, body)
}

fn partial(signal: Signal) -> Vec<u8> {
    use opentelemetry_proto::tonic::collector::{logs::v1::*, metrics::v1::*, trace::v1::*};
    match signal {
        Signal::Logs => ExportLogsServiceResponse {
            partial_success: Some(ExportLogsPartialSuccess {
                rejected_log_records: 1,
                error_message: String::new(),
            }),
        }
        .encode_to_vec(),
        Signal::Metrics => ExportMetricsServiceResponse {
            partial_success: Some(ExportMetricsPartialSuccess {
                rejected_data_points: 1,
                error_message: String::new(),
            }),
        }
        .encode_to_vec(),
        Signal::Traces => ExportTraceServiceResponse {
            partial_success: Some(ExportTracePartialSuccess {
                rejected_spans: 1,
                error_message: String::new(),
            }),
        }
        .encode_to_vec(),
    }
}

#[test]
fn receiver_returns_standard_partial_success_and_never_records_credentials_or_payload() {
    for signal in [Signal::Logs, Signal::Metrics, Signal::Traces] {
        assert_eq!(signal.rejected(&partial(signal), 2).unwrap(), 1);
    }
    let (signal, bytes) = crate::otlp::client_fixtures()
        .into_iter()
        .find(|(signal, _)| *signal == Signal::Logs)
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/x-protobuf".parse().unwrap());
    let uri = "/insert/opentelemetry/v1/logs".parse().unwrap();
    assert_eq!(signal, Signal::Logs);
    assert!(
        receive(
            &axum::http::Method::POST,
            &uri,
            &headers,
            &bytes,
            ResponseMode::Success
        )
        .is_err()
    );
    headers.insert(
        "authorization",
        "Bearer isolated-fixture-only".parse().unwrap(),
    );
    let record = receive(
        &axum::http::Method::POST,
        &uri,
        &headers,
        &bytes,
        ResponseMode::Success,
    )
    .unwrap();
    let json = serde_json::to_string(&record).unwrap();
    assert!(
        !json.contains("fixture-only") && !json.contains("protobuf") && !json.contains("http://")
    );
    assert!(
        receive(
            &axum::http::Method::POST,
            &"/must-not-follow".parse().unwrap(),
            &headers,
            &bytes,
            ResponseMode::Success
        )
        .is_err()
    );
}
