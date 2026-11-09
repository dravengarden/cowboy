//! Image sources of a widget document (docs/widgets.md §5.4). Validation is
//! offline: an `artifact` URL must name a well-formed artifact, an `event`
//! source a non-negative sequence, and a `data` image must decode completely as
//! a small still PNG. Whether an artifact exists, and which artifact an event
//! source names, is decided by the Controller's write path, which rewrites
//! every `event` source to an `artifact` before storing.
use super::{Checked, Invalid, object};
use base64::Engine as _;
use serde_json::Value;

/// Encoded bytes of a `data` image after base64 decoding.
pub const INLINE_IMAGE_BYTES: usize = 8 * 1024;
/// Largest intrinsic width and height of a `data` image.
pub const INLINE_IMAGE_PIXELS: u32 = 256;
pub const ARTIFACT_PREFIX: &str = "/api/artifacts/";
/// Encoded bytes of one image an app draws (docs/widget-apps.md §6).
pub const APP_IMAGE_BYTES: usize = 1024 * 1024;
/// Largest intrinsic width and height of an app image.
pub const APP_IMAGE_PIXELS: u32 = 2048;
/// Decoded pixels across every image of one app.
pub const APP_PIXELS: u64 = 16 * 1024 * 1024;
/// Image blocks an `event` source may index into.
const EVENT_IMAGES: u64 = 64;

/// The artifact name a source URL names, when it is well formed.
#[must_use]
pub fn artifact_name(url: &str) -> Option<&str> {
    url.strip_prefix(ARTIFACT_PREFIX)
        .filter(|name| crate::artifacts::valid_artifact_name(name))
}

pub(super) fn image_source(value: Option<&Value>, path: &str) -> Checked {
    let map = object(value.ok_or_else(|| Invalid::at(path, "required"))?, path)?;
    let field = |name: &str| format!("{path}.{name}");
    match map.get("kind").and_then(Value::as_str) {
        Some("artifact") => map
            .get("url")
            .and_then(Value::as_str)
            .and_then(artifact_name)
            .map(|_| ())
            .ok_or_else(|| {
                Invalid::at(
                    &field("url"),
                    "expected /api/artifacts/<sha256>.<png|jpg|webp|gif|avif>",
                )
            }),
        Some("event") => {
            if !map
                .get("seq")
                .and_then(Value::as_u64)
                .is_some_and(|seq| seq < i64::MAX as u64)
            {
                return Err(Invalid::at(&field("seq"), "expected an integer ≥ 0"));
            }
            if map
                .get("index")
                .is_some_and(|index| !index.as_u64().is_some_and(|i| i < EVENT_IMAGES))
            {
                return Err(Invalid::at(&field("index"), "expected an integer 0..63"));
            }
            Ok(())
        }
        Some("data") => {
            if map.get("mime").and_then(Value::as_str) != Some("image/png") {
                return Err(Invalid::at(
                    &field("mime"),
                    "data images are image/png; publish larger or other images as artifacts",
                ));
            }
            let at = field("base64");
            let encoded = map
                .get("base64")
                .and_then(Value::as_str)
                .ok_or_else(|| Invalid::at(&at, "expected base64 text"))?;
            if encoded.len() > INLINE_IMAGE_BYTES.div_ceil(3) * 4 {
                return Err(Invalid::at(&at, "larger than 8 KiB"));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| Invalid::at(&at, "expected base64 text"))?;
            still_png(&bytes, INLINE_IMAGE_BYTES, INLINE_IMAGE_PIXELS)
                .map(|_| ())
                .map_err(|message| Invalid::at(&at, message))
        }
        _ => Err(Invalid::at(
            &field("kind"),
            "expected one of artifact, event, data",
        )),
    }
}

/// An image an app draws: a still PNG of at most 1 MiB and 2048 × 2048,
/// decoded completely. Returns its pixel count.
pub fn app_png(bytes: &[u8]) -> Result<u64, &'static str> {
    still_png(bytes, APP_IMAGE_BYTES, APP_IMAGE_PIXELS)
}

/// Decode the whole PNG, checksums included, within a memory bound derived
/// from the largest accepted side. Returns the pixel count.
fn still_png(bytes: &[u8], max_bytes: usize, max_side: u32) -> Result<u64, &'static str> {
    if bytes.is_empty() || bytes.len() > max_bytes {
        return Err(if max_bytes == INLINE_IMAGE_BYTES {
            "larger than 8 KiB"
        } else {
            "larger than 1 MiB"
        });
    }
    let side = max_side as usize;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: side * side * 8,
    });
    let mut reader = decoder.read_info().map_err(|_| "not a valid PNG")?;
    let info = reader.info();
    if info.animation_control.is_some() {
        return Err("animated PNG is not accepted");
    }
    let (width, height) = (info.width, info.height);
    if width == 0 || height == 0 || width > max_side || height > max_side {
        return Err(if max_side == INLINE_IMAGE_PIXELS {
            "larger than 256 × 256"
        } else {
            "larger than 2048 × 2048"
        });
    }
    let size = reader.output_buffer_size().ok_or("not a valid PNG")?;
    let mut buffer = vec![0; size];
    reader
        .next_frame(&mut buffer)
        .map_err(|_| "not a valid PNG")?;
    reader.finish().map_err(|_| "not a valid PNG")?;
    Ok(u64::from(width) * u64::from(height))
}

/// One image source of a document.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceAt {
    /// JSON Pointer to the source object, for rewriting it in place.
    pub pointer: String,
    /// The path the agent wrote (`root.children[0].src`), for refusals.
    pub path: String,
    pub source: Value,
    /// An image of an app's `images`: it must be a still PNG within the app
    /// bounds once resolved.
    pub app: bool,
}

/// Every image source of a (validated) document, in document order.
#[must_use]
pub fn image_sources(document: &Value) -> Vec<SourceAt> {
    let mut out = Vec::new();
    walk(document, "", "", &mut out);
    out
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn walk(value: &Value, pointer: &str, path: &str, out: &mut Vec<SourceAt>) {
    match value {
        Value::Object(map) => {
            match map.get("type").and_then(Value::as_str) {
                Some("image") => {
                    if let Some(source) = map.get("src") {
                        out.push(SourceAt {
                            pointer: format!("{pointer}/src"),
                            path: join(path, "src"),
                            source: source.clone(),
                            app: false,
                        });
                    }
                }
                Some("gallery") => {
                    for (index, image) in map
                        .get("images")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        if let Some(source) = image.get("src") {
                            out.push(SourceAt {
                                pointer: format!("{pointer}/images/{index}/src"),
                                path: format!("{}[{index}].src", join(path, "images")),
                                source: source.clone(),
                                app: false,
                            });
                        }
                    }
                }
                Some("app") => {
                    for (key, source) in map
                        .get("images")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                    {
                        let escaped = key.replace('~', "~0").replace('/', "~1");
                        out.push(SourceAt {
                            pointer: format!("{pointer}/images/{escaped}"),
                            path: format!("{}.{key}", join(path, "images")),
                            source: source.clone(),
                            app: true,
                        });
                    }
                }
                _ => {}
            }
            // An app's `props` is opaque data, never widget nodes.
            let app = map.get("type").and_then(Value::as_str) == Some("app");
            for (key, child) in map {
                if key != "src"
                    && key != "images"
                    && !(app && key == "props")
                    && (child.is_object() || child.is_array())
                {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    walk(
                        child,
                        &format!("{pointer}/{escaped}"),
                        &join(path, key),
                        out,
                    );
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(
                    item,
                    &format!("{pointer}/{index}"),
                    &format!("{path}[{index}]"),
                    out,
                );
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png(width: u32, height: u32) -> String {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![0; (width * height) as usize])
            .unwrap();
        writer.finish().unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn sources_are_checked_offline() {
        let artifact =
            json!({"kind":"artifact","url":format!("/api/artifacts/{}.png", "a".repeat(64))});
        image_source(Some(&artifact), "src").unwrap();
        let remote = json!({"kind":"artifact","url":"https://example.com/x.png"});
        assert_eq!(
            image_source(Some(&remote), "src").unwrap_err().path,
            "src.url"
        );
        image_source(Some(&json!({"kind":"event","seq":12,"index":1})), "src").unwrap();
        let negative = json!({"kind":"event","seq":-1});
        assert_eq!(
            image_source(Some(&negative), "src").unwrap_err().path,
            "src.seq"
        );
        let small = json!({"kind":"data","mime":"image/png","base64":png(16, 16)});
        image_source(Some(&small), "src").unwrap();
        let big = json!({"kind":"data","mime":"image/png","base64":png(300, 4)});
        assert_eq!(
            image_source(Some(&big), "src").unwrap_err().message,
            "larger than 256 × 256"
        );
        let jpeg = json!({"kind":"data","mime":"image/jpeg","base64":"AAAA"});
        assert_eq!(
            image_source(Some(&jpeg), "src").unwrap_err().path,
            "src.mime"
        );
        let junk = json!({"kind":"data","mime":"image/png","base64":"aGVsbG8="});
        assert_eq!(
            image_source(Some(&junk), "src").unwrap_err().message,
            "not a valid PNG"
        );
    }

    #[test]
    fn sources_are_listed_with_pointer_and_path() {
        let document = json!({"root":{"type":"stack","children":[
            {"type":"image","alt":"a","src":{"kind":"event","seq":3}},
            {"type":"gallery","images":[{"alt":"b","src":{"kind":"event","seq":4,"index":1}}]}]}});
        let sources = image_sources(&document);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].pointer, "/root/children/0/src");
        assert_eq!(sources[0].path, "root.children[0].src");
        assert_eq!(sources[1].pointer, "/root/children/1/images/0/src");
        assert_eq!(sources[1].path, "root.children[1].images[0].src");
        assert_eq!(
            document.pointer(&sources[1].pointer),
            Some(&sources[1].source)
        );
    }

    #[test]
    fn app_images_are_listed_by_key_and_bounded() {
        let document = json!({"root":{"type":"app","id":"a","images":{
            "logo":{"kind":"event","seq":5},"a/b":{"kind":"event","seq":6}},
            "props":{"type":"image","src":{"kind":"event","seq":7}}}});
        let sources = image_sources(&document);
        assert_eq!(sources.len(), 2, "props stay opaque");
        assert!(sources.iter().all(|source| source.app));
        let escaped = sources
            .iter()
            .find(|s| s.path == "root.images.a/b")
            .unwrap();
        assert_eq!(escaped.pointer, "/root/images/a~1b");
        assert_eq!(document.pointer(&escaped.pointer), Some(&escaped.source));

        let decode = |text: String| {
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .unwrap()
        };
        assert_eq!(app_png(&decode(png(300, 200))), Ok(60_000));
        assert_eq!(
            app_png(&decode(png(2049, 1))),
            Err("larger than 2048 × 2048")
        );
        assert_eq!(app_png(b"not a png"), Err("not a valid PNG"));
        assert_eq!(
            app_png(&vec![0; APP_IMAGE_BYTES + 1]),
            Err("larger than 1 MiB")
        );
    }
}
