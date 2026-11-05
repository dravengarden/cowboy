use super::{Format, Severity, parse, parse_config};

#[test]
fn declarations_are_validated() {
    let ok = br#"{"schemaVersion":1,"checks":[{"id":"rust","argv":["cargo","check","--message-format=json"],"format":"cargo-json","timeoutSeconds":900}]}"#;
    let config = parse_config(ok).unwrap();
    assert_eq!(config.checks[0].timeout().as_secs(), 900);
    for bad in [
        &br#"{"schemaVersion":2,"checks":[]}"#[..],
        br#"{"schemaVersion":1,"checks":[]}"#,
        br#"{"schemaVersion":1,"checks":[{"id":"Rust","argv":["x"],"format":"lines"}]}"#,
        br#"{"schemaVersion":1,"checks":[{"id":"a","argv":[],"format":"lines"}]}"#,
        br#"{"schemaVersion":1,"checks":[{"id":"a","argv":["x"],"format":"lines"},{"id":"a","argv":["y"],"format":"lines"}]}"#,
        br#"{"schemaVersion":1,"checks":[{"id":"a","argv":["x"],"format":"make"}]}"#,
        br#"{"schemaVersion":1,"checks":[{"id":"a","argv":["x"],"format":"lines","shell":true}]}"#,
    ] {
        assert!(parse_config(bad).is_err(), "{}", String::from_utf8_lossy(bad));
    }
    let long = parse_config(
        br#"{"schemaVersion":1,"checks":[{"id":"a","argv":["x"],"format":"lines","timeoutSeconds":99999}]}"#,
    )
    .unwrap();
    assert_eq!(long.checks[0].timeout().as_secs(), 1_800);
}

#[test]
fn cargo_messages_keep_primary_spans_relative_to_the_worktree() {
    let stdout = [
        r#"{"reason":"compiler-artifact","package_id":"x"}"#,
        r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","code":{"code":"E0308"},"spans":[{"file_name":"src/other.rs","line_start":1,"column_start":1,"is_primary":false},{"file_name":"/w/src/lib.rs","line_start":12,"column_start":5,"is_primary":true}]}}"#,
        r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable","code":null,"spans":[{"file_name":"src/main.rs","line_start":3,"column_start":9,"is_primary":true}]}}"#,
        r#"{"reason":"compiler-message","message":{"level":"failure-note","message":"x","spans":[]}}"#,
        "not json",
    ]
    .join("\n");
    let parsed = parse(Format::CargoJson, &stdout, "", "/w");
    assert_eq!(parsed.diagnostics.len(), 2);
    let first = &parsed.diagnostics[0];
    assert_eq!(
        (first.path.as_str(), first.line, first.column),
        ("src/lib.rs", 12, 5)
    );
    assert_eq!(first.severity, Severity::Error);
    assert_eq!(first.code.as_deref(), Some("E0308"));
    assert_eq!(parsed.diagnostics[1].severity, Severity::Warning);
}

#[test]
fn tsc_and_generic_lines() {
    let tsc = "src/a.ts(4,7): error TS2322: Type 'string' is not assignable to type 'number'.\nFound 1 error.";
    let parsed = parse(Format::Tsc, tsc, "", "/w");
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].code.as_deref(), Some("TS2322"));
    assert_eq!(
        (parsed.diagnostics[0].line, parsed.diagnostics[0].column),
        (4, 7)
    );

    let generic = "/w/pkg/x.go:10:2: undefined: foo\nmain.c:3: warning: implicit declaration\nok\n";
    let parsed = parse(Format::Lines, "", generic, "/w");
    assert_eq!(parsed.diagnostics.len(), 2);
    assert_eq!(parsed.diagnostics[0].path, "pkg/x.go");
    assert_eq!(parsed.diagnostics[0].severity, Severity::Error);
    assert_eq!(parsed.diagnostics[1].column, 0);
    assert_eq!(parsed.diagnostics[1].severity, Severity::Warning);
}

#[test]
fn diagnostics_are_bounded() {
    let output: String = (0..1_200)
        .map(|n| format!("a.c:{}: error: x\n", n + 1))
        .collect();
    let parsed = parse(Format::Lines, &output, "", "/w");
    assert_eq!(parsed.diagnostics.len(), super::MAX_DIAGNOSTICS);
    assert!(parsed.truncated);
}
