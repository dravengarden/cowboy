use super::{Hub, SessionOrigin, merge_session_order, project_sync_value};
use std::collections::HashSet;

#[test]
fn stale_omit_keeps_the_missing_visible_id() {
    let merged = merge_session_order(
        &["A".to_owned(), "B".to_owned(), "C".to_owned()],
        &["C".to_owned(), "A".to_owned()],
    );
    assert_eq!(merged, vec!["C".to_owned(), "B".to_owned(), "A".to_owned()]);
}

#[test]
fn hidden_id_stays_in_its_held_slot() {
    let merged = merge_session_order(
        &["A".to_owned(), "hidden".to_owned(), "B".to_owned()],
        &["B".to_owned(), "A".to_owned()],
    );
    assert_eq!(
        merged,
        vec!["B".to_owned(), "hidden".to_owned(), "A".to_owned()]
    );
}

#[test]
fn hub_reorder_never_drops_omitted_ids() {
    let hub = Hub::new();
    for id in ["A", "B", "C"] {
        hub.create_local_session(
            id.to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            id.to_owned(),
            SessionOrigin::Web,
            false,
        );
    }
    hub.reorder_sessions(&["C".to_owned(), "A".to_owned()]);
    let order: Vec<String> = hub.session_list().into_iter().map(|meta| meta.id).collect();
    assert_eq!(order, vec!["C".to_owned(), "B".to_owned(), "A".to_owned()]);
}

#[test]
fn project_sync_value_drops_hidden_title_and_order_ids() {
    let visible = HashSet::from(["A".to_owned(), "C".to_owned()]);
    let titles = project_sync_value(
        "title",
        serde_json::json!({ "A": "one", "B": "hidden", "C": "three" }),
        &visible,
    );
    assert_eq!(titles, serde_json::json!({ "A": "one", "C": "three" }));
    let order = project_sync_value("order", serde_json::json!(["A", "B", "C"]), &visible);
    assert_eq!(order, serde_json::json!(["A", "C"]));
}
