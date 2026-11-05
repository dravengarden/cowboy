
use super::*;

#[test]
fn trashed_session_accepts_only_non_waking_commands() {
    let session_id = "sess-1".to_owned();
    assert!(allowed_on_trashed_session(&Inbound::SetQueueEditing {
        session_id: session_id.clone(),
        id: None,
    }));
    assert!(!allowed_on_trashed_session(&Inbound::SetQueueEditing {
        session_id: session_id.clone(),
        id: Some("m-1".to_owned()),
    }));
    assert!(allowed_on_trashed_session(&Inbound::OpenSession {
        session_id: session_id.clone(),
    }));
    assert!(!allowed_on_trashed_session(&Inbound::ResumeTurn {
        session_id
    }));
}
