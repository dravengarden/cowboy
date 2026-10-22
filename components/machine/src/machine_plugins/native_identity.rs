//! Read native identity only from a disposable login home, without following
//! symlinks, opening special files, or including native output in errors.
use anyhow::{Context as _, Result, ensure};
use cowboy_provider_sdk::NativeIdentityRecipe;
use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
use std::io::Read as _;
use std::path::{Component, Path};

const LIMIT: usize = 256 * 1024;

pub(super) fn read(
    home: &Path,
    recipe: &NativeIdentityRecipe,
) -> Result<crate::provider_account::NativeIdentity> {
    let NativeIdentityRecipe::FreshLoginJsonV1 {
        authority,
        relative_path,
        subject_path,
        tenant_path,
    } = recipe;
    let bytes = read_login_file(home, relative_path, LIMIT)?;
    let document: UniqueJson = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid native identity JSON"))?;
    let document = document.0;
    let subject = field(&document, subject_path)?;
    let tenant = tenant_path
        .as_ref()
        .map(|path| field(&document, path))
        .transpose()?;
    Ok(crate::provider_account::NativeIdentity {
        authority: authority.clone(),
        subject,
        tenant,
    })
}

/// Native account identity must have one interpretation. `Value` alone silently
/// keeps the last duplicate member, which can disagree with another reader of
/// the same native login file. Check every object, including objects in arrays,
/// and never include a rejected member name or value in an error.
struct UniqueJson(serde_json::Value);

impl<'de> serde::Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON without duplicate object members")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut object = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate native identity member"));
                    }
                    object.insert(key, map.next_value::<UniqueJson>()?.0);
                }
                Ok(UniqueJson(serde_json::Value::Object(object)))
            }

            fn visit_seq<S: serde::de::SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> Result<Self::Value, S::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueJson>()? {
                    values.push(value.0);
                }
                Ok(UniqueJson(serde_json::Value::Array(values)))
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|value| UniqueJson(serde_json::Value::Number(value)))
                    .ok_or_else(|| serde::de::Error::custom("invalid native identity number"))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(serde_json::Value::Null))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Shared by identity and credential export: enforce the bound before reading
/// and on the opened descriptor, including files that grow after fstat.
pub(super) fn read_login_file(home: &Path, relative_path: &str, limit: usize) -> Result<Vec<u8>> {
    let parts: Vec<_> = Path::new(relative_path).components().collect();
    ensure!(
        !parts.is_empty()
            && parts
                .iter()
                .all(|part| matches!(part, Component::Normal(_))),
        "unsafe native login output path"
    );
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open(home, directory_flags, Mode::empty())
        .map_err(std::io::Error::from)
        .context("opening native login home")?;
    for part in &parts[..parts.len() - 1] {
        directory = openat(&directory, part.as_os_str(), directory_flags, Mode::empty())
            .map_err(std::io::Error::from)
            .context("opening native login output parent")?;
    }
    let file = openat(
        &directory,
        parts.last().unwrap().as_os_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)
    .context("opening native login output file")?;
    let metadata = fstat(&file)?;
    ensure!(
        FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile,
        "native login output is not a regular file"
    );
    ensure!(
        usize::try_from(metadata.st_size).is_ok_and(|size| size <= limit),
        "native login output exceeds size limit"
    );
    let mut bytes = Vec::new();
    std::fs::File::from(file)
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "native login output exceeds size limit"
    );
    Ok(bytes)
}

fn field(document: &serde_json::Value, path: &[String]) -> Result<String> {
    ensure!(
        !path.is_empty() && path.len() <= 16,
        "invalid native identity field path"
    );
    let mut value = document;
    for part in path {
        value = value
            .as_object()
            .and_then(|object| object.get(part))
            .context("native identity field is missing")?;
    }
    let value = value
        .as_str()
        .context("native identity field must be a string")?;
    ensure!(
        !value.is_empty()
            && value.len() <= 512
            && value.trim() == value
            && !value.chars().any(char::is_control),
        "invalid native identity field"
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn recipe() -> NativeIdentityRecipe {
        NativeIdentityRecipe::FreshLoginJsonV1 {
            authority: "fixture".into(),
            relative_path: "profile.json".into(),
            subject_path: vec!["account".into(), "id".into()],
            tenant_path: Some(vec!["tenant".into()]),
        }
    }

    #[test]
    fn claude_subscription_recipe_requires_native_account_and_organization() {
        let source: serde_json::Value =
            serde_json::from_str(include_str!("../../../../plugins/claude-code/provider.json")).unwrap();
        let method = source["authentication"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|method| method["id"] == "claude-subscription")
            .unwrap();
        let recipe: NativeIdentityRecipe =
            serde_json::from_value(method["native_identity"].clone()).unwrap();
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude.json");
        for profile in [
            r#"{"oauthAccount":{"emailAddress":"person@example.invalid"}}"#,
            r#"{"oauthAccount":{"accountUuid":"account"}}"#,
            r#"{"oauthAccount":{"accountUuid":"","organizationUuid":"org"}}"#,
        ] {
            fs::write(&path, profile).unwrap();
            assert!(read(home.path(), &recipe).is_err());
        }
        fs::write(&path, r#"{"oauthAccount":{"accountUuid":"account","organizationUuid":"org","emailAddress":"person@example.invalid"}}"#).unwrap();
        let identity = read(home.path(), &recipe).unwrap();
        assert_eq!(identity.authority, "claude-ai");
        assert_eq!(identity.subject, "account");
        assert_eq!(identity.tenant.as_deref(), Some("org"));
        assert!(
            !serde_json::to_string(&identity)
                .unwrap()
                .contains("person@")
        );
        assert_eq!(
            method["native_logout"]["arguments"],
            serde_json::json!(["auth", "logout"])
        );
        assert!(
            source["authentication"]["methods"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|method| method["id"] != "claude-subscription")
                .all(|method| method.get("native_identity").is_none())
        );
    }

    #[test]
    fn reads_only_declared_stable_fields_and_rejects_incomplete_identity() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("profile.json");
        fs::write(&path, r#"{"account":{"id":"subject"},"tenant":"org","email":"irrelevant","token":"fixture-secret"}"#).unwrap();
        let identity = read(home.path(), &recipe()).unwrap();
        assert_eq!(identity.subject, "subject");
        assert_eq!(identity.tenant.as_deref(), Some("org"));
        assert!(
            !serde_json::to_string(&identity)
                .unwrap()
                .contains("fixture-secret")
        );
        for invalid in [
            r#"{"account":{"id":"subject"}}"#,
            r#"{"account":{"id":3},"tenant":"org"}"#,
            r#"{"account":{"id":" "},"tenant":"org"}"#,
            "fixture-secret-invalid-json",
        ] {
            fs::write(&path, invalid).unwrap();
            let error = read(home.path(), &recipe()).unwrap_err().to_string();
            assert!(!error.contains("fixture-secret"));
        }
    }

    #[test]
    fn ambiguous_identity_json_is_rejected_without_leaking_fields() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("profile.json");
        for invalid in [
            r#"{"account":{"id":"first"},"account":{"id":"second"},"tenant":"org"}"#,
            r#"{"account":{"id":"first","id":"second"},"tenant":"org"}"#,
            r#"{"account":{"id":"first","\u0069d":"second"},"tenant":"org"}"#,
            r#"{"account":{"id":"subject"},"tenant":"org","extra":[{"secret-key":"secret-value","secret-key":"other"}]}"#,
        ] {
            fs::write(&path, invalid).unwrap();
            let error = read(home.path(), &recipe()).unwrap_err();
            assert_eq!(format!("{error:#}"), "invalid native identity JSON");
        }
        // Unknown native fields remain forward compatible; only ambiguous
        // objects are rejected. Preserve ordinary JSON values and bounds.
        fs::write(&path, r#"{"account":{"id":"subject"},"tenant":"org","extra":[null,true,false,-1,18446744073709551615,1.5,"text",{"id":"independent"}]}"#).unwrap();
        assert_eq!(read(home.path(), &recipe()).unwrap().subject, "subject");
        fs::write(&path, format!("{}0{}", "[".repeat(130), "]".repeat(130))).unwrap();
        assert_eq!(
            read(home.path(), &recipe()).unwrap_err().to_string(),
            "invalid native identity JSON"
        );
    }

    #[test]
    fn login_output_is_bounded_and_special_files_cannot_block_export() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("credentials");
        fs::write(&path, b"test-secret").unwrap();
        assert_eq!(
            read_login_file(home.path(), "credentials", 11).unwrap(),
            b"test-secret"
        );
        assert!(read_login_file(home.path(), "credentials", 10).is_err());
        fs::remove_file(&path).unwrap();
        crate::test_fifo(&path).unwrap();
        assert!(read_login_file(home.path(), "credentials", 1024).is_err());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(read_login_file(home.path(), "credentials", 1024).is_err());
        for path in ["../credentials", "/credentials", ""] {
            assert!(read_login_file(home.path(), path, 1024).is_err());
        }
        let missing = read_login_file(home.path(), "missing", 1024).unwrap_err();
        assert_eq!(
            missing.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn rejects_file_and_parent_aliases_and_oversized_input() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("profile.json"), "{}").unwrap();
        let path = home.path().join("profile.json");
        symlink(outside.path().join("profile.json"), &path).unwrap();
        assert!(read(home.path(), &recipe()).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, vec![b' '; LIMIT + 1]).unwrap();
        assert!(read(home.path(), &recipe()).is_err());
        symlink(outside.path(), home.path().join("alias")).unwrap();
        let mut nested = recipe();
        let NativeIdentityRecipe::FreshLoginJsonV1 { relative_path, .. } = &mut nested;
        *relative_path = "alias/profile.json".into();
        assert!(read(home.path(), &nested).is_err());
    }
}
