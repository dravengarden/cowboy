//! An installation incarnation is not an artifact digest or authorization.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InstallationRevision(String);

impl TryFrom<String> for InstallationRevision {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.strip_prefix("installation-").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            Ok(Self(value))
        } else {
            Err("invalid installation revision")
        }
    }
}

impl From<InstallationRevision> for String {
    fn from(value: InstallationRevision) -> Self {
        value.0
    }
}

impl InstallationRevision {
    #[cfg(feature = "machine-host")]
    pub(crate) fn fresh() -> anyhow::Result<Self> {
        use rand::RngCore as _;
        use std::fmt::Write as _;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes)?;
        let mut value = String::from("installation-");
        for byte in bytes {
            write!(value, "{byte:02x}").expect("writing to a String");
        }
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_is_closed_and_distinct_from_release_digest() {
        for invalid in [
            "null".to_owned(),
            "123".into(),
            "{}".into(),
            format!("\"sha256:{}\"", "a".repeat(64)),
            format!("\"installation-{}\"", "A".repeat(64)),
            format!("\"installation-{}\"", "a".repeat(63)),
        ] {
            assert!(serde_json::from_str::<InstallationRevision>(&invalid).is_err());
        }
        let valid = format!("\"installation-{}\"", "a".repeat(64));
        let parsed: InstallationRevision = serde_json::from_str(&valid).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), valid);
    }
}
