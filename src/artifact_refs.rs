//! Image digests and prompt-echo references shared by the worker and the
//! Controller's artifact store. The worker only names digests; storing and
//! serving the bytes stays in `artifacts`, so the worker does not depend on it.

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

/// Inline image payloads below this encoded size stay in the event row.
pub(crate) const INLINE_IMAGE_LIMIT_BYTES: usize = 32 * 1024;

/// Replaces `data` on a worker's prompt echo image when Cowboy stored those
/// exact bytes before dispatch (`CoreCommand::Prompt::echo_artifacts`). The
/// Controller resolves it to the artifact URL before the event is recorded.
pub const ECHO_ARTIFACT_FIELD: &str = "artifactSha256";

/// Content digest of an inline image payload large enough to be externalized.
#[must_use]
pub fn large_image_digest(encoded: &str) -> Option<String> {
    if encoded.len() < INLINE_IMAGE_LIMIT_BYTES {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Swap the bytes of a prompt-echo image for their digest when the digest is
/// one Cowboy stored before dispatch. Other updates are left untouched.
pub fn reference_echoed_image(update: &mut serde_json::Value, stored: &[String]) {
    if stored.is_empty()
        || update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            != Some("user_message_chunk")
    {
        return;
    }
    let Some(content) = update
        .get_mut("content")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    if content.get("type").and_then(serde_json::Value::as_str) != Some("image") {
        return;
    }
    let Some(digest) = content
        .get("data")
        .and_then(serde_json::Value::as_str)
        .and_then(large_image_digest)
        .filter(|digest| stored.contains(digest))
    else {
        return;
    };
    content.remove("data");
    content.insert(
        ECHO_ARTIFACT_FIELD.to_owned(),
        serde_json::Value::String(digest),
    );
}
