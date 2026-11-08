use super::*;

fn lease(owners: &[BufferOwner]) -> BufferLease {
    BufferLease {
        lease_ids: owners.iter().cloned().collect(),
        remote_id: 7,
        version: Vec::new(),
        sync: None,
        closing: false,
    }
}

#[test]
fn only_buffers_owned_exclusively_by_agent_acquisitions_may_reload() {
    let agents: HashSet<u64> = [1, 2].into();
    assert!(agent_only(&lease(&[BufferOwner::Owned(1)]), &agents));
    assert!(agent_only(
        &lease(&[BufferOwner::Owned(1), BufferOwner::Owned(2)]),
        &agents
    ));
    for owners in [
        vec![],
        vec![BufferOwner::Owned(3)],
        vec![BufferOwner::Owned(1), BufferOwner::Owned(3)],
        vec![BufferOwner::Owned(1), BufferOwner::Legacy("tab".into())],
        // A legacy name cannot impersonate an agent acquisition.
        vec![BufferOwner::Legacy("cowboy-agent-fixture".into())],
        vec![BufferOwner::Owned(1), BufferOwner::Navigation(1)],
        vec![BufferOwner::NavigationPending(1)],
    ] {
        assert!(!agent_only(&lease(&owners), &agents), "{owners:?}");
    }
    let mut closing = lease(&[BufferOwner::Owned(1)]);
    closing.closing = true;
    assert!(!agent_only(&closing, &agents));
}

#[test]
fn refresh_wire_values_are_closed() {
    assert_eq!(
        serde_json::from_str::<Refresh>(r#""agent_owned""#).unwrap(),
        Refresh::AgentOwned
    );
    assert!(serde_json::from_str::<Refresh>(r#""any""#).is_err());
    assert_eq!(
        serde_json::to_value(Outcome::NotNeeded).unwrap(),
        "not_needed"
    );
    let request: crate::Request = serde_json::from_str(
        r#"{"type":"bufferLanguage","worktree":"/tmp","path":"a.rs","refresh":"agent_owned"}"#,
    )
    .unwrap();
    assert!(matches!(
        request,
        crate::Request::BufferLanguage {
            refresh: Some(Refresh::AgentOwned),
            ..
        }
    ));
    let plain: crate::Request =
        serde_json::from_str(r#"{"type":"bufferLanguage","worktree":"/tmp","path":"a.rs"}"#)
            .unwrap();
    assert!(matches!(
        plain,
        crate::Request::BufferLanguage { refresh: None, .. }
    ));
}
