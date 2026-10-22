//! Bounded binary output from a native image tool. References, never image
//! bytes or arbitrary native filesystem paths, cross the agent API boundary.

#[cfg(feature = "full")]
use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

const ARTIFACT_FAILURE: &str = "cowboyImageFailure";
pub const ARTIFACT_REJECTION_MARKER: &str = "cowboyArtifactRejected";

/// Decode the complete PNG, including its checksums, with explicit memory and
/// pixel bounds. A filename, model claim or savedPath is never sufficient.
pub fn inspect(bytes: &[u8]) -> Result<ImageArtifact, &'static str> {
    if bytes.len() > MAX_IMAGE_BYTES || bytes.is_empty() {
        return Err("artifact_too_large");
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_BYTES,
    });
    let mut reader = decoder.read_info().map_err(|_| "artifact_invalid")?;
    let info = reader.info();
    if info.width == 0
        || info.height == 0
        || u64::from(info.width) * u64::from(info.height) > 16 * 1024 * 1024
        || info.animation_control.is_some()
    {
        return Err("artifact_invalid");
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_DECODED_BYTES)
        .ok_or("artifact_too_large")?;
    let mut buffer = vec![0; size];
    let output = reader
        .next_frame(&mut buffer)
        .map_err(|_| "artifact_invalid")?;
    reader.finish().map_err(|_| "artifact_invalid")?;
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let id = format!("{sha256}.png");
    Ok(ImageArtifact {
        url: format!("/api/artifacts/{id}"),
        id,
        sha256,
        mime_type: "image/png".into(),
        bytes: bytes.len(),
        width: output.width,
        height: output.height,
    })
}

pub fn generated_update(update: &Value) -> bool {
    matches!(
        update["sessionUpdate"].as_str(),
        Some("tool_call" | "tool_call_update")
    ) && matches!(update["status"].as_str(), Some("completed" | "failed"))
        && (update["rawOutput"][ARTIFACT_MARKER] == true || update[ARTIFACT_MARKER] == true)
}

/// Keep the small native provenance and quota receipt when transcript
/// compaction discards rawOutput. This projection is idempotent on recovery.
pub fn compact_metadata(update: &mut Value) {
    if !generated_update(update) {
        return;
    }
    let failure = quota_failure(update);
    let object = update.as_object_mut().expect("generated tool update");
    object.insert(ARTIFACT_MARKER.into(), Value::Bool(true));
    if let Some(failure) = failure {
        object.insert(ARTIFACT_FAILURE.into(), failure);
    }
}

/// Preserve only the native quota fields needed to decide when to retry.
pub fn quota_failure(update: &Value) -> Option<Value> {
    let failure = update
        .get(ARTIFACT_FAILURE)
        .unwrap_or(&update["rawOutput"]["failure"]);
    if !generated_update(update) || failure["type"] != "usageLimitExceeded" {
        return None;
    }
    let id = failure["limitId"].as_str().filter(|id| id.len() <= 128)?;
    Some(
        serde_json::json!({"type":"usageLimitExceeded", "limitId":id,
        "resetsAt":failure["resetsAt"].as_i64()}),
    )
}

/// Only the native image reporter marks generated output. View-image results,
/// input echoes, text paths and arbitrary resource links are not deliverables.
#[cfg(feature = "full")]
pub fn collect(
    update: &Value,
    store: &crate::artifacts::ArtifactStore,
) -> Result<Vec<ImageArtifact>, &'static str> {
    if !generated_update(update) || update["status"] != "completed" {
        return Ok(Vec::new());
    }
    let mut artifacts = Vec::new();
    for block in update["content"].as_array().into_iter().flatten() {
        let image = &block["content"];
        if block["type"] != "content" {
            continue;
        }
        if let Some(code) = image[ARTIFACT_REJECTION_MARKER].as_str() {
            return Err(match code {
                "artifact_too_large" => "artifact_too_large",
                "artifact_storage_failed" => "artifact_storage_failed",
                _ => "artifact_invalid",
            });
        }
        if image["type"] != "image" {
            continue;
        }
        if image["mimeType"] != "image/png" {
            return Err("artifact_invalid");
        }
        let bytes = if let Some(url) = image["url"].as_str() {
            let name = url
                .strip_prefix("/api/artifacts/")
                .ok_or("artifact_invalid")?;
            let path = store.path(name).ok_or("artifact_missing")?;
            if std::fs::metadata(&path)
                .map_err(|_| "artifact_missing")?
                .len()
                > MAX_IMAGE_BYTES as u64
            {
                return Err("artifact_too_large");
            }
            std::fs::read(path).map_err(|_| "artifact_missing")?
        } else if let Some(data) = image["data"].as_str() {
            if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                return Err("artifact_too_large");
            }
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "artifact_invalid")?
        } else {
            return Err("artifact_missing");
        };
        let artifact = inspect(&bytes)?;
        if image["url"].as_str().is_some_and(|url| url != artifact.url) {
            return Err("artifact_invalid");
        }
        store
            .put_bytes(&bytes, "image/png")
            .map_err(|_| "artifact_storage_failed")?;
        if !artifacts.contains(&artifact) {
            artifacts.push(artifact);
        }
        if artifacts.len() > MAX_IMAGES {
            return Err("artifact_too_many");
        }
    }
    Ok(artifacts)
}

/// Classify the complete native turn before starting any file delivery.
#[cfg(feature = "full")]
pub fn collect_turn(
    updates: &[Value],
    store: &crate::artifacts::ArtifactStore,
) -> Result<Vec<ImageArtifact>, &'static str> {
    let mut images = Vec::new();
    for update in updates {
        for image in collect(update, store)? {
            if !images.contains(&image) {
                images.push(image);
            }
        }
        if images.len() > MAX_IMAGES {
            return Err("artifact_too_many");
        }
    }
    if images.is_empty() {
        return Err(
            if updates.iter().any(|update| quota_failure(update).is_some()) {
                "image_quota_exceeded"
            } else if updates.iter().any(|update| update["status"] == "failed") {
                "native_image_failed"
            } else {
                "image_output_missing"
            },
        );
    }
    Ok(images)
}

pub use cowboy_wire::managed_calls::media::{
    ARTIFACT_MARKER, Delivery, ImageArtifact, MAX_DECODED_BYTES, MAX_IMAGE_BYTES, MAX_IMAGES,
};

#[cfg(all(test, feature = "full"))]
pub(crate) mod tests {
    use super::*;
    pub fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[10, 20, 30, 255]).unwrap();
        drop(writer);
        bytes
    }

    #[test]
    fn only_verified_native_images_become_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::artifacts::ArtifactStore::new(root.path().to_owned()).unwrap();
        let bytes = png();
        let mut update = serde_json::json!({"sessionUpdate":"tool_call_update","status":"completed",
            "rawOutput":{"cowboyGeneratedImage":true},"content":[{"type":"content","content":{
                "type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(&bytes)}}]});
        let mut envelope = serde_json::json!({"kind":"update","update":update});
        store.externalize_images(&mut envelope).unwrap();
        assert!(
            envelope["update"]["content"][0]["content"]
                .get("data")
                .is_none()
        );
        assert!(
            envelope["update"]["content"][0]["content"]["url"]
                .as_str()
                .unwrap()
                .starts_with("/api/artifacts/")
        );
        let artifacts = collect(&update, &store).unwrap();
        assert_eq!(collect(&envelope["update"], &store).unwrap(), artifacts);
        assert_eq!(
            collect_turn(&[update.clone(), envelope["update"].clone()], &store).unwrap(),
            artifacts
        );
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].width, 1);
        assert!(store.path(&artifacts[0].id).is_some());
        update["content"][0]["content"] =
            serde_json::json!({"type":"image","mimeType":"image/png","url":artifacts[0].url});
        assert_eq!(collect(&update, &store).unwrap(), artifacts);
        // A valid PNG stored under another image's digest is corruption, not
        // a new successful result. Native references bind the exact bytes.
        let mut other = Vec::new();
        let mut writer = png::Encoder::new(&mut other, 1, 1).write_header().unwrap();
        writer.write_image_data(&[1]).unwrap();
        drop(writer);
        std::fs::write(store.path(&artifacts[0].id).unwrap(), other).unwrap();
        assert_eq!(collect(&update, &store).unwrap_err(), "artifact_invalid");
        update["status"] = serde_json::json!("failed");
        assert!(collect(&update, &store).unwrap().is_empty());
        assert_eq!(
            collect_turn(&[update.clone()], &store).unwrap_err(),
            "native_image_failed"
        );
        update["rawOutput"]["failure"] = serde_json::json!({
            "type":"usageLimitExceeded","limitId":"image_generation","resetsAt":1234});
        crate::persistence::compact_tool_update(&mut update);
        assert!(update.get("rawOutput").is_none());
        assert_eq!(
            collect_turn(&[update], &store).unwrap_err(),
            "image_quota_exceeded"
        );
        assert_eq!(
            collect_turn(&[], &store).unwrap_err(),
            "image_output_missing"
        );
        assert!(inspect(b"not an image").is_err());
        assert!(inspect(&bytes[..bytes.len() / 2]).is_err());
        let mut corrupt = bytes;
        corrupt[45] ^= 1;
        assert!(inspect(&corrupt).is_err());
    }
}
