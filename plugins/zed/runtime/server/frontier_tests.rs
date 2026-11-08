// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use fs::FakeFs;
use gpui::TestAppContext;
use language::{FakeLspAdapter, rust_lang};

fn evidence(value: u32) -> Evidence {
    Evidence {
        stamp: clock::Lamport {
            value,
            replica_id: clock::ReplicaId::LOCAL,
        },
        versioned: true,
        published: clock::Global::new(),
        saved: clock::Global::new(),
        disk_based: false,
        reused: false,
    }
}

#[test]
fn cowboy_frontier_state_is_bounded_and_forgets_the_oldest_publication() {
    let mut state = State::default();
    let server = LanguageServerId(1);
    for id in 1..=MAX_ENTRIES as u64 + 1 {
        state.record(BufferId::new(id).unwrap(), server, evidence(id as u32));
    }
    assert_eq!(state.len(), MAX_ENTRIES);
    assert!(state.get(BufferId::new(1).unwrap(), server).is_none());
    assert_eq!(
        state
            .get(BufferId::new(2).unwrap(), server)
            .unwrap()
            .stamp
            .value,
        2
    );
    // Replacing a retained key never evicts another.
    state.record(BufferId::new(2).unwrap(), server, evidence(99));
    assert_eq!(state.len(), MAX_ENTRIES);
    assert!(state.get(BufferId::new(3).unwrap(), server).is_some());
}

#[test]
fn cowboy_frontier_server_projection_bounds_names_and_marks_absent_evidence() {
    let absent = server(LanguageServerId(7), "rust-analyzer", None);
    assert_eq!(absent.server_id, 7);
    assert!(!absent.published && absent.analyzed.is_empty());
    let long = "é".repeat(100);
    let value = server(LanguageServerId(7), &long, Some(&evidence(5)));
    assert!(value.name.len() <= MAX_NAME && long.starts_with(&value.name));
    assert!(value.published && value.versioned);
    assert_eq!(value.lamport_timestamp, 5);
}

async fn fixture(
    cx: &mut TestAppContext,
) -> (
    Entity<crate::Project>,
    Entity<Buffer>,
    lsp::FakeLanguageServer,
    OpenLspBufferHandle,
) {
    cx.update(|cx| {
        if cx.try_global::<settings::SettingsStore>().is_none() {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        }
        release_channel::init(semver::Version::new(0, 0, 0), cx);
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/dir", serde_json::json!({"a.rs": "fn a() { A }\n"}))
        .await;
    let project = crate::Project::test(fs, [Path::new("/dir")], cx).await;
    let registry = project.read_with(cx, |project, _| project.languages().clone());
    registry.add(rust_lang());
    let mut servers = registry.register_fake_lsp(
        "Rust",
        FakeLspAdapter {
            disk_based_diagnostics_sources: vec!["disk".into()],
            ..Default::default()
        },
    );
    let buffer = project
        .update(cx, |project, cx| project.open_local_buffer("/dir/a.rs", cx))
        .await
        .unwrap();
    let handle = project.update(cx, |project, cx| {
        project.register_buffer_with_language_servers(&buffer, cx)
    });
    let server = servers.next().await.unwrap();
    (project, buffer, server, handle)
}

fn publish(server: &lsp::FakeLanguageServer, version: Option<i32>, source: &str) {
    server.notify::<lsp::notification::PublishDiagnostics>(lsp::PublishDiagnosticsParams {
        uri: lsp::Uri::from_file_path("/dir/a.rs").unwrap(),
        version,
        diagnostics: vec![lsp::Diagnostic {
            range: lsp::Range::new(lsp::Position::new(0, 9), lsp::Position::new(0, 10)),
            severity: Some(lsp::DiagnosticSeverity::ERROR),
            message: "undefined".into(),
            source: Some(source.into()),
            ..Default::default()
        }],
    });
}

fn observed(
    project: &Entity<crate::Project>,
    buffer: &Entity<Buffer>,
    cx: &mut TestAppContext,
) -> (Evidence, clock::Global) {
    let id = buffer.read_with(cx, |buffer, _| buffer.remote_id());
    let store = project.read_with(cx, |project, _| project.lsp_store());
    let server = store.update(cx, |store, cx| {
        buffer.update(cx, |buffer, cx| {
            store.language_servers_for_local_buffer(buffer, cx)
        })
    });
    assert_eq!(server.len(), 1);
    let evidence = store.read_with(cx, |store, _| {
        store
            .as_local()
            .unwrap()
            .cowboy_frontier
            .get(id, server[0])
            .cloned()
            .expect("publication recorded")
    });
    (evidence, buffer.read_with(cx, |buffer, _| buffer.version()))
}

#[gpui::test]
async fn cowboy_frontier_records_the_analyzed_version_of_each_publication(cx: &mut TestAppContext) {
    let (project, buffer, mut server, handle) = fixture(cx).await;
    let opened = server
        .receive_notification::<lsp::notification::DidOpenTextDocument>()
        .await
        .text_document
        .version;
    let at_open = buffer.read_with(cx, |buffer, _| buffer.version());
    // gopls omits its diagnostic version when the document version is zero.
    // Both didOpen and the retained snapshot must begin at the same nonzero ID.
    assert_eq!(opened, 1);
    publish(&server, (opened != 0).then_some(opened), "rustc");
    cx.executor().run_until_parked();
    let (initial, current) = observed(&project, &buffer, cx);
    assert!(initial.versioned);
    assert_eq!(initial.published, current);
    buffer.update(cx, |buffer, cx| buffer.edit([(0..0, "\n")], None, cx));
    let changed = server
        .receive_notification::<lsp::notification::DidChangeTextDocument>()
        .await
        .text_document
        .version;

    // A versioned publication for the opened text: the analyzed version is
    // the older snapshot, so the current text is newer than the evidence.
    publish(&server, Some(opened), "rustc");
    cx.executor().run_until_parked();
    let (evidence, current) = observed(&project, &buffer, cx);
    assert!(evidence.versioned && !evidence.disk_based && !evidence.reused);
    assert_eq!(evidence.published, at_open);
    assert_ne!(evidence.published, current);
    let first = evidence.stamp;

    // A versioned publication of the current text matches it exactly.
    publish(&server, Some(changed), "rustc");
    cx.executor().run_until_parked();
    let (evidence, current) = observed(&project, &buffer, cx);
    assert_eq!(evidence.published, current);
    assert!(evidence.stamp > first);

    // Unversioned: bounded by the text at publication, never exact.
    publish(&server, None, "rustc");
    cx.executor().run_until_parked();
    let (evidence, current) = observed(&project, &buffer, cx);
    assert!(!evidence.versioned);
    assert_eq!(evidence.published, current);

    // Save-triggered analyzers report disk-based diagnostics.
    publish(&server, None, "disk");
    cx.executor().run_until_parked();
    let (evidence, _) = observed(&project, &buffer, cx);
    assert!(evidence.disk_based);
    drop(handle);
    cx.executor().run_until_parked();
}

#[gpui::test]
async fn cowboy_frontier_attributes_pulled_diagnostics_to_the_version_pulled(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        if cx.try_global::<settings::SettingsStore>().is_none() {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        }
        release_channel::init(semver::Version::new(0, 0, 0), cx);
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/dir", serde_json::json!({"a.rs": "fn a() { A }\n"}))
        .await;
    let project = crate::Project::test(fs, [Path::new("/dir")], cx).await;
    let registry = project.read_with(cx, |project, _| project.languages().clone());
    registry.add(rust_lang());
    let mut servers = registry.register_fake_lsp(
        "Rust",
        FakeLspAdapter {
            capabilities: lsp::ServerCapabilities {
                diagnostic_provider: Some(lsp::DiagnosticServerCapabilities::Options(
                    lsp::DiagnosticOptions {
                        identifier: Some("cowboy-pull".into()),
                        inter_file_dependencies: false,
                        workspace_diagnostics: false,
                        work_done_progress_options: Default::default(),
                    },
                )),
                ..Default::default()
            },
            initializer: Some(Box::new(|server| {
                server.set_request_handler::<lsp::request::DocumentDiagnosticRequest, _, _>(
                    |_, _| async move {
                        Ok(lsp::DocumentDiagnosticReportResult::Report(
                            lsp::DocumentDiagnosticReport::Full(
                                lsp::RelatedFullDocumentDiagnosticReport {
                                    related_documents: None,
                                    full_document_diagnostic_report:
                                        lsp::FullDocumentDiagnosticReport {
                                            result_id: None,
                                            items: vec![lsp::Diagnostic {
                                                range: lsp::Range::new(
                                                    lsp::Position::new(0, 9),
                                                    lsp::Position::new(0, 10),
                                                ),
                                                severity: Some(lsp::DiagnosticSeverity::ERROR),
                                                message: "undefined".into(),
                                                ..Default::default()
                                            }],
                                        },
                                },
                            ),
                        ))
                    },
                );
            })),
            ..Default::default()
        },
    );
    let buffer = project
        .update(cx, |project, cx| project.open_local_buffer("/dir/a.rs", cx))
        .await
        .unwrap();
    let handle = project.update(cx, |project, cx| {
        project.register_buffer_with_language_servers(&buffer, cx)
    });
    let _server = servers.next().await.unwrap();
    cx.executor().run_until_parked();
    let store = project.read_with(cx, |project, _| project.lsp_store());
    for edit in [None, Some("\n")] {
        if let Some(text) = edit {
            buffer.update(cx, |buffer, cx| buffer.edit([(0..0, text)], None, cx));
            cx.executor().run_until_parked();
        }
        store
            .update(cx, |store, cx| {
                store.pull_diagnostics_for_buffer(buffer.clone(), cx)
            })
            .await
            .unwrap();
        cx.executor().run_until_parked();
        let (evidence, current) = observed(&project, &buffer, cx);
        assert!(
            evidence.versioned,
            "a pull names the version it was answered for"
        );
        assert_eq!(evidence.published, current);
    }
    drop(handle);
    cx.executor().run_until_parked();
}

#[gpui::test]
async fn cowboy_frontier_zero_without_a_snapshot_is_not_versioned(cx: &mut TestAppContext) {
    let (project, buffer, mut server, handle) = fixture(cx).await;
    server
        .receive_notification::<lsp::notification::DidOpenTextDocument>()
        .await;
    let store = project.read_with(cx, |project, _| project.lsp_store());
    store.update(cx, |store, cx| {
        store
            .as_local_mut()
            .unwrap()
            .buffer_snapshots
            .remove(&buffer.read(cx).remote_id());
    });
    publish(&server, Some(0), "rustc");
    cx.executor().run_until_parked();
    let (evidence, current) = observed(&project, &buffer, cx);
    assert!(
        !evidence.versioned,
        "the version-zero fallback has no exact snapshot"
    );
    assert_eq!(evidence.published, current);
    drop(handle);
    cx.executor().run_until_parked();
}
