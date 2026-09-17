use super::*;
use crate::coordinates::tests::{peer, wire};
use serde_json::{Value, json};

fn owner(n: u64) -> LeaseRef {
    serde_json::from_value(json!({"instance":"a".repeat(32), "id":format!("{n:016x}")})).unwrap()
}

fn share(cache: &mut Cache, text: &str, complete: bool) {
    for variant in [
        proto::create_buffer_for_peer::Variant::State(proto::BufferState {
            id: 7,
            base_text: text.into(),
            ..Default::default()
        }),
        proto::create_buffer_for_peer::Variant::Chunk(proto::BufferChunk {
            buffer_id: 7,
            is_last: complete,
            ..Default::default()
        }),
    ] {
        cache.observe(&proto::envelope::Payload::CreateBufferForPeer(
            proto::CreateBufferForPeer {
                variant: Some(variant),
                ..Default::default()
            },
        ));
    }
}

fn page(cache: &Cache, owner: &LeaseRef, content: &Content, request: &Page) -> Value {
    serde_json::to_value(cache.text_read(7, owner, content, request).unwrap()).unwrap()
}

#[test]
fn exact_unicode_text_uses_the_shared_wire() {
    let fixture: Value = serde_json::from_str(include_str!("../../fixtures/text.json")).unwrap();
    let content: Content = serde_json::from_value(fixture["request"]["content"].clone()).unwrap();
    let request: Page = serde_json::from_value(fixture["request"]["page"].clone()).unwrap();
    let mut cache = Cache::default();
    share(
        &mut cache,
        fixture["response"]["result"]["result"]["text"]
            .as_str()
            .unwrap(),
        true,
    );
    assert_eq!(cache.content(7).unwrap(), content);
    let actual = page(&cache, &owner(1), &content, &request);
    let mut expected = fixture["response"]["result"]["result"].clone();
    assert_eq!(actual["snapshot"].as_str().unwrap().len(), 64);
    expected["snapshot"] = actual["snapshot"].clone();
    assert_eq!(actual, expected);
}

#[test]
fn pages_clip_utf8_and_preserve_nuls_boms_empty_and_maximum_content() {
    for text in [
        String::new(),
        "\u{feff}\0\n".into(),
        format!("{}🙂z\n", "a".repeat(MAX_PAGE_BYTES - 1)),
        "\0".repeat(crate::coordinates::MAX_HISTORY),
    ] {
        let mut cache = Cache::default();
        share(&mut cache, &text, true);
        let content = cache.content(7).unwrap();
        let mut request = Page::Start {};
        let mut actual = String::new();
        let mut count = 0;
        loop {
            let value = page(&cache, &owner(1), &content, &request);
            assert_eq!(value["offset"], actual.len());
            let part = value["text"].as_str().unwrap();
            assert!(part.len() <= MAX_PAGE_BYTES);
            actual.push_str(part);
            count += 1;
            assert!(count <= 65);
            if value["nextOffset"].is_null() {
                break;
            }
            request = Page::Continue {
                offset: value["nextOffset"].as_u64().unwrap().try_into().unwrap(),
                snapshot: value["snapshot"].as_str().unwrap().into(),
            };
        }
        assert_eq!(actual, text);
    }
}

#[test]
fn continuations_refuse_owner_replacement_and_edit_undo_aba_without_state_allocation() {
    let base = format!("{}🙂\n", "a".repeat(MAX_PAGE_BYTES - 1));
    let mut cache = Cache::default();
    share(&mut cache, &base, true);
    let content = cache.content(7).unwrap();
    let first = page(&cache, &owner(1), &content, &Page::Start {});
    let request = Page::Continue {
        offset: (MAX_PAGE_BYTES - 1).try_into().unwrap(),
        snapshot: first["snapshot"].as_str().unwrap().into(),
    };
    assert_eq!(page(&cache, &owner(1), &content, &request)["text"], "🙂\n");
    assert_eq!(page(&cache, &owner(2), &content, &request)["kind"], "stale");
    let mut source = peer(&base, 1);
    let edit = source.edit([(0..0, "changed")]);
    source.finalize_last_transaction();
    let undo = source.undo().unwrap().1;
    let update = |cache: &mut Cache, operation: &text::Operation| {
        cache.observe(&proto::envelope::Payload::UpdateBuffer(
            proto::UpdateBuffer {
                buffer_id: 7,
                operations: vec![wire(operation)],
                ..Default::default()
            },
        ));
    };
    update(&mut cache, &edit);
    assert_eq!(
        page(&cache, &owner(1), &content, &request),
        json!({"kind":"mismatch"})
    );
    update(&mut cache, &undo);
    assert_eq!(cache.content(7).unwrap(), content);
    assert_eq!(
        page(&cache, &owner(1), &content, &request),
        json!({"kind":"stale"})
    );
    share(&mut cache, &base, true);
    assert_eq!(
        page(&cache, &owner(1), &content, &request),
        json!({"kind":"stale"})
    );
    assert_eq!(
        page(&cache, &owner(1), &content, &Page::Start {})["kind"],
        "page"
    );
}

#[test]
fn malformed_offsets_and_unavailable_history_never_read_a_path() {
    let mut cache = Cache::default();
    share(&mut cache, "a🙂z\n", true);
    let content = cache.content(7).unwrap();
    let first = page(&cache, &owner(1), &content, &Page::Start {});
    for offset in [0, 2, 3, 4, 7, u32::MAX] {
        let request = Page::Continue {
            offset,
            snapshot: first["snapshot"].as_str().unwrap().into(),
        };
        assert!(cache.text_read(7, &owner(1), &content, &request).is_err());
    }
    share(&mut cache, "a🙂z\n", false);
    assert!(
        cache
            .text_read(7, &owner(1), &content, &Page::Start {})
            .is_err()
    );
    cache.remove(7);
    assert!(
        cache
            .text_read(7, &owner(1), &content, &Page::Start {})
            .is_err()
    );
}
