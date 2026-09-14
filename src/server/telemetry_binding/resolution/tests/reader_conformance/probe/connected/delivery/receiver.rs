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
    requests: Vec<(Signal, Vec<u8>)>,
    forwarding: bool,
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
        Self::start_with(None).await
    }

    pub async fn forwarding(addresses: [std::net::SocketAddr; 3]) -> Result<Self, Failure> {
        check(
            addresses
                .iter()
                .all(|address| address.ip().is_loopback() && address.port() != 0),
        )?;
        Self::start_with(Some(addresses)).await
    }

    async fn start_with(forward: Option<[std::net::SocketAddr; 3]>) -> Result<Self, Failure> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Failure::Setup)?;
        let address = listener.local_addr().map_err(|_| Failure::Setup)?;
        let state = Arc::new(parking_lot::Mutex::new(State {
            mode: ResponseMode::Success,
            received: Vec::new(),
            invalid: false,
            requests: Vec::new(),
            forwarding: false,
        }));
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| Failure::Setup)?;
        let shared = state.clone();
        let app = axum::Router::new()
            .fallback(
                move |method: axum::http::Method,
                      OriginalUri(uri): OriginalUri,
                      headers: HeaderMap,
                      bytes: Bytes| {
                    let shared = shared.clone();
                    let client = client.clone();
                    async move {
                        let mode = {
                            let mut state = shared.lock();
                            if state.invalid
                                || state.received.len() >= 96
                                || state.forwarding
                                || (forward.is_some()
                                    && (state.requests.len() >= 8
                                        || state.mode != ResponseMode::Success))
                            {
                                state.invalid = true;
                                return preserve_response((
                                    StatusCode::TOO_MANY_REQUESTS,
                                    HeaderMap::new(),
                                    Vec::new(),
                                ));
                            }
                            state.forwarding = forward.is_some();
                            state.mode
                        };
                        let result = receive(&method, &uri, &headers, &bytes, mode);
                        let Ok(mut record) = result else {
                            shared.lock().invalid = true;
                            return preserve_response((
                                StatusCode::BAD_REQUEST,
                                HeaderMap::new(),
                                Vec::new(),
                            ));
                        };
                        let response = if let Some(addresses) = forward {
                            let index = match record.signal {
                                Signal::Logs => 0,
                                Signal::Metrics => 1,
                                Signal::Traces => 2,
                            };
                            match forward_request(&client, addresses[index], uri.path(), &bytes)
                                .await
                            {
                                Ok(response) => response,
                                Err(_) => {
                                    shared.lock().invalid = true;
                                    return preserve_response((
                                        StatusCode::BAD_GATEWAY,
                                        HeaderMap::new(),
                                        Vec::new(),
                                    ));
                                }
                            }
                        } else {
                            response(mode, record.signal)
                        };
                        if forward.is_some() {
                            record.database = Some(DatabaseHttp {
                                status: response.0.as_u16(),
                                content_type: match response.1.get("content-type") {
                                    None => DatabaseMediaType::Absent,
                                    Some(value) if value == "application/x-protobuf" => {
                                        DatabaseMediaType::Protobuf
                                    }
                                    Some(_) => DatabaseMediaType::Other,
                                },
                                body_bytes: response.2.len(),
                                body_sha256: sha256(&response.2),
                            });
                        }
                        let mut state = shared.lock();
                        state.forwarding = false;
                        if forward.is_some() {
                            state.requests.push((record.signal, bytes.to_vec()));
                        }
                        state.received.push(record);
                        preserve_response(response)
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

    // Only disposable fixture payloads, bounded in memory; never serialized in a receipt.
    pub fn requests(&self) -> Result<Vec<(Signal, Vec<u8>)>, Failure> {
        let state = self.state.lock();
        check(!state.invalid)?;
        Ok(state.requests.clone())
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

fn preserve_response(
    (status, headers, bytes): (StatusCode, HeaderMap, Vec<u8>),
) -> axum::response::Response {
    // Vec<u8>'s IntoResponse adds application/octet-stream when a real empty
    // OTLP success omitted Content-Type. A transparent relay must not add it.
    let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

async fn forward_request(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    route: &str,
    bytes: &[u8],
) -> Result<(StatusCode, HeaderMap, Vec<u8>), Failure> {
    // The fixture's bearer token is validated at the relay, never forwarded.
    // Preserve the actual database status, content type and response bytes.
    let mut response = client
        .post(format!("http://{address}{route}"))
        .header("content-type", "application/x-protobuf")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|_| Failure::ConnectionClosed)?;
    let status = response.status();
    let mut headers = HeaderMap::new();
    if let Some(value) = response.headers().get("content-type") {
        headers.insert("content-type", value.clone());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::FrameDecode)? {
        check(body.len() + chunk.len() <= 64 * 1024)?;
        body.extend_from_slice(&chunk);
    }
    Ok((status, headers, body))
}

#[tokio::test]
async fn database_relay_preserves_response_bytes_without_credentials_redirects_or_retries() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let observations = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let captured = observations.clone();
    let app = axum::Router::new().fallback(
        move |OriginalUri(uri): OriginalUri, headers: HeaderMap, bytes: Bytes| {
            captured.lock().push((
                uri.path().to_owned(),
                headers.contains_key("authorization"),
                bytes.to_vec(),
            ));
            async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [
                        ("content-type", "application/x-protobuf"),
                        ("location", "/must-not-follow"),
                    ],
                    vec![1_u8, 2, 3],
                )
            }
        },
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let result = forward_request(
        &client,
        address,
        "/insert/opentelemetry/v1/logs",
        b"fixture",
    )
    .await
    .unwrap();
    assert_eq!(result.0, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(result.1["content-type"], "application/x-protobuf");
    assert_eq!(result.2, [1, 2, 3]);
    assert_eq!(
        *observations.lock(),
        vec![(
            "/insert/opentelemetry/v1/logs".to_owned(),
            false,
            b"fixture".to_vec()
        )]
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        Destination::forwarding(["192.0.2.1:1234".parse().unwrap(); 3])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn database_relay_does_not_invent_content_type_on_an_empty_upstream_response() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .fallback(|| async { axum::response::Response::new(axum::body::Body::empty()) });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut destination = Destination::forwarding([address; 3]).await.unwrap();
    let bytes = crate::otlp::client_fixtures()
        .into_iter()
        .find(|(signal, _)| *signal == Signal::Logs)
        .unwrap()
        .1;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let response = client
        .post(format!(
            "http://{}/insert/opentelemetry/v1/logs",
            destination.address
        ))
        .header("content-type", "application/x-protobuf")
        .header("authorization", "Bearer isolated-fixture-only")
        .body(bytes)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let content_type = response.headers().get("content-type").cloned();
    let body = response.bytes().await.unwrap();
    drop(client);
    destination.finish().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    assert_eq!(content_type, None);
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
        database: None,
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
