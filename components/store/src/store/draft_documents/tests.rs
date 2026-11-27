use super::*;

fn mutation(id: &str, op: &str, expected_revision: i64, change: DraftChange) -> DraftMutation {
    DraftMutation {
        document_id: id.to_owned(),
        operation_id: op.to_owned(),
        expected_revision,
        change,
    }
}
fn create(kind: DraftKind, title: &str, parent: Option<&str>, body: &str) -> DraftChange {
    DraftChange::Create {
        kind,
        title: title.to_owned(),
        parent_id: parent.map(str::to_owned),
        body: body.to_owned(),
        attachments: vec![],
        trashed: false,
        source_session_id: None,
    }
}
fn write(body: &str) -> DraftChange {
    DraftChange::Write {
        body: body.to_owned(),
        attachments: vec![],
    }
}
fn applied(result: DraftResult) -> DraftDocument {
    match result {
        DraftResult::Applied(document) => document,
        other => panic!("expected applied: {other:?}"),
    }
}

#[allow(clippy::too_many_lines)] // One revision/CAS/history contract shared by SQLite and PostgreSQL.
async fn contract(store: &Store) {
    store.migrate().await.unwrap();
    // Metadata keyset pages stay owner-scoped and full across tombstones.
    for (id, owner) in [
        ("a", "pager"),
        ("b", "pager"),
        ("c", "pager"),
        ("d", "pager"),
        ("e", "other-pager"),
    ] {
        applied(
            store
                .mutate_draft_document(
                    owner,
                    &mutation(
                        id,
                        &format!("page-create-{id}"),
                        0,
                        create(DraftKind::Document, id, None, &"large body".repeat(1000)),
                    ),
                )
                .await
                .unwrap(),
        );
    }
    applied(
        store
            .mutate_draft_document("pager", &mutation("b", "page-trash", 1, DraftChange::Trash))
            .await
            .unwrap(),
    );
    let page = store
        .draft_documents_page("pager", None, 2, false)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ["a", "c"]
    );
    assert!(
        page.iter()
            .all(|d| d.body.is_empty() && d.attachments.is_empty())
    );
    let page = store
        .draft_documents_page("pager", Some("c"), 2, false)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ["d"]
    );
    let page = store
        .draft_documents_page("pager", Some("a"), 2, true)
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ["b", "c"]
    );
    assert!(page[0].deleted);
    assert!(
        store
            .draft_documents_page("pager", Some("d"), 2, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .draft_documents_page("pager", None, 0, false)
            .await
            .is_err()
    );
    assert!(
        store
            .draft_documents_page("pager", None, 201, false)
            .await
            .is_err()
    );
    // Text-only agent edits preserve attachments. Receipts commit with the
    // edit; replay cannot reapply stale content after a human's later edit.
    let attachment =
        serde_json::json!({"id":"attachment-test","type":"image","url":"attachment:test"});
    let with_attachment = mutation(
        "attachment-doc",
        "attachment-create",
        0,
        DraftChange::Create {
            kind: DraftKind::Document,
            title: "Attached".into(),
            parent_id: None,
            body: "Before".into(),
            attachments: vec![attachment.clone()],
            trashed: false,
            source_session_id: None,
        },
    );
    applied(
        store
            .mutate_draft_document("writer", &with_attachment)
            .await
            .unwrap(),
    );
    let text_edit = mutation(
        "attachment-doc",
        "text-edit",
        1,
        DraftChange::WriteText {
            body: "Agent".into(),
        },
    );
    let edited_text = applied(
        store
            .mutate_draft_document("writer", &text_edit)
            .await
            .unwrap(),
    );
    assert_eq!(edited_text.attachments, vec![attachment]);
    assert_eq!(
        store
            .draft_operation_document("writer", "text-edit")
            .await
            .unwrap()
            .as_deref(),
        Some("attachment-doc")
    );
    assert!(
        store
            .draft_operation_document("other", "text-edit")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .draft_operation_document("writer", "unknown")
            .await
            .unwrap()
            .is_none()
    );
    let later = applied(
        store
            .mutate_draft_document(
                "writer",
                &mutation("attachment-doc", "later-text", 2, write("Human")),
            )
            .await
            .unwrap(),
    );
    assert_eq!(
        applied(
            store
                .mutate_draft_document("writer", &text_edit)
                .await
                .unwrap()
        ),
        later
    );
    assert!(matches!(
        store
            .mutate_draft_document(
                "writer",
                &mutation(
                    "attachment-doc",
                    "text-edit",
                    1,
                    DraftChange::WriteText {
                        body: "Changed".into()
                    }
                )
            )
            .await
            .unwrap(),
        DraftResult::Invalid(_)
    ));
    let folder = mutation(
        "folder",
        "create-folder",
        0,
        create(DraftKind::Folder, "Plans", None, ""),
    );
    applied(
        store
            .mutate_draft_document("writer", &folder)
            .await
            .unwrap(),
    );
    let doc = mutation(
        "doc",
        "create-doc",
        0,
        create(DraftKind::Document, "中文 📝", Some("folder"), "Original\n"),
    );
    let first = applied(store.mutate_draft_document("writer", &doc).await.unwrap());
    assert_eq!(first.body_revision, 1);
    assert!(
        store
            .draft_document("other", "doc")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .draft_history("other", "doc")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store
            .mutate_draft_document(
                "other",
                &mutation(
                    "other-doc",
                    "steal-folder",
                    0,
                    create(DraftKind::Document, "Other", Some("folder"), "")
                )
            )
            .await
            .unwrap(),
        DraftResult::Invalid(_)
    ));

    let moved = applied(
        store
            .mutate_draft_document(
                "writer",
                &mutation("doc", "move-root", 1, DraftChange::Move { parent_id: None }),
            )
            .await
            .unwrap(),
    );
    assert_eq!(moved.body_revision, 1);
    assert_eq!(moved.metadata_revision, 2);
    let edit = mutation("doc", "write-one", 1, write("First writer"));
    let edited = applied(store.mutate_draft_document("writer", &edit).await.unwrap());
    assert_eq!(edited.parent_id, None);
    assert_eq!(edited.body, "First writer");
    assert!(matches!(
        store
            .mutate_draft_document(
                "writer",
                &mutation("doc", "stale-write", 1, write("Second writer"))
            )
            .await
            .unwrap(),
        DraftResult::Conflict(Some(_))
    ));
    // Retrying after a transport failure/restart never writes twice.
    assert_eq!(
        applied(
            store
                .clone()
                .mutate_draft_document("writer", &edit)
                .await
                .unwrap()
        ),
        edited
    );
    assert!(matches!(
        store
            .mutate_draft_document(
                "writer",
                &mutation("doc", "write-one", 1, write("Changed request"))
            )
            .await
            .unwrap(),
        DraftResult::Invalid(_)
    ));
    assert_eq!(
        store.draft_history("writer", "doc").await.unwrap()[0].body,
        "Original\n"
    );
    assert!(
        store
            .draft_documents("writer")
            .await
            .unwrap()
            .iter()
            .all(|d| d.body.is_empty() && d.attachments.is_empty())
    );

    // Autosave cadence: writes seconds apart replace the newest text without
    // pushing a snapshot of every keystroke into the recovery history.
    let typing = applied(
        store
            .mutate_draft_document(
                "writer",
                &mutation(
                    "typing",
                    "typing-create",
                    0,
                    create(DraftKind::Document, "Typing", None, "Start"),
                ),
            )
            .await
            .unwrap(),
    );
    let mut revision = typing.body_revision;
    for (index, body) in ["typed a", "typed ab", "typed abc"].into_iter().enumerate() {
        revision = applied(
            store
                .mutate_draft_document(
                    "writer",
                    &mutation(
                        "typing",
                        &format!("autosave-{index}"),
                        revision,
                        write(body),
                    ),
                )
                .await
                .unwrap(),
        )
        .body_revision;
    }
    let history = store.draft_history("writer", "typing").await.unwrap();
    assert_eq!(
        history.iter().map(|d| d.body.as_str()).collect::<Vec<_>>(),
        ["Start"]
    );
    // The History list carries no content; a version is read on its own.
    let index = store.draft_history_index("writer", "typing").await.unwrap();
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].preview, "Start");
    assert_eq!(index[0].characters, 5);
    assert_eq!(index[0].body_revision, typing.body_revision);
    let snapshot = store
        .draft_history_snapshot("writer", "typing", index[0].revision)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.body, "Start");
    assert!(
        store
            .draft_history_snapshot("other", "typing", index[0].revision)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .draft_history_snapshot("writer", "typing", index[0].revision + 1000)
            .await
            .unwrap()
            .is_none()
    );

    // Independent metadata and content writers, then competing body writers.
    let left = mutation("doc", "race-a", 2, write("Left"));
    let right = mutation("doc", "race-b", 2, write("Right"));
    let (a, b) = tokio::join!(
        store.mutate_draft_document("writer", &left),
        store.mutate_draft_document("writer", &right)
    );
    assert!(matches!(
        (&a, &b),
        (Ok(DraftResult::Applied(_)), Ok(DraftResult::Conflict(_)))
            | (Ok(DraftResult::Conflict(_)), Ok(DraftResult::Applied(_)))
    ));
    let current = store
        .draft_document("writer", "doc")
        .await
        .unwrap()
        .unwrap();
    let trashed = applied(
        store
            .mutate_draft_document(
                "writer",
                &mutation("doc", "trash", current.revision, DraftChange::Trash),
            )
            .await
            .unwrap(),
    );
    assert!(matches!(
        store
            .mutate_draft_document(
                "writer",
                &mutation(
                    "doc",
                    "write-deleted",
                    current.body_revision,
                    write("Resurrect silently")
                )
            )
            .await
            .unwrap(),
        DraftResult::Conflict(_)
    ));
    assert!(applied(store.mutate_draft_document("writer", &doc).await.unwrap()).deleted);
    let restored = applied(
        store
            .mutate_draft_document(
                "writer",
                &mutation("doc", "restore", trashed.revision, DraftChange::Restore),
            )
            .await
            .unwrap(),
    );
    assert!(!restored.deleted);

    applied(
        store
            .mutate_draft_document(
                "writer",
                &mutation(
                    "child",
                    "new-child",
                    0,
                    create(DraftKind::Folder, "Child", Some("folder"), ""),
                ),
            )
            .await
            .unwrap(),
    );
    assert!(matches!(
        store
            .mutate_draft_document(
                "writer",
                &mutation(
                    "folder",
                    "cycle",
                    1,
                    DraftChange::Move {
                        parent_id: Some("child".to_owned())
                    }
                )
            )
            .await
            .unwrap(),
        DraftResult::Invalid(_)
    ));
    assert!(matches!(
        store
            .mutate_draft_document(
                "writer",
                &mutation("folder", "trash-parent", 1, DraftChange::Trash)
            )
            .await
            .unwrap(),
        DraftResult::Invalid(_)
    ));
}

#[tokio::test]
async fn sqlite_documents_preserve_conflicts_replay_and_ownership() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[tokio::test]
#[ignore = "run with just test-postgres in a disposable database"]
async fn postgres_documents_preserve_conflicts_replay_and_ownership() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    contract(&store).await;
}

#[allow(clippy::too_many_lines)] // One migration/delete/content contract on both database backends.
async fn workspace_contract(store: &Store) {
    store.migrate().await.unwrap();
    for owner in ["alice", "bob"] {
        for (id, parent, kind, body) in [
            ("root", None, DraftKind::Folder, ""),
            ("nested", Some("root"), DraftKind::Folder, ""),
            (
                "document",
                Some("nested"),
                DraftKind::Document,
                "中文\n# saved content",
            ),
        ] {
            applied(
                store
                    .mutate_draft_document(
                        owner,
                        &mutation(id, id, 0, create(kind, "Same name", parent, body)),
                    )
                    .await
                    .unwrap(),
            );
        }
    }
    store.integrate_draft_folders().await.unwrap();
    let first = store.load_session_folders().await.unwrap();
    assert_eq!(first.len(), 4);
    for owner in ["alice", "bob"] {
        let document = store
            .draft_document(owner, "document")
            .await
            .unwrap()
            .unwrap();
        let nested = first
            .iter()
            .find(|folder| Some(&folder.id) == document.parent_id.as_ref())
            .unwrap();
        assert_eq!(nested.owner_user_id.as_deref(), Some(owner));
        assert!(nested.parent.is_some());
        assert_eq!(document.body, "中文\n# saved content");
        assert_eq!(document.body_revision, 1);
        let root = first
            .iter()
            .find(|folder| Some(&folder.id) == nested.parent.as_ref())
            .unwrap();
        store
            .replace_session_folders(Some(owner), std::slice::from_ref(root))
            .await
            .unwrap();
        let document = store
            .draft_document(owner, "document")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(document.parent_id.as_ref(), Some(&root.id));
        // A writer that began before folder deletion still owns its body clock.
        applied(
            store
                .mutate_draft_document(
                    owner,
                    &mutation(
                        "document",
                        "write-after-move",
                        1,
                        write("Written after folder deletion"),
                    ),
                )
                .await
                .unwrap(),
        );
        store
            .replace_session_folders(Some(owner), &[])
            .await
            .unwrap();
        let document = store
            .draft_document(owner, "document")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(document.parent_id, None);
        assert_eq!(document.body, "Written after folder deletion");
        assert_eq!(
            store.draft_history(owner, "document").await.unwrap().len(),
            1
        );
    }
    store.integrate_draft_folders().await.unwrap();
    assert!(store.load_session_folders().await.unwrap().is_empty());
    for owner in ["alice", "bob"] {
        assert_eq!(
            store
                .draft_document(owner, "document")
                .await
                .unwrap()
                .unwrap()
                .parent_id,
            None
        );
    }
    store
        .update_workspace_order("alice", &["draft:document".to_owned()])
        .await
        .unwrap();
    store
        .update_workspace_order("bob", &["session:other".to_owned()])
        .await
        .unwrap();
    let orders = store.load_workspace_orders().await.unwrap();
    assert_eq!(orders.len(), 2);
    assert!(orders.contains(&("alice".to_owned(), vec!["draft:document".to_owned()])));
}

#[tokio::test]
async fn sqlite_workspace_import_and_folder_deletion_preserve_content_and_owners() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    workspace_contract(&store).await;
}

#[tokio::test]
#[ignore = "run with just test-postgres in a disposable database"]
async fn postgres_workspace_import_and_folder_deletion_preserve_content_and_owners() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    workspace_contract(&store).await;
}

#[test]
fn recovery_snapshots_are_spaced_like_obsidian_file_recovery() {
    let minute = 60 * 1000;
    // The first snapshot of a document is always kept.
    assert!(keeps_snapshot(10 * minute, 10 * minute, None));
    // Continuous writing: one snapshot per interval, not per autosave.
    assert!(!keeps_snapshot(
        12 * minute,
        12 * minute - 400,
        Some(10 * minute)
    ));
    assert!(keeps_snapshot(
        16 * minute,
        16 * minute - 400,
        Some(10 * minute)
    ));
    // Resuming after a pause keeps the text as it was left.
    assert!(keeps_snapshot(60 * minute, 11 * minute, Some(10 * minute)));
}

fn create_trashed(title: &str, body: &str, source_session_id: Option<&str>) -> DraftChange {
    DraftChange::Create {
        kind: DraftKind::Document,
        title: title.into(),
        parent_id: None,
        body: body.into(),
        attachments: vec![],
        trashed: true,
        source_session_id: source_session_id.map(str::to_owned),
    }
}

#[tokio::test]
async fn draft_trash_lifecycle_keeps_legacy_json_and_pre_upgrade_create_hash() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();

    // Pre-upgrade Create omits default trashed/source_session_id so operation hashes match.
    let ordinary = mutation(
        "d-live",
        "c-live",
        0,
        create(DraftKind::Document, "Live", None, "hello"),
    );
    let serialized = serde_json::to_string(&ordinary).unwrap();
    assert!(!serialized.contains("trashed") && !serialized.contains("source_session_id"));

    let live = applied(
        store
            .mutate_draft_document("alice", &ordinary)
            .await
            .unwrap(),
    );
    assert_eq!(live.trashed_at_ms, None);
    let trashed = applied(
        store
            .mutate_draft_document(
                "alice",
                &mutation("d-live", "t-live", 1, DraftChange::Trash),
            )
            .await
            .unwrap(),
    );
    assert!(trashed.deleted && trashed.trashed_at_ms.is_some());

    // Stored JSON in draft_documents omits trashed_at_ms/source_session_id for rollback safety.
    let archived = applied(
        store
            .mutate_draft_document(
                "alice",
                &mutation(
                    "d-sess",
                    "c-sess",
                    0,
                    create_trashed("Session draft", "unsent", Some("sess-42")),
                ),
            )
            .await
            .unwrap(),
    );
    assert!(archived.deleted && archived.trashed_at_ms.is_some());
    assert_eq!(archived.source_session_id.as_deref(), Some("sess-42"));
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    let (raw_val, raw_meta): (String, String) = sqlx::query_as(
        "SELECT value, metadata FROM draft_documents WHERE owner_user_id = 'alice' AND id = 'd-sess'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(!raw_val.contains("trashed_at_ms") && !raw_val.contains("source_session_id"));
    assert!(!raw_meta.contains("trashed_at_ms") && !raw_meta.contains("source_session_id"));

    let restored = applied(
        store
            .mutate_draft_document(
                "alice",
                &mutation("d-sess", "r-sess", 1, DraftChange::Restore),
            )
            .await
            .unwrap(),
    );
    assert!(
        !restored.deleted
            && restored.trashed_at_ms.is_none()
            && restored.source_session_id.is_none()
    );
}

#[tokio::test]
async fn draft_trash_legacy_backfill_and_tombstone_replay() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();

    let exp = applied(
        store
            .mutate_draft_document(
                "alice",
                &mutation("d-old", "c-old", 0, create_trashed("Old", "old", None)),
            )
            .await
            .unwrap(),
    );
    let StorageBackend::Sqlite(db) = &store.backend else {
        unreachable!()
    };
    sqlx::query("DELETE FROM draft_document_trash_meta WHERE document_id = 'd-old'")
        .execute(&db.pool)
        .await
        .unwrap();
    let listed = store.list_trashed_drafts_for_retention().await.unwrap();
    assert_eq!(listed.len(), 1);
    let persisted_ts = listed[0].2;
    assert_eq!(
        store
            .draft_document("alice", "d-old")
            .await
            .unwrap()
            .unwrap()
            .trashed_at_ms,
        Some(persisted_ts)
    );
    assert!(
        store
            .purge_expired_draft("alice", "d-old", persisted_ts + 1)
            .await
            .unwrap()
            .is_none()
    );
    let purged_rev = store
        .purge_expired_draft("alice", "d-old", persisted_ts)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(purged_rev, exp.revision + 1);
    let replay = store
        .mutate_draft_document(
            "alice",
            &mutation("d-old", "c-old", 0, create_trashed("Old", "old", None)),
        )
        .await
        .unwrap();
    assert!(
        matches!(replay, DraftResult::Purged { id, revision } if id == "d-old" && revision == purged_rev)
    );
}

#[tokio::test]
async fn draft_trash_purge_optimistic_checks_reject_stale_or_live_drafts() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();

    let keep_change = create(DraftKind::Document, "Keep", None, "body");
    applied(
        store
            .mutate_draft_document("alice", &mutation("d-keep", "c-keep", 0, keep_change))
            .await
            .unwrap(),
    );
    let del_change = create_trashed("Trashed", "bye", Some("sess-9"));
    let del = applied(
        store
            .mutate_draft_document("alice", &mutation("d-del", "c-del", 0, del_change))
            .await
            .unwrap(),
    );
    let del_ts = del.trashed_at_ms.unwrap();

    let stale = store
        .purge_trashed_drafts(
            "alice",
            &[PurgeDraftTarget {
                id: "d-del".into(),
                expected_revision: del.revision + 1,
                expected_trashed_at_ms: del_ts,
            }],
        )
        .await
        .unwrap();
    assert!(matches!(stale, Err(PurgeDraftError::Conflict(_))));
    let live_err = store
        .purge_trashed_drafts(
            "alice",
            &[
                PurgeDraftTarget {
                    id: "d-del".into(),
                    expected_revision: del.revision,
                    expected_trashed_at_ms: del_ts,
                },
                PurgeDraftTarget {
                    id: "d-keep".into(),
                    expected_revision: 1,
                    expected_trashed_at_ms: del_ts,
                },
            ],
        )
        .await
        .unwrap();
    assert!(matches!(live_err, Err(PurgeDraftError::Conflict(_))));
    assert!(
        store
            .draft_document("alice", "d-del")
            .await
            .unwrap()
            .is_some()
    );

    let purged = store
        .purge_trashed_drafts(
            "alice",
            &[PurgeDraftTarget {
                id: "d-del".into(),
                expected_revision: del.revision,
                expected_trashed_at_ms: del_ts,
            }],
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(purged, vec![("d-del".to_owned(), del.revision + 1)]);
    assert!(
        store
            .draft_document("alice", "d-del")
            .await
            .unwrap()
            .is_none()
    );
}
