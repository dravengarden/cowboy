use super::*;

fn snapshot(sha: char, buffer: &str) -> BufferSnapshot {
    serde_json::from_value(
        json!({"instance":"a".repeat(32),"buffer":buffer,"bufferEpoch":"1",
        "version":[],"content":{"sha256":sha.to_string().repeat(64),"utf8Bytes":10}}),
    )
    .unwrap()
}

fn diagnostic(row: u32, severity: i32, code: Option<&str>, buffer: &str) -> CodeDiagnostic {
    let mut value = json!({"start":{"row":row,"column":0},"end":{"row":row,"column":1},
        "severity":severity,"source":"compiler","message":format!("message {row}"),
        "provenance":{"instance":"a".repeat(32),"buffer":buffer,"bufferEpoch":"1",
            "server":"3","sourceKind":1,"registration":null}});
    if let Some(code) = code {
        value["code"] = json!(code);
    }
    serde_json::from_value(value).unwrap()
}

fn servers(ids: &[u64]) -> Vec<ServerAnalysis> {
    ids.iter()
        .map(|id| {
            serde_json::from_value(json!({"serverId":id,"name":"","state":"document_current"}))
                .unwrap()
        })
        .collect()
}

fn scope(id: &str) -> SessionCodeScope {
    let hub = crate::core::Hub::new();
    hub.create_local_session(
        id.into(),
        "codex".into(),
        "/tmp".into(),
        "fixture".into(),
        crate::core::SessionOrigin::Web,
        false,
    );
    hub.session_code_scope(id).unwrap()
}

fn observe(scope: &SessionCodeScope, sha: char, diagnostics: &[CodeDiagnostic]) -> Value {
    let pending = capture(diagnostics, Some(&snapshot(sha, "7")), &servers(&[3])).unwrap();
    record(scope, "a.rs", pending, diagnostics)
}

#[test]
fn first_capture_and_unchanged_text_have_no_clean_comparison() {
    let scope = scope("baseline-first");
    let current = [diagnostic(0, 1, Some("E1"), "7")];
    assert_eq!(observe(&scope, 'b', &current), unknown("no_baseline"));
    // Re-reading the same text never promotes it to its own baseline.
    assert_eq!(observe(&scope, 'b', &current), unknown("no_baseline"));
}

#[test]
fn changed_text_compares_by_identity_multiplicity_not_position_or_message() {
    let scope = scope("baseline-change");
    observe(
        &scope,
        'b',
        &[
            diagnostic(1, 2, Some("W1"), "7"),
            diagnostic(5, 1, Some("E1"), "7"),
        ],
    );
    let result = observe(
        &scope,
        'c',
        &[
            // Moved and reworded: persists.
            diagnostic(9, 1, Some("E1"), "7"),
            // Escalated from warning to error.
            diagnostic(2, 1, Some("W1"), "7"),
            // A second instance of a present identity is ambiguous.
            diagnostic(4, 1, Some("E1"), "7"),
            diagnostic(6, 1, Some("E2"), "7"),
        ],
    );
    assert_eq!(result["state"], "complete");
    assert_eq!(result["against"]["sha256"], "b".repeat(64));
    assert_eq!(result["introduced"], 2);
    assert_eq!(result["resolved"], 0);
    assert_eq!(result["persisting"], 2);
    assert_eq!(result["escalated"], 1);
    let candidates = result["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3);
    assert!(
        candidates
            .iter()
            .any(|c| c["code"] == "E2" && c["ambiguous"] == false)
    );
    assert!(
        candidates
            .iter()
            .filter(|c| c["code"] == "E1")
            .all(|c| c["ambiguous"] == true)
    );
    // The next edit compares against the immediately preceding text.
    let next = observe(&scope, 'd', &[]);
    assert_eq!(next["against"]["sha256"], "c".repeat(64));
    assert_eq!(next["resolved"], 4);
}

#[test]
fn diagnostics_without_code_stay_unmatched_instead_of_hashing_messages() {
    let scope = scope("baseline-uncoded");
    observe(&scope, 'b', &[diagnostic(0, 1, None, "7")]);
    let result = observe(&scope, 'c', &[diagnostic(0, 1, None, "7")]);
    assert_eq!(result["state"], "partial");
    assert_eq!(result["persisting"], 0);
    assert_eq!(result["unmatched"], json!({"baseline":1,"current":1}));
}

#[test]
fn incompatible_baselines_are_unknown() {
    let scope = scope("baseline-incompatible");
    let base = [diagnostic(0, 1, Some("E1"), "7")];
    observe(&scope, 'b', &base);
    let pending = capture(&base, Some(&snapshot('c', "7")), &servers(&[3, 4])).unwrap();
    assert_eq!(
        record(&scope, "a.rs", pending, &base),
        unknown("servers_changed")
    );
    let replaced = [diagnostic(0, 1, Some("E1"), "8")];
    let pending = capture(&replaced, Some(&snapshot('d', "8")), &servers(&[3, 4])).unwrap();
    assert_eq!(
        record(&scope, "a.rs", pending, &replaced),
        unknown("buffer_replaced")
    );
    let many: Vec<_> = (0..=MAX_CAPTURED as u32)
        .map(|row| diagnostic(row, 1, Some("E1"), "8"))
        .collect();
    let pending = capture(&many, Some(&snapshot('e', "8")), &servers(&[3, 4])).unwrap();
    assert_eq!(
        record(&scope, "a.rs", pending, &many),
        unknown("over_capture_bound")
    );
    // A different Session scope never sees another's baseline.
    assert_eq!(
        observe(&super::tests::scope("baseline-other"), 'f', &base),
        unknown("no_baseline")
    );
}

#[test]
fn records_are_bounded_per_session() {
    let scope = scope("baseline-bounded");
    for index in 0..=MAX_FILES_PER_SESSION {
        let pending = capture(&[], Some(&snapshot('b', "7")), &servers(&[3])).unwrap();
        record(&scope, &format!("f{index}.rs"), pending, &[]);
    }
    let records = RECORDS.lock();
    assert_eq!(
        records.keys().filter(|(owner, _)| owner == &scope).count(),
        MAX_FILES_PER_SESSION
    );
    assert!(!records.contains_key(&(scope.clone(), "f0.rs".to_owned())));
}

#[test]
fn unbound_or_unbounded_snapshots_cannot_be_captured() {
    assert!(capture(&[], None, &servers(&[3])).is_none());
    let invalid: BufferSnapshot = serde_json::from_value(json!({"instance":"short","buffer":"7",
        "bufferEpoch":"1","version":[],"content":{"sha256":"b".repeat(64),"utf8Bytes":10}}))
    .unwrap();
    assert!(capture(&[], Some(&invalid), &servers(&[3])).is_none());
    assert_eq!(rank(0), i32::MAX);
}
