//! Image sources on the widget write path (docs/widgets.md §5.4, §6.3). Before
//! a document is stored, every `artifact` source must name an existing
//! artifact, which is pinned against collection, and every `event` source is
//! rewritten to the `artifact` of that image block of this session's event.
//! The stored notice event then carries the artifact URL, which the artifact
//! collector already treats as a root for as long as the row exists.
use super::*;
use crate::widgets as rules;
use base64::Engine as _;

const EVENT_MIMES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

fn refusal(path: &str, message: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":"invalid_widget","path":path,"message":message}})
}

/// The `index`-th image block of an event payload, as an artifact URL; an
/// inline block (live events keep their bytes) is stored first.
fn event_image(
    store: &cowboy::artifacts::ArtifactStore,
    payload: &Value,
    index: usize,
) -> Result<Option<String>, &'static str> {
    let mut blocks = Vec::new();
    collect_image_blocks(payload, &mut blocks);
    let Some(block) = blocks.get(index) else {
        return Ok(None);
    };
    if let Some(url) = block.get("url").and_then(Value::as_str) {
        let pinned = rules::artifact_name(url)
            .map(|name| store.pin(name))
            .transpose()
            .map_err(|_| "unavailable")?;
        return Ok(pinned.unwrap_or(false).then(|| url.to_owned()));
    }
    let inline = block
        .get("source")
        .filter(|source| source.get("data").is_some())
        .unwrap_or(block);
    let (Some(data), Some(mime)) = (
        inline.get("data").and_then(Value::as_str),
        inline
            .get("mimeType")
            .or_else(|| inline.get("media_type"))
            .and_then(Value::as_str)
            .or(Some("image/png")),
    ) else {
        return Ok(None);
    };
    if !EVENT_MIMES.contains(&mime)
        || data.len() > crate::managed_calls::media::MAX_IMAGE_BYTES.div_ceil(3) * 4
    {
        return Ok(None);
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
        return Ok(None);
    };
    store
        .put_pinned(&bytes, mime)
        .map(Some)
        .map_err(|_| "unavailable")
}

fn collect_image_blocks<'v>(value: &'v Value, out: &mut Vec<&'v Value>) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("image") {
                out.push(value);
                return;
            }
            for child in map.values() {
                collect_image_blocks(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_image_blocks(item, out);
            }
        }
        _ => {}
    }
}

/// The payload of this session's event at `seq`, from memory or storage.
async fn event_payload(
    state: &AppState,
    meta: &SessionMeta,
    seq: u64,
) -> Result<Option<Value>, &'static str> {
    let owner = meta.owner_user_id.clone();
    let visible = |session: &SessionMeta| !session.system && session.owner_user_id == owner;
    let mut events = state
        .hub
        .log_window(&meta.id, None, Some(seq + 1), 1, visible)
        .map(|(events, _)| events)
        .unwrap_or_default();
    events.retain(|event| event.seq == seq);
    if events.is_empty()
        && let Some(store) = state.store.as_ref()
    {
        events = store
            .history_page(&meta.id, seq + 1, 1)
            .await
            .map_err(|_| "unavailable")?
            .0;
        events.retain(|event| event.seq == seq);
    }
    events
        .into_iter()
        .next()
        .map(|event| serde_json::to_value(event.event).map_err(|_| "unavailable"))
        .transpose()
}

/// Pixels of a resolved app image, decoded completely (docs/widget-apps.md
/// §6): `Err(message)` when it is not a still PNG within the app bounds.
fn app_image_pixels(
    store: &cowboy::artifacts::ArtifactStore,
    source: &Value,
) -> Result<u64, &'static str> {
    let bytes = match source["kind"].as_str() {
        Some("data") => base64::engine::general_purpose::STANDARD
            .decode(source["base64"].as_str().unwrap_or_default())
            .map_err(|_| "not a valid PNG")?,
        _ => {
            let name = source["url"]
                .as_str()
                .and_then(rules::artifact_name)
                .ok_or("apps take PNG artifacts only")?;
            if !name.ends_with(".png") {
                return Err("apps take PNG artifacts only");
            }
            let path = store.path(name).ok_or("no such artifact")?;
            let size = std::fs::metadata(&path)
                .map_err(|_| "no such artifact")?
                .len();
            if size > rules::APP_IMAGE_BYTES as u64 {
                return Err("larger than 1 MiB");
            }
            std::fs::read(path).map_err(|_| "no such artifact")?
        }
    };
    rules::app_png(&bytes)
}

/// Verify, pin and resolve every image source of `document` in place.
/// `Ok(Some(refusal))` names the first source that cannot be used.
pub(super) async fn resolve(
    state: &AppState,
    meta: &SessionMeta,
    document: &mut Value,
) -> Result<Option<Value>, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?.artifacts();
    let mut app_pixels = 0_u64;
    for rules::SourceAt {
        pointer,
        path,
        source,
        app,
    } in rules::image_sources(document)
    {
        let url = match source["kind"].as_str() {
            Some("artifact") => {
                let url = source["url"].as_str().unwrap_or_default();
                let name = rules::artifact_name(url).unwrap_or_default();
                if !store.pin(name).map_err(|_| "unavailable")? {
                    return Ok(Some(refusal(&format!("{path}.url"), "no such artifact")));
                }
                if app {
                    match app_image_pixels(&store, &source) {
                        Ok(pixels) => app_pixels += pixels,
                        Err(message) => return Ok(Some(refusal(&path, message))),
                    }
                }
                continue;
            }
            Some("event") => {
                let seq = source["seq"].as_u64().unwrap_or(u64::MAX);
                let index = source["index"].as_u64().unwrap_or(0);
                let image = match event_payload(state, meta, seq).await? {
                    Some(payload) => event_image(
                        &store,
                        &payload,
                        usize::try_from(index).unwrap_or(usize::MAX),
                    )?,
                    None => None,
                };
                let Some(url) = image else {
                    return Ok(Some(refusal(
                        &path,
                        &format!("event {seq} of this session has no image {index}"),
                    )));
                };
                url
            }
            _ => {
                if app {
                    match app_image_pixels(&store, &source) {
                        Ok(pixels) => app_pixels += pixels,
                        Err(message) => return Ok(Some(refusal(&path, message))),
                    }
                }
                continue;
            }
        };
        if app {
            match app_image_pixels(&store, &json!({"kind":"artifact","url":url})) {
                Ok(pixels) => app_pixels += pixels,
                Err(message) => return Ok(Some(refusal(&path, message))),
            }
        }
        if let Some(slot) = document.pointer_mut(&pointer) {
            *slot = json!({"kind":"artifact","url":url});
        }
    }
    if app_pixels > rules::APP_PIXELS {
        return Ok(Some(refusal(
            "root",
            "an app's images decode to more than 16 megapixels together",
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_images_resolve_to_pinned_artifacts() {
        let root =
            std::env::temp_dir().join(format!("cowboy-widget-images-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = cowboy::artifacts::ArtifactStore::new(root.clone()).unwrap();
        let stored = store.put_bytes(b"first", "image/png").unwrap();
        let inline = base64::engine::general_purpose::STANDARD.encode(b"second");
        let payload = json!({"update":{"sessionUpdate":"tool_call_update","content":[
            {"type":"content","content":{"type":"image","url":stored,"mimeType":"image/png"}},
            {"type":"content","content":{"type":"image","data":inline,"mimeType":"image/jpeg"}},
            {"type":"content","content":{"type":"image","data":"AAAA","mimeType":"text/html"}}]}});
        assert_eq!(
            event_image(&store, &payload, 0).unwrap(),
            Some(stored.clone())
        );
        let second = event_image(&store, &payload, 1).unwrap().unwrap();
        assert!(second.ends_with(".jpg"));
        assert!(
            root.join(second.trim_start_matches("/api/artifacts/"))
                .is_file()
        );
        assert_eq!(
            event_image(&store, &payload, 2).unwrap(),
            None,
            "not an image type"
        );
        assert_eq!(
            event_image(&store, &payload, 3).unwrap(),
            None,
            "no such block"
        );
        std::fs::remove_file(root.join(stored.trim_start_matches("/api/artifacts/"))).unwrap();
        assert_eq!(
            event_image(&store, &payload, 0).unwrap(),
            None,
            "a vanished artifact"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
