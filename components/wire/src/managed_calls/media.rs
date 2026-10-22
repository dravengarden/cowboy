//! Image delivery records that cross the Machine protocol.

use serde::{Deserialize, Serialize};

pub const MAX_IMAGES: usize = 4;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
pub const ARTIFACT_MARKER: &str = "cowboyGeneratedImage";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageArtifact {
    pub id: String,
    pub sha256: String,
    pub mime_type: String,
    pub bytes: usize,
    pub width: u32,
    pub height: u32,
    pub url: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub parent_session_id: String,
    pub child_session_id: String,
    pub call_id: String,
    pub artifact: ImageArtifact,
    pub data: String,
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedArtifactDelivery([private])")
    }
}
