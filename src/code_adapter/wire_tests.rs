use super::*;
use serde_json::json;

#[test]
fn closed_requests_preserve_every_existing_wire_operation() {
    let mut cases = vec![
        (CodeOperation::Manifest, json!({ "type": "manifest" })),
        (
            CodeOperation::Directory {
                path: "src".into(),
                limit: 80,
            },
            json!({ "type": "directory", "path": "src", "limit": 80 }),
        ),
        (
            CodeOperation::Search {
                query: "文件".into(),
                limit: 25,
            },
            json!({ "type": "search", "query": "文件", "limit": 25 }),
        ),
        (CodeOperation::Changes, json!({ "type": "changes" })),
        (
            CodeOperation::Repository { after: None },
            json!({ "type": "repository" }),
        ),
        (
            CodeOperation::Repository {
                after: Some("commit-id".into()),
            },
            json!({ "type": "repository", "after": "commit-id" }),
        ),
        (
            CodeOperation::Commit {
                oid: "commit-id".into(),
            },
            json!({ "type": "commit", "oid": "commit-id" }),
        ),
        (
            CodeOperation::CommitDiff {
                oid: "commit-id".into(),
                path: "src/lib.rs".into(),
            },
            json!({ "type": "commit_diff", "oid": "commit-id", "path": "src/lib.rs" }),
        ),
        (
            CodeOperation::File {
                path: "src/lib.rs".into(),
                cursor: None,
            },
            json!({ "type": "file", "path": "src/lib.rs", "cursor": null }),
        ),
        (
            CodeOperation::File {
                path: "src/lib.rs".into(),
                cursor: Some("digest:12".into()),
            },
            json!({ "type": "file", "path": "src/lib.rs", "cursor": "digest:12" }),
        ),
        (
            CodeOperation::FileRaw {
                path: "image.png".into(),
            },
            json!({ "type": "file_raw", "path": "image.png" }),
        ),
    ];
    for (scope, wire_scope) in [
        (DiffScope::Combined, "combined"),
        (DiffScope::Staged, "staged"),
        (DiffScope::Unstaged, "unstaged"),
    ] {
        cases.push((
            CodeOperation::Diff { path: "src/lib.rs".into(), context: 7, show_whitespace: true, scope },
            json!({ "type": "diff", "path": "src/lib.rs", "context": 7, "show_whitespace": true, "scope": wire_scope }),
        ));
    }
    for (operation, mut expected) in cases {
        expected["root"] = json!("/workspace/项目");
        let wire = serde_json::to_value(CodeAdapterRequest {
            root: "/workspace/项目".into(),
            operation,
        })
        .unwrap();
        assert_eq!(wire, expected);
        let decoded: CodeAdapterRequest = serde_json::from_value(wire).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }
}

#[test]
fn unknown_operations_and_invalid_typed_fields_are_rejected() {
    for invalid in [
        json!({ "root": "/work", "type": "execute" }),
        json!({ "root": "/work", "type": "search", "query": "a", "limit": "20" }),
        json!({ "root": "/work", "type": "file", "cursor": null }),
        json!({ "root": "/work", "type": "diff", "path": "a", "context": 3, "show_whitespace": false, "scope": "all" }),
    ] {
        assert!(serde_json::from_value::<CodeAdapterRequest>(invalid).is_err());
    }
}
