// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn location(index: usize) -> Unresolved {
    Unresolved {
        origin: None,
        uri: format!("file:///workspace/target-{index}.rs")
            .parse()
            .unwrap(),
        range: lsp::Range::default(),
    }
}
fn batch(server: u64, count: usize, distinct: usize) -> (LanguageServerId, Vec<Unresolved>) {
    (
        LanguageServerId::from_proto(server),
        (0..count).map(|i| location(i % distinct)).collect(),
    )
}

#[test]
fn cowboy_navigation_budgets_span_every_server_without_truncation() {
    let root = Path::new("/workspace");
    assert_eq!(
        plan(root, vec![batch(1, 128, 32), batch(2, 128, 32)])
            .unwrap()
            .iter()
            .map(|(_, v)| v.len())
            .sum::<usize>(),
        MAX_LOCATIONS
    );
    assert!(matches!(
        plan(root, vec![batch(1, 128, 1), batch(2, 129, 1)]),
        Err(Refusal::Budget)
    ));
    let mut last = batch(2, 1, 1);
    last.1[0] = location(32);
    assert!(matches!(
        plan(root, vec![batch(1, 32, 32), last]),
        Err(Refusal::Budget)
    ));
    assert!(plan(root, (1..=4).map(|id| batch(id, 0, 1)).collect()).is_ok());
    assert!(matches!(
        plan(root, (1..=5).map(|id| batch(id, 0, 1)).collect()),
        Err(Refusal::Budget)
    ));
    assert!(matches!(
        Some(
            (0..257)
                .map(|i| lsp::Location {
                    uri: location(i).uri,
                    range: lsp::Range::default()
                })
                .collect::<Vec<_>>()
        )
        .locations(),
        Err(Refusal::Budget)
    ));
}

#[test]
fn cowboy_navigation_rejects_external_and_nonfile_targets_before_acquisition() {
    for uri in [
        "file:///elsewhere/private",
        "file:///workspace-other/file",
        "file:///workspace",
        "zip:///workspace/file",
        "https://example.invalid/workspace/file",
        "file:///workspace/../elsewhere/file",
        "file:///workspace/a%00b",
    ] {
        let mut value = location(0);
        value.uri = uri.parse().unwrap();
        assert!(
            matches!(
                plan(
                    Path::new("/workspace"),
                    vec![(LanguageServerId::from_proto(1), vec![location(0), value])]
                ),
                Err(Refusal::Target)
            ),
            "{uri}"
        );
    }
    let mut value = location(0);
    value.range.start.line = 1;
    assert!(matches!(
        plan(
            Path::new("/workspace"),
            vec![(LanguageServerId::from_proto(1), vec![value])]
        ),
        Err(Refusal::Target)
    ));
}

#[test]
fn cowboy_navigation_admission_is_single_and_released_by_owned_guard() {
    let state = State::default();
    let first = state.admit().unwrap();
    assert!(matches!(state.admit(), Err(Refusal::Budget)));
    drop(first);
    assert!(state.admit().is_ok());
}

#[gpui::test]
async fn cowboy_navigation_two_real_request_handlers_refuse_before_any_target_open(
    cx: &mut gpui::TestAppContext,
) {
    use fs::FakeFs;
    use language::{FakeLspAdapter, rust_lang};
    use std::sync::atomic::AtomicUsize;
    cx.update(|cx| {
        let settings = settings::SettingsStore::test(cx);
        cx.set_global(settings);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        "/workspace",
        serde_json::json!({"source.rs": "fn source() {}", "target.rs":"🙂x\n"}),
    )
    .await;
    let project = Project::test(fs, [Path::new("/workspace")], cx).await;
    let languages = project.read_with(cx, |project, _| project.languages().clone());
    languages.add(rust_lang());
    let mut first = languages.register_fake_lsp(
        "Rust",
        FakeLspAdapter {
            name: "cowboy-first",
            ..Default::default()
        },
    );
    let mut second = languages.register_fake_lsp(
        "Rust",
        FakeLspAdapter {
            name: "cowboy-second",
            ..Default::default()
        },
    );
    let (buffer, _handle) = project
        .update(cx, |project, cx| {
            project.open_local_buffer_with_lsp(Path::new("/workspace/source.rs"), cx)
        })
        .await
        .unwrap();
    let first = first.next().await.unwrap();
    let second = second.next().await.unwrap();
    cx.run_until_parked();
    let mode = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let servers = [first, second];
    for (index, server) in servers.iter().enumerate() {
        let second = index == 1;
        let mode = mode.clone();
        let calls = calls.clone();
        server.set_request_handler::<lsp::request::GotoDefinition, _, _>(move |_, _| {
            let case = mode.load(Ordering::SeqCst);
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if second && case == 1 {
                    return Err(anyhow!("content modified"));
                }
                let count = if second && case == 0 { 129 } else { 128 };
                Ok(Some(lsp::GotoDefinitionResponse::Array(
                    (0..count)
                        .map(|_| lsp::Location {
                            uri: "file:///workspace/target.rs".parse().unwrap(),
                            range: lsp::Range::new(
                                lsp::Position::new(0, if case == 3 { 1 } else { 0 }),
                                lsp::Position::new(0, 2),
                            ),
                        })
                        .collect(),
                )))
            }
        });
    }
    let store = project.read_with(cx, |project, _| project.lsp_store());
    store
        .update(cx, |store, cx| {
            store.buffer_store.update(cx, |store, cx| {
                store.create_buffer_for_peer(&buffer, PeerId::default(), cx)
            })
        })
        .await
        .unwrap();
    for (case, reason, expected_opens) in [
        (0, Some(Refusal::Budget), 0),
        (1, Some(Refusal::LanguageServer), 0),
        (2, None, 1),
        (3, Some(Refusal::Target), 2),
    ] {
        mode.store(case, Ordering::SeqCst);
        let request = buffer.read_with(cx, |buffer, _| {
            GetDefinitions {
                position: PointUtf16::new(0, 0),
            }
            .to_proto(proto::REMOTE_SERVER_PROJECT_ID, buffer)
        });
        let result = LspStore::cowboy_navigate::<GetDefinitions>(
            store.clone(),
            request,
            PeerId::default(),
            &mut cx.to_async(),
        )
        .await;
        if let Some(reason) = reason {
            assert_eq!(result.unwrap_err(), reason);
        } else {
            let result = proto::LspQueryResponse::decode(result.unwrap().as_slice()).unwrap();
            assert_eq!(result.responses.len(), 2);
            assert!(result.responses.iter().all(|response| matches!(&response.response, Some(proto::lsp_response::Response::GetDefinitionResponse(value)) if value.links.len() == 128)));
        }
        assert_eq!(
            store.read_with(cx, |store, _| store.cowboy_navigation.1),
            expected_opens
        );
    }
    // The all-success/over-budget cases must actually use both language servers.
    assert!(calls.load(Ordering::SeqCst) >= 7);
    let calls_before = calls.load(Ordering::SeqCst);
    store.update(cx, |store, cx| {
        store.buffer_store.update(cx, |store, _| {
            store.forget_shared_buffers_for(&PeerId::default())
        })
    });
    let request = buffer.read_with(cx, |buffer, _| {
        GetDefinitions {
            position: PointUtf16::new(0, 0),
        }
        .to_proto(proto::REMOTE_SERVER_PROJECT_ID, buffer)
    });
    let result = LspStore::cowboy_navigate::<GetDefinitions>(
        store.clone(),
        request,
        PeerId::default(),
        &mut cx.to_async(),
    )
    .await;
    assert_eq!(result.unwrap_err(), Refusal::Source);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        calls_before,
        "lost source ownership dispatched language queries"
    );
    assert_eq!(store.read_with(cx, |store, _| store.cowboy_navigation.1), 2);
}
