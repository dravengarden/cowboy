use super::*;

fn value(message: &str) -> Value {
    json!({"deliver":true,"files":[{"path":"a.go","diagnostics":[{"message":message}]}]})
}

#[test]
fn identical_results_are_suppressed_but_new_ones_are_delivered() {
    let session = "deliveries-repeat";
    let first = begin(
        session,
        Route::Adapter,
        vec!["a.go".into()],
        digest(&value("x")),
    );
    assert!(first.is_some());
    assert_eq!(
        begin(
            session,
            Route::Adapter,
            vec!["a.go".into()],
            digest(&value("x"))
        ),
        None
    );
    assert!(
        begin(
            session,
            Route::Adapter,
            vec!["a.go".into()],
            digest(&value("y"))
        )
        .is_some()
    );
    let listed = list(session);
    let states: Vec<&str> = listed["deliveries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["computed", "suppressed", "computed"]);
    assert_eq!(listed["deliveries"][1]["reason"], "repeat");
    assert!(!listed.to_string().contains("\"x\""), "no repository text");
}

#[test]
fn a_refused_delivery_does_not_suppress_its_retry() {
    let session = "deliveries-refused";
    let id = begin(
        session,
        Route::CodexSteering,
        vec!["a.go".into()],
        digest(&value("x")),
    )
    .unwrap();
    assert_eq!(list(session)["deliveries"][0]["state"], "steering");
    finish(
        session,
        &id,
        State::Refused,
        Some("there is no current turn"),
    );
    assert_eq!(list(session)["deliveries"][0]["state"], "refused");
    let retry = begin(
        session,
        Route::CodexSteering,
        vec!["a.go".into()],
        digest(&value("x")),
    )
    .unwrap();
    finish(session, &retry, State::Appended, None);
    let listed = list(session);
    assert_eq!(listed["deliveries"][1]["state"], "appended");
    assert_eq!(listed["deliveries"][1]["route"], "codex_steering");
}

#[test]
fn the_ledger_is_bounded_per_session() {
    let session = "deliveries-bounded";
    for index in 0..=PER_SESSION {
        begin(
            session,
            Route::Adapter,
            vec![],
            digest(&value(&index.to_string())),
        );
    }
    assert_eq!(
        list(session)["deliveries"].as_array().unwrap().len(),
        PER_SESSION
    );
    assert_eq!(list("deliveries-none")["deliveries"], json!([]));
}
