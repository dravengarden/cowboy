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

async fn contract(store: &Store) {
    store.migrate().await.unwrap();
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
