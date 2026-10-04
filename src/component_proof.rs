//! Canonical signed component identity, shared by update and bootstrap admission.

use crate::machine_protocol::{ArtifactFormat, DesiredComponent};

pub(crate) fn component_slot(desired: &DesiredComponent) -> String {
    let kind = serde_json::to_value(&desired.id.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "component".to_owned());
    if desired.id.slot.is_empty() {
        kind
    } else {
        format!("{kind}-{}", desired.id.slot.replace('/', "_"))
    }
}

pub(crate) fn component_proof(desired: &DesiredComponent) -> Vec<u8> {
    let format = match desired.artifact_format {
        ArtifactFormat::Raw => "raw",
        ArtifactFormat::TarGz => "tar_gz",
    };
    let probe = desired
        .probe
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .expect("component probe serializes")
        .unwrap_or_default();
    let mut fields = vec![
        component_slot(desired),
        desired.version.clone(),
        desired.generation.clone(),
        desired.digest.clone(),
        format.to_owned(),
        desired.entrypoint.clone().unwrap_or_default(),
        probe,
        desired.automatic.to_string(),
    ];
    let mut proof = if let Some(reader) = &desired.session_deletion_journal {
        fields.push(serde_json::to_string(reader).expect("reader declaration serializes"));
        b"cowboy-component-v4\n".to_vec()
    } else {
        // Preserve every byte of existing signatures when no claim is present.
        b"cowboy-component-v3\n".to_vec()
    };
    for field in fields {
        proof.extend_from_slice(field.len().to_string().as_bytes());
        proof.push(b':');
        proof.extend_from_slice(field.as_bytes());
        proof.push(b'\n');
    }
    proof
}
