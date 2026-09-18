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
