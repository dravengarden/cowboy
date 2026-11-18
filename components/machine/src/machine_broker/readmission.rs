//! The only session context a native readmission may change.
use crate::runtime_wire::StartSession;

/// Apply the selected binding of `session` to the `previous` declaration.
/// Every workspace, extension and execution placement field must still
/// agree afterwards. The native thread resets, except in an account
/// sandbox, whose persistent home keeps the conversation across sign-ins
/// (R3-13).
pub(super) fn rebind(previous: &mut StartSession, session: &StartSession) {
    previous
        .provider_version
        .clone_from(&session.provider_version);
    previous
        .provider_generation_digest
        .clone_from(&session.provider_generation_digest);
    previous.provider_auth_generation = session.provider_auth_generation;
    previous
        .provider_behavior
        .clone_from(&session.provider_behavior);
    if !crate::provider_account::named_only(&previous.provider) {
        previous.agent_session_id = None;
    }
    previous.adopt_only = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_account_sandbox_keeps_its_conversation_on_readmission() {
        for (provider, kept) in [("antigravity", true), ("claude-code", false)] {
            let mut previous: StartSession = serde_json::from_value(serde_json::json!({
                "session_id": "s", "provider": provider, "cwd": "/tmp", "generation": "g",
                "agent_session_id": "native-thread", "provider_version": "old",
            }))
            .unwrap();
            let mut session = previous.clone();
            session.provider_version = "new".into();
            rebind(&mut previous, &session);
            assert_eq!(previous.provider_version, "new");
            assert!(previous.adopt_only);
            assert_eq!(previous.agent_session_id.is_some(), kept, "{provider}");
        }
    }
}
