use super::*;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::{MachineCommand, MachineEvent};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse as _;
use base64::Engine as _;
use tokio::sync::mpsc;

struct Fixture {
    store: Store,
    control: MachineControl,
    connection: ConnectionToken,
    _commands: mpsc::UnboundedReceiver<MachineCommand>,
    _root: tempfile::TempDir,
}

fn workspace() -> MachineWorkspace {
    MachineWorkspace {
        id: "workspace".into(),
        display_name: "Workspace".into(),
        canonical_path: "/work/original".into(),
    }
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let token = store
            .create_machine_enrollment("machine", "Machine", 60)
            .await
            .unwrap();
        let secret = x25519_dalek::StaticSecret::from([0x24_u8; 32]);
        let public = base64::engine::general_purpose::STANDARD
            .encode(x25519_dalek::PublicKey::from(&secret).as_bytes());
        store
            .consume_machine_enrollment(&token, "ssh-ed25519 QUJD", &public)
            .await
            .unwrap();
        store
            .machine_connected(
                "machine",
                "epoch",
                "linux",
                "x86_64",
                "outbound_wss",
                &serde_json::json!({"workspaces": []}),
            )
            .await
            .unwrap();
        let control = MachineControl::default();
        let (tx, commands) = mpsc::unbounded_channel();
        let connection = control.install("machine".into(), "epoch".into(), false, 21, tx);
        let fixture = Self {
            store,
            control,
            connection,
            _commands: commands,
            _root: root,
        };
        fixture.observe(vec![workspace()]).await;
        fixture
    }

    async fn observe(&self, workspaces: Vec<MachineWorkspace>) {
        self.control.record_remote(
            &self.connection,
            MachineEvent::Inventory {
                components: vec![],
                workspaces: Some(workspaces.clone()),
                workspace_identities: None,
                workspace_revision: None,
                observed_at_ms: 0,
            },
        );
        self.store
            .machine_seen(
                "machine",
                "epoch",
                Some(&serde_json::json!({"workspaces": workspaces})),
            )
            .await
            .unwrap();
    }

    async fn scope(&self) -> Option<CodeReadScope> {
        resolve(
            &self.store,
            &self.control,
            "service-test",
            "machine",
            "workspace",
        )
        .await
    }

    async fn current(&self, original: &CodeReadScope) -> bool {
        self.scope().await.as_ref() == Some(original)
            && match original {
                CodeReadScope::Workspace(scope) => self.control.workspace_scope_is_current(scope),
                CodeReadScope::Session(_) => false,
            }
    }
}

#[tokio::test]
async fn workspace_read_scope_never_revives_after_observed_remove_and_readd() {
    let fixture = Fixture::new().await;
    let original = fixture.scope().await.unwrap();
    fixture.observe(vec![]).await;
    fixture.observe(vec![workspace()]).await;
    assert_ne!(fixture.scope().await.unwrap(), original);
}

#[tokio::test]
async fn workspace_read_scope_never_adopts_an_identical_replacement_connection() {
    let fixture = Fixture::new().await;
    let original = fixture.scope().await.unwrap();
    let (tx, _commands) = mpsc::unbounded_channel();
    let connection = fixture
        .control
        .install("machine".into(), "epoch".into(), false, 21, tx);
    fixture.control.record_remote(
        &connection,
        MachineEvent::Inventory {
            components: vec![],
            workspaces: Some(vec![workspace()]),
            workspace_identities: None,
            workspace_revision: None,
            observed_at_ms: 0,
        },
    );
    assert_ne!(fixture.scope().await.unwrap(), original);
}

#[tokio::test]
async fn workspace_resolution_requires_both_live_and_persisted_unambiguous_inventory() {
    let fixture = Fixture::new().await;
    let original = fixture.scope().await.unwrap();
    assert!(
        resolve(
            &fixture.store,
            &fixture.control,
            "foreign-service",
            "machine",
            "workspace"
        )
        .await
        .is_none()
    );
    let mut changed = workspace();
    changed.canonical_path = "/different".into();
    for roots in [vec![], vec![workspace(), workspace()], vec![changed]] {
        fixture
            .store
            .machine_seen(
                "machine",
                "epoch",
                Some(&serde_json::json!({"workspaces": roots})),
            )
            .await
            .unwrap();
        assert!(fixture.scope().await.is_none());
        assert!(!fixture.current(&original).await);
    }
    fixture.observe(vec![workspace()]).await;
    assert!(fixture.current(&original).await);
    fixture.control.disconnect("machine");
    assert!(
        fixture.scope().await.is_none(),
        "saved inventory is not a live owner"
    );
}

#[tokio::test]
async fn workspace_resolution_refuses_revoked_enrollment_even_with_a_live_channel() {
    let fixture = Fixture::new().await;
    let original = fixture.scope().await.unwrap();
    fixture.store.revoke_machine("machine").await.unwrap();
    assert!(fixture.scope().await.is_none());
    assert!(!fixture.current(&original).await);
}

#[tokio::test]
async fn workspace_buffered_responses_discard_aba_even_for_cache_hits_and_errors() {
    for status in [
        StatusCode::OK,
        StatusCode::NOT_MODIFIED,
        StatusCode::BAD_GATEWAY,
    ] {
        let fixture = Fixture::new().await;
        let original = fixture.scope().await.unwrap();
        let response = super::super::guarded_response(
            || async {
                fixture
                    .current(&original)
                    .await
                    .then_some(())
                    .ok_or(super::super::Denial::Context)
            },
            || async {
                fixture.observe(vec![]).await;
                fixture.observe(vec![workspace()]).await;
                (status, [(header::ETAG, "\"old\"")], "old bytes").into_response()
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::GONE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!response.headers().contains_key(header::ETAG));
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap(),
            "code context changed"
        );
    }
}

#[tokio::test]
async fn workspace_stale_scope_cannot_invoke_read_setup() {
    let fixture = Fixture::new().await;
    let original = fixture.scope().await.unwrap();
    fixture.observe(vec![]).await;
    fixture.observe(vec![workspace()]).await;
    let invoked = std::cell::Cell::new(false);
    let response = super::super::guarded_response(
        || async {
            fixture
                .current(&original)
                .await
                .then_some(())
                .ok_or(super::super::Denial::Context)
        },
        || {
            invoked.set(true);
            async { "invalid".into_response() }
        },
    )
    .await;
    assert!(!invoked.get());
    assert_eq!(response.status(), StatusCode::GONE);
}

#[tokio::test]
async fn workspace_file_and_diff_cursors_cannot_be_adopted_by_a_recreated_root() {
    use super::super::file_pages::{CursorError, PageCursors};
    use crate::code_review::{DiffDocument, DiffScope, FileDocument};
    use crate::diff_snapshot::{DiffSnapshotCache, DiffSnapshotKey};
    let fixture = Fixture::new().await;
    let original = fixture.scope().await.unwrap();
    let pages = PageCursors::default();
    let cursor = pages
        .project(
            &original,
            "a.txt",
            None,
            FileDocument {
                path: "a.txt".into(),
                revision: "a".repeat(64),
                text: "abc".into(),
                size: 100,
                truncated: true,
                next_cursor: Some(format!("{}:3", "a".repeat(64))),
                limited: false,
            },
        )
        .unwrap()
        .next_cursor
        .unwrap();
    let diffs = DiffSnapshotCache::default();
    let key = DiffSnapshotKey {
        owner: original.clone(),
        path: "a.txt".into(),
        context: 6,
        show_whitespace: true,
        scope: DiffScope::Unstaged,
    };
    let diff = diffs
        .first_page(key, || async {
            Ok(DiffDocument {
                path: "a.txt".into(),
                text: "+line\n".repeat(50_000),
                added: 50_000,
                removed: 0,
                truncated: false,
            })
        })
        .await
        .unwrap()
        .next_cursor
        .unwrap();
    let mut renamed = workspace();
    renamed.display_name = "Renamed".into();
    fixture.observe(vec![renamed]).await;
    let continuous = fixture.scope().await.unwrap();
    assert!(pages.resolve(&continuous, "a.txt", Some(&cursor)).is_ok());
    assert!(diffs.next_page(&continuous, &diff).await.is_ok());
    fixture.observe(vec![]).await;
    fixture.observe(vec![workspace()]).await;
    let replacement = fixture.scope().await.unwrap();
    assert_eq!(
        pages
            .resolve(&replacement, "a.txt", Some(&cursor))
            .unwrap_err(),
        CursorError::Expired
    );
    assert_eq!(
        diffs.next_page(&replacement, &diff).await.unwrap_err(),
        "diff snapshot expired"
    );
}
