#[test]
fn cowboy_cli_binary_artifact_is_emitted() {
    let path = std::path::Path::new(env!("CARGO_BIN_EXE_cowboy"));
    assert!(
        path.is_file(),
        "expected cowboy binary at {}",
        path.display()
    );
}
