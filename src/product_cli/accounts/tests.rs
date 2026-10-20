use super::*;
use clap::Parser;

const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn usage_cli_distinguishes_cached_read_and_explicit_refresh() {
    for (tail, refresh) in [vec![], vec!["--refresh"]].into_iter().zip([false, true]) {
        let mut args = vec!["cowboy", "account", "usage", ACCOUNT];
        args.extend(tail);
        let parsed = crate::cli::Cli::try_parse_from(args);
        assert!(parsed.is_ok(), "usage refresh={refresh}");
    }
}

#[tokio::test]
async fn discovery_cli_uses_existing_owner_login_without_dispatch_or_automatic_retry() {
    use axum::{
        Json, Router,
        extract::{OriginalUri, State},
        routing::get,
    };
    use std::sync::{Arc, Mutex};
    let _ = rustls::crypto::ring::default_provider().install_default();
    type Calls = Arc<Mutex<Vec<String>>>;
    let calls: Calls = Default::default();
    let route = format!("/api/provider-accounts/{ACCOUNT}/logins");
    let app = Router::new()
        .route(
            &route,
            get(
                |State(calls): State<Calls>, OriginalUri(uri): OriginalUri| async move {
                    calls.lock().unwrap().push(format!("GET {uri}"));
                    Json(json!({"logins":[]}))
                },
            ),
        )
        .route(
            &format!("{route}/original"),
            get(|State(calls): State<Calls>| async move {
                calls.lock().unwrap().push("status".into());
                Json(json!({"status":"active"}))
            })
            .delete(|State(calls): State<Calls>| async move {
                calls.lock().unwrap().push("cancel".into());
                (StatusCode::SERVICE_UNAVAILABLE, "unknown")
            }),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::fixture(
        Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap(),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    execute(
        &client,
        AccountCommand::Logins {
            account: parse_account(ACCOUNT).unwrap(),
            after_machine: Some("a&b".into()),
            limit: 2,
        },
        std::io::empty(),
    )
    .await
    .unwrap();
    execute(
        &client,
        AccountCommand::LoginStatus {
            account: parse_account(ACCOUNT).unwrap(),
            request: "original".into(),
        },
        std::io::empty(),
    )
    .await
    .unwrap();
    assert!(
        execute(
            &client,
            AccountCommand::CancelRequest {
                account: parse_account(ACCOUNT).unwrap(),
                request: "original".into()
            },
            std::io::empty()
        )
        .await
        .is_err()
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            format!("GET {route}?limit=2&after_machine=a%26b"),
            "status".into(),
            "cancel".into()
        ]
    );
    server.abort();
}

#[tokio::test]
async fn revocation_cli_keeps_owner_routes_and_does_not_retry_unknown_writes() {
    use axum::{Json, Router, extract::State, routing::get};
    use std::sync::{Arc, Mutex};
    let _ = rustls::crypto::ring::default_provider().install_default();
    type Calls = Arc<Mutex<Vec<(String, Value)>>>;
    let calls: Calls = Default::default();
    let route = format!("/api/provider-accounts/{ACCOUNT}/revocation");
    let app = Router::new()
        .route(
            &route,
            get(|State(calls): State<Calls>| async move {
                calls.lock().unwrap().push(("status".into(), Value::Null));
                Json(json!({"status":"revoking"}))
            })
            .post(
                |State(calls): State<Calls>, Json(body): Json<Value>| async move {
                    calls.lock().unwrap().push(("revoke".into(), body));
                    (StatusCode::SERVICE_UNAVAILABLE, "outcome unknown")
                },
            ),
        )
        .route(
            &format!("{route}/reconcile"),
            axum::routing::post(
                |State(calls): State<Calls>, Json(body): Json<Value>| async move {
                    calls.lock().unwrap().push(("resume".into(), body));
                    Json(json!({"status":"revoking"}))
                },
            ),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::fixture(
        Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap(),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert!(
        execute(
            &client,
            AccountCommand::Revoke {
                account: parse_account(ACCOUNT).unwrap(),
                operation: "revoke-original".into(),
                dataset: "dataset".into(),
                expected_revision: 7,
            },
            std::io::empty()
        )
        .await
        .is_err()
    );
    execute(
        &client,
        AccountCommand::Revocation {
            account: parse_account(ACCOUNT).unwrap(),
        },
        std::io::empty(),
    )
    .await
    .unwrap();
    execute(
        &client,
        AccountCommand::Reconcile {
            account: parse_account(ACCOUNT).unwrap(),
            dataset: "dataset".into(),
        },
        std::io::empty(),
    )
    .await
    .unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            (
                "revoke".into(),
                json!({"operation":"revoke-original","dataset":"dataset","expected_revision":7})
            ),
            ("status".into(), Value::Null),
            ("resume".into(), json!({"dataset":"dataset"})),
        ]
    );
    server.abort();
}

#[test]
fn metadata_input_cannot_set_identity_or_accept_credentials() {
    assert!(
        read_change(
            br#"{"type":"create","provider_id":"claude-code","label":"Personal"}"#.as_slice()
        )
        .is_ok()
    );
    for value in [
        json!({"type":"bind_identity","id":ACCOUNT,"provider_id":"claude-code","identity":{"authority":"native","subject":"private-value","tenant":null}}),
        json!({"type":"create","provider_id":"claude-code","label":"Personal","token":"private-value"}),
    ] {
        let error = read_change(value.to_string().as_bytes())
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-value"));
    }
    assert!(read_change(vec![b' '; 16 * 1024 + 1].as_slice()).is_err());
}

#[derive(Parser)]
struct AccountParser {
    #[command(flatten)]
    args: AccountArgs,
}

#[test]
fn login_requires_exact_release_and_codes_never_take_a_positional_argument() {
    assert!(
        AccountParser::try_parse_from(["account", "login", ACCOUNT, "--machine", "machine-a"])
            .is_err()
    );
    assert!(
        AccountParser::try_parse_from([
            "account",
            "submit-code",
            "claude-code",
            "login-1",
            "private-code"
        ])
        .is_err()
    );
    for part in [".", "..", "provider/other", "request?query", "%2f", ""] {
        assert!(request_path(part, "login-1").is_err());
        assert!(request_path("claude-code", part).is_err());
    }
    assert!(login_path(&ProviderAccountId::default()).is_err());
}

#[test]
fn native_code_input_is_bounded_without_echoing_rejected_values() {
    assert_eq!(read_code(" code#state\n".as_bytes()).unwrap(), "code#state");
    for input in [
        String::new(),
        "secret\nsecond-line".into(),
        "secret\0".into(),
        "s".repeat(4097),
    ] {
        let error = read_code(input.as_bytes()).unwrap_err().to_string();
        assert!(!error.contains("secret"));
    }
}

#[test]
fn session_subscription_is_independent_of_both_machines() {
    #[derive(Parser)]
    struct SessionParser {
        #[command(flatten)]
        args: SessionArgs,
    }
    let parsed = SessionParser::try_parse_from([
        "session",
        "new",
        "claude-code",
        "--machine",
        "workspace",
        "--runtime-machine",
        "runtime",
        "--subscription",
        ACCOUNT,
    ])
    .unwrap();
    let SessionCommand::New {
        machine,
        runtime_machine,
        subscription,
        ..
    } = parsed.args.command
    else {
        panic!("expected new session")
    };
    assert_eq!(machine.as_deref(), Some("workspace"));
    assert_eq!(runtime_machine.as_deref(), Some("runtime"));
    assert_eq!(subscription.unwrap().as_str(), ACCOUNT);
    assert!(
        SessionParser::try_parse_from(["session", "new", "--subscription", "friendly-label"])
            .is_err()
    );
}

#[tokio::test]
async fn native_login_cli_uses_scoped_routes_and_never_retries_service_failure() {
    use axum::{
        Json, Router,
        extract::{Path, Query, State},
        routing::get,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    let _ = rustls::crypto::ring::default_provider().install_default();
    type Calls = Arc<Mutex<Vec<(String, Value)>>>;
    let calls: Calls = Default::default();
    let app =
        Router::new()
            .route(
                "/api/provider-accounts/login-operations/{operation}",
                get(
                    |State(calls): State<Calls>, Path(operation): Path<String>| async move {
                        calls
                            .lock()
                            .unwrap()
                            .push(("receipt".into(), json!({"operation":operation})));
                        Json(json!({"status":"unknown","dispatch_reserved":true}))
                    },
                )
                .delete(
                    |State(calls): State<Calls>, Path(operation): Path<String>| async move {
                        calls
                            .lock()
                            .unwrap()
                            .push(("cancel".into(), json!({"operation":operation})));
                        Json(json!({"status":"cancellation_requested"}))
                    },
                ),
            )
            .route(
                "/api/provider-accounts/{account}/login",
                get(
                    |State(calls): State<Calls>,
                     Path(account): Path<String>,
                     Query(query): Query<HashMap<String, String>>| async move {
                        calls
                            .lock()
                            .unwrap()
                            .push(("options".into(), json!({"account":account,"query":query})));
                        Json(json!({"available":true}))
                    },
                )
                .post(
                    |State(calls): State<Calls>,
                     Path(account): Path<String>,
                     Json(body): Json<Value>| async move {
                        calls
                            .lock()
                            .unwrap()
                            .push(("start".into(), json!({"account":account,"body":body})));
                        (StatusCode::SERVICE_UNAVAILABLE, "outcome unknown")
                    },
                ),
            )
            .route(
                "/api/providers/{provider}/auth/{request}",
                get(
                    |State(calls): State<Calls>,
                     Path((provider, request)): Path<(String, String)>| async move {
                        calls.lock().unwrap().push((
                            "events".into(),
                            json!({"provider":provider,"request":request}),
                        ));
                        Json(json!({"events":[]}))
                    },
                )
                .post(
                    |State(calls): State<Calls>, Json(body): Json<Value>| async move {
                        calls.lock().unwrap().push(("code".into(), body));
                        StatusCode::ACCEPTED
                    },
                ),
            )
            .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::fixture(
        Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap(),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    execute(
        &client,
        AccountCommand::LoginOptions {
            account: parse_account(ACCOUNT).unwrap(),
            machine: "machine &other=value".into(),
        },
        std::io::empty(),
    )
    .await
    .unwrap();
    assert!(
        execute(
            &client,
            AccountCommand::Login {
                operation: "login-operation".into(),
                account: parse_account(ACCOUNT).unwrap(),
                machine: "machine-a".into(),
                provider_version: "1.0.0".into(),
                generation_digest: "sha256:exact".into(),
                method: "native".into()
            },
            std::io::empty()
        )
        .await
        .is_err()
    );
    for command in [
        AccountCommand::Events {
            provider: "claude-code".into(),
            request: "login-1".into(),
        },
        AccountCommand::SubmitCode {
            provider: "claude-code".into(),
            request: "login-1".into(),
        },
        AccountCommand::Cancel {
            operation: "login-operation".into(),
        },
        AccountCommand::Receipt {
            operation: "login-operation".into(),
        },
    ] {
        execute(&client, command, "synthetic-code\n".as_bytes())
            .await
            .unwrap();
    }
    let observed = calls.lock().unwrap();
    assert_eq!(
        observed
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["options", "start", "events", "code", "cancel", "receipt"]
    );
    assert_eq!(
        observed[0].1,
        json!({"account":ACCOUNT,"query":{"machine_id":"machine &other=value"}})
    );
    assert_eq!(
        observed[1].1,
        json!({"account":ACCOUNT,"body":{"operation":"login-operation","machine_id":"machine-a","provider_version":"1.0.0","generation_digest":"sha256:exact","method":"native"}})
    );
    assert_eq!(
        observed[2].1,
        json!({"provider":"claude-code","request":"login-1"})
    );
    assert_eq!(observed[3].1, json!({"code":"synthetic-code"}));
    assert_eq!(observed[4].1, json!({"operation":"login-operation"}));
    assert_eq!(observed[5].1, json!({"operation":"login-operation"}));
    server.abort();
}
