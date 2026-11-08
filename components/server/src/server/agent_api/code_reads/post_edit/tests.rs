use super::*;

fn diagnostic(line: u64, severity: &str, code: &str, message: &str) -> Value {
    json!({"severity":severity,"line":line,"column":3,"endLine":line,"endColumn":4,
        "source":"compiler","code":code,"provenance":null,"message":message})
}

fn candidate(line: u64, code: &str, ambiguous: bool) -> Value {
    json!({"severity":"error","line":line,"column":3,"source":"compiler","code":code,"ambiguous":ambiguous})
}

#[test]
fn new_diagnostics_against_a_baseline_are_delivered_with_their_messages() {
    let observation = json!({"files":[{"path":"a.go","state":"document_current",
        "diagnostics":[diagnostic(4,"error","E1","old"),diagnostic(6,"error","E1","new"),
            diagnostic(9,"error","E2","other")],
        "comparison":{"state":"complete","introduced":2,"resolved":0,"escalated":0,
            "candidates":[candidate(4,"E1",true),candidate(6,"E1",true),candidate(9,"E2",false)]}}]});
    let result = project(&observation);
    assert_eq!(result["deliver"], true);
    let file = &result["files"][0];
    assert_eq!(file["comparison"]["introduced"], 2);
    let listed = file["diagnostics"].as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[2]["message"], "other");
    assert_eq!(listed[2]["ambiguous"], false);
    assert!(file["diagnostics"][0].get("provenance").is_none());
}

#[test]
fn unchanged_resolved_or_unobserved_files_stay_silent() {
    for file in [
        json!({"path":"a.go","state":"document_current","diagnostics":[diagnostic(1,"error","E1","x")],
            "comparison":{"state":"complete","introduced":0,"resolved":1,"escalated":0,"candidates":[]}}),
        json!({"path":"a.go","state":"stale","diagnostics":[diagnostic(1,"error","E1","x")],
            "comparison":{"state":"unknown","reason":"not_current"}}),
        json!({"path":"a.go","state":"not_open"}),
        json!({"path":"a.go","state":"restricted"}),
    ] {
        let result = project(&json!({"files":[file]}));
        assert_eq!(result["deliver"], false, "{result}");
        assert!(result["files"][0].get("diagnostics").is_none());
    }
}

#[test]
fn without_a_baseline_only_current_errors_are_delivered() {
    let result = project(&json!({"files":[{"path":"a.go","state":"document_current",
        "diagnostics":[diagnostic(1,"warning","W","warn"),diagnostic(2,"error","E","err")],
        "comparison":{"state":"unknown","reason":"no_baseline"}}]}));
    assert_eq!(result["deliver"], true);
    assert_eq!(result["files"][0]["comparison"]["reason"], "no_baseline");
    let listed = result["files"][0]["diagnostics"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["severity"], "error");
}

#[test]
fn listings_are_bounded_by_count_and_text() {
    let diagnostics: Vec<Value> = (1..=30)
        .map(|line| diagnostic(line, "error", "E", &"m".repeat(500)))
        .collect();
    let result = project(&json!({"files":[{"path":"a.go","state":"document_current",
        "diagnostics":diagnostics,"comparison":{"state":"unknown","reason":"no_baseline"}}]}));
    let listed = result["files"][0]["diagnostics"].as_array().unwrap();
    assert_eq!(listed.len(), MAX_ERRORS);
    assert!(
        listed
            .iter()
            .all(|value| value["message"].as_str().unwrap().chars().count() == MAX_MESSAGE_CHARS)
    );
    assert_eq!(result["truncated"], true);
    let total: usize = listed
        .iter()
        .map(|value| value["message"].as_str().unwrap().chars().count())
        .sum();
    assert!(total <= MAX_TOTAL_CHARS);
}

#[test]
fn observed_writes_say_whether_the_analysis_saw_that_exact_text() {
    let observation = json!({"files":[
        {"path":"a.go","state":"document_current","bufferSnapshot":{"content":{"sha256":"aa"}}},
        {"path":"b.go","state":"stale","bufferSnapshot":{"content":{"sha256":"bb"}}},
        {"path":"c.go","state":"not_open"}]});
    let mut projected = project(&observation);
    attach_writes(&mut projected, &observation, |path| match path {
        "a.go" => Some("aa".into()),
        "b.go" => Some("new".into()),
        _ => None,
    });
    assert_eq!(
        projected["files"][0]["write"],
        json!({"observed":true,"matches_analyzed":true})
    );
    assert_eq!(
        projected["files"][1]["write"],
        json!({"observed":true,"matches_analyzed":false})
    );
    assert!(projected["files"][2].get("write").is_none());
}
