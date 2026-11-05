
use super::{parse_workspace_code_context, parse_worktree_code_context};

#[test]
fn a_worktree_context_names_an_absolute_path_once() {
    let hex = |path: &str| {
        path.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let value = format!("worktree::hawk::cowboy::{}", hex("/tasks/a"));
    assert_eq!(
        parse_worktree_code_context(&value),
        Some(("hawk", "cowboy", "/tasks/a".to_owned()))
    );
    for bad in [
        format!("worktree::hawk::cowboy::{}", hex("relative")),
        format!("worktree::hawk::cowboy::{}", hex("/nul\0")),
        format!("worktree::hawk::::{}", hex("/a")),
        format!("worktree::hawk::cowboy::{}::extra", hex("/a")),
        "worktree::hawk::cowboy::zz".to_owned(),
        "worktree::hawk::cowboy::2".to_owned(),
    ] {
        assert_eq!(parse_worktree_code_context(&bad), None, "{bad}");
    }
}

#[test]
fn only_explicit_machine_workspace_pairs_are_code_contexts() {
    assert_eq!(
        parse_workspace_code_context("workspace::hawk::cowboy"),
        Some(("hawk", "cowboy"))
    );
    assert_eq!(parse_workspace_code_context("sess-123"), None);
    assert_eq!(parse_workspace_code_context("workspace::::cowboy"), None);
    assert_eq!(
        parse_workspace_code_context("workspace::hawk::cowboy::extra"),
        None
    );
}
