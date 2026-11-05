
use super::{code_file_error_response, zed_language_target};
use axum::http::StatusCode;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn file_errors_distinguish_missing_binary_and_invalid_requests() {
    assert_eq!(
        code_file_error_response("file not found").status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        code_file_error_response("binary file").status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        code_file_error_response("file is not UTF-8").status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        code_file_error_response("invalid file cursor").status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn local_buffer_leases_require_a_real_file_inside_the_session_worktree() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cowboy-buffer-policy-{unique}"));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

    let root_text = root.to_string_lossy();
    assert!(zed_language_target("local", &root_text, "src/main.rs").is_some());
    assert!(zed_language_target("local", &root_text, "src/missing.rs").is_none());
    assert!(zed_language_target("local", &root_text, "../outside.rs").is_none());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_buffer_leases_are_resolved_on_the_session_machine() {
    let remote_root = "/Users/example/.local/state/cowboy-machine/worktrees/session";
    assert_eq!(
        zed_language_target(
            "macbook-air",
            remote_root,
            "service/workflow/core/webhook_helpers.go",
        ),
        Some((
            remote_root.to_owned(),
            "service/workflow/core/webhook_helpers.go".to_owned(),
        ))
    );
    assert!(zed_language_target("macbook-air", remote_root, "../outside.go").is_none());
    assert!(zed_language_target("macbook-air", remote_root, "/etc/passwd").is_none());
    assert!(zed_language_target("macbook-air", remote_root, "src\\outside.go").is_none());
}
