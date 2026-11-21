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
    account: &crate::provider_account::ProviderAccountId,
) -> Result<crate::provider_account::NativeIdentity> {
    let (authority, relative_path, subject_path, tenant_path, subject_claim) = match recipe {
        NativeIdentityRecipe::OwnerDeclaredV1 { authority } => {
            ensure!(
                !account.is_default(),
                "owner-declared identity requires a named account"
            );
            return Ok(crate::provider_account::NativeIdentity {
                authority: authority.clone(),
                subject: account.as_str().to_owned(),
                tenant: None,
            });
        }
        NativeIdentityRecipe::FreshLoginJsonV1 {
            authority,
            relative_path,
            subject_path,
            tenant_path,
            subject_claim,
        } => (
            authority,
            relative_path,
            subject_path,
            tenant_path,
            subject_claim,
        ),
    };
    let bytes = read_login_file(home, relative_path, LIMIT)?;
    let document: UniqueJson = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid native identity JSON"))?;
    let document = document.0;
    let subject = match subject_claim {
        None => field(&document, subject_path)?,
        Some(claim) => field(&jwt_payload(&string(&document, subject_path)?)?, claim)?,
    };
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
/// Check a credential file of a native home from descriptor metadata only,
/// never reading its bytes into this process: every parent is opened without
/// following links and the file is `stat`ed relative to its parent without
/// following a final link, and it must be a
/// regular file with exactly one link, owned by this Machine's user. `false`
/// when it does not exist; any other shape is an error.
pub(super) fn credential_file_safe(home: &Path, relative_path: &str) -> Result<bool> {
    let parts: Vec<_> = Path::new(relative_path).components().collect();
    ensure!(
        !parts.is_empty()
            && parts
                .iter()
                .all(|part| matches!(part, Component::Normal(_))),
        "unsafe native credential path"
    );
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let absent = |error: rustix::io::Errno| error == rustix::io::Errno::NOENT;
    let mut directory = open(home, directory_flags, Mode::empty())
        .map_err(std::io::Error::from)
        .context("opening native home")?;
    for part in &parts[..parts.len() - 1] {
        directory = match openat(&directory, part.as_os_str(), directory_flags, Mode::empty()) {
            Ok(next) => next,
            Err(error) if absent(error) => return Ok(false),
            Err(error) => {
                return Err(std::io::Error::from(error)).context("native credential parent");
            }
        };
    }
    // `statat` without following the final link, relative to the verified
    // parent descriptor (portable to every Machine platform, unlike O_PATH).
    let metadata = match rustix::fs::statat(
        &directory,
        parts.last().unwrap().as_os_str(),
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(metadata) => metadata,
        Err(error) if absent(error) => return Ok(false),
        Err(error) => return Err(std::io::Error::from(error)).context("native credential file"),
    };
    ensure!(
        FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
            && metadata.st_nlink == 1
            && metadata.st_uid == rustix::process::geteuid().as_raw(),
        "native credential is not a private regular file"
    );
    Ok(true)
}

pub(super) fn read_login_file(home: &Path, relative_path: &str, limit: usize) -> Result<Vec<u8>> {
    let file = open_login_file(home, relative_path, limit)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "native login output exceeds size limit"
    );
    Ok(bytes)
}

/// Open a regular login file of `home` without following links, bounded by
/// `limit` bytes at open time.
fn open_login_file(home: &Path, relative_path: &str, limit: usize) -> Result<std::fs::File> {
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
    Ok(std::fs::File::from(file))
}

/// The most a live native login file is scanned: config files that grow with
/// projects and MCP servers stay readable, but nothing beyond this is read.
const LIVE_SCAN_LIMIT: usize = 64 * 1024 * 1024;
/// The fields of a live login file a caller needs, as a tree of keys.
#[derive(Default)]
struct Wanted(std::collections::BTreeMap<String, Wanted>);

/// Selects exactly the wanted fields while streaming: unwanted members are
/// skipped without being kept, every kept leaf is a string of at most 512
/// bytes (or null), so memory stays bounded however large the file is. A
/// wanted member appearing twice is refused; [`NATIVE_IDENTITY_SOLE_MEMBER`]
/// selects the only member of an object and refuses any other shape.
///
/// [`NATIVE_IDENTITY_SOLE_MEMBER`]: cowboy_provider_sdk::NATIVE_IDENTITY_SOLE_MEMBER
struct Pick<'a>(&'a Wanted);

impl<'de> serde::de::DeserializeSeed<'de> for Pick<'_> {
    type Value = serde_json::Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Pick<'_> {
    type Value = serde_json::Value;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0.0.is_empty() {
            "a short string"
        } else {
            "an object"
        })
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        if !self.0.0.is_empty() || value.len() > 512 {
            return Err(E::custom("unexpected native identity field"));
        }
        Ok(value.into())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut selected = serde_json::Map::new();
        if let Some(only) = self
            .0
            .0
            .get(cowboy_provider_sdk::NATIVE_IDENTITY_SOLE_MEMBER)
        {
            let key = map
                .next_key::<String>()?
                .ok_or_else(|| serde::de::Error::custom("native identity object is empty"))?;
            selected.insert(key, map.next_value_seed(Pick(only))?);
            if map.next_key::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom(
                    "native identity object must have one member",
                ));
            }
            return Ok(serde_json::Value::Object(selected));
        }
        while let Some(key) = map.next_key::<String>()? {
            let Some(child) = self.0.0.get(&key) else {
                map.next_value::<serde::de::IgnoredAny>()?;
                continue;
            };
            if selected.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate native identity member"));
            }
            let value = map.next_value_seed(Pick(child))?;
            selected.insert(key, value);
        }
        Ok(serde_json::Value::Object(selected))
    }
}

/// Stream a live login file and keep only the fields at `paths` (see
/// [`Pick`]); the result is a skeleton document for [`field`] and [`lookup`].
pub(super) fn read_live_fields(
    home: &Path,
    relative_path: &str,
    paths: &[&[String]],
) -> Result<serde_json::Value> {
    let mut wanted = Wanted::default();
    for path in paths {
        ensure!(
            !path.is_empty() && path.len() <= 16,
            "invalid native identity field path"
        );
        let mut node = &mut wanted;
        for part in *path {
            node = node.0.entry(part.clone()).or_default();
        }
    }
    let file = open_login_file(home, relative_path, LIVE_SCAN_LIMIT)?;
    let reader = std::io::BufReader::new(file.take(LIVE_SCAN_LIMIT as u64));
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let document = serde::de::DeserializeSeed::deserialize(Pick(&wanted), &mut deserializer)
        .map_err(|_| anyhow::anyhow!("invalid native identity JSON"))?;
    deserializer
        .end()
        .map_err(|_| anyhow::anyhow!("invalid native identity JSON"))?;
    Ok(document)
}

/// [`read`] on a live native home: a fresh-login recipe reads exactly its
/// fields by streaming, with bounded memory.
pub(super) fn read_live(
    home: &Path,
    recipe: &NativeIdentityRecipe,
    account: &crate::provider_account::ProviderAccountId,
) -> Result<crate::provider_account::NativeIdentity> {
    let NativeIdentityRecipe::FreshLoginJsonV1 {
        authority,
        relative_path,
        subject_path,
        tenant_path,
        subject_claim: None,
    } = recipe
    else {
        return read(home, recipe, account);
    };
    let paths: Vec<&[String]> = std::iter::once(subject_path.as_slice())
        .chain(tenant_path.as_deref())
        .collect();
    let document = read_live_fields(home, relative_path, &paths)?;
    Ok(crate::provider_account::NativeIdentity {
        authority: authority.clone(),
        subject: field(&document, subject_path)?,
        tenant: tenant_path
            .as_ref()
            .map(|path| field(&document, path))
            .transpose()?,
    })
}

/// Decode the payload of a compact JWT without trusting it: the token comes
/// from the same fresh login file, so it only locates a stable claim.
fn jwt_payload(token: &str) -> Result<serde_json::Value> {
    use base64::Engine as _;
    let mut parts = token.split('.');
    let (Some(_), Some(payload), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        anyhow::bail!("native identity token is not a compact JWT");
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| anyhow::anyhow!("invalid native identity token"))?;
    let claims: UniqueJson = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid native identity token"))?;
    Ok(claims.0)
}

fn lookup<'a>(document: &'a serde_json::Value, path: &[String]) -> Result<&'a serde_json::Value> {
    ensure!(
        !path.is_empty() && path.len() <= 16,
        "invalid native identity field path"
    );
    let mut value = document;
    for part in path {
        let object = value
            .as_object()
            .context("native identity field is missing")?;
        value = if part == cowboy_provider_sdk::NATIVE_IDENTITY_SOLE_MEMBER {
            let mut members = object.values();
            match (members.next(), members.next()) {
                (Some(only), None) => only,
                _ => anyhow::bail!("native identity object must have exactly one member"),
            }
        } else {
            object
                .get(part)
                .context("native identity field is missing")?
        };
    }
    Ok(value)
}

/// A bounded string such as a token; identity fields use [`field`].
fn string(document: &serde_json::Value, path: &[String]) -> Result<String> {
    let value = lookup(document, path)?
        .as_str()
        .context("native identity field must be a string")?;
    ensure!(
        !value.is_empty() && value.len() <= 16 * 1024 && value.is_ascii(),
        "invalid native identity token"
    );
    Ok(value.to_owned())
}

fn field(document: &serde_json::Value, path: &[String]) -> Result<String> {
    let value = lookup(document, path)?
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

    fn named() -> crate::provider_account::ProviderAccountId {
        crate::provider_account::ProviderAccountId::try_from(
            "8f0c6f39-2c55-4a3b-9b3b-7c1d6e5a4b21".to_owned(),
        )
        .unwrap()
    }

    #[test]
    fn owner_declared_identity_is_the_named_account_and_reads_no_file() {
        let home = tempfile::tempdir().unwrap();
        let recipe = NativeIdentityRecipe::OwnerDeclaredV1 {
            authority: "google".into(),
        };
        let identity = read(home.path(), &recipe, &named()).unwrap();
        assert_eq!(identity.authority, "google");
        assert_eq!(identity.subject, named().as_str());
        assert_eq!(identity.tenant, None);
        assert!(
            read(
                home.path(),
                &recipe,
                &crate::provider_account::ProviderAccountId::default()
            )
            .is_err()
        );
    }
    use std::fs;
    use std::os::unix::fs::symlink;

    fn recipe() -> NativeIdentityRecipe {
        NativeIdentityRecipe::FreshLoginJsonV1 {
            authority: "fixture".into(),
            relative_path: "profile.json".into(),
            subject_path: vec!["account".into(), "id".into()],
            tenant_path: Some(vec!["tenant".into()]),
            subject_claim: None,
        }
    }

    #[test]
    fn claude_subscription_recipe_requires_native_account_and_organization() {
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../plugins/claude-code/provider.json"
        ))
        .unwrap();
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
            assert!(read(home.path(), &recipe, &named()).is_err());
        }
        fs::write(&path, r#"{"oauthAccount":{"accountUuid":"account","organizationUuid":"org","emailAddress":"person@example.invalid"}}"#).unwrap();
        let identity = read(home.path(), &recipe, &named()).unwrap();
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
        let identity = read(home.path(), &recipe(), &named()).unwrap();
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
            let error = read(home.path(), &recipe(), &named())
                .unwrap_err()
                .to_string();
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
            let error = read(home.path(), &recipe(), &named()).unwrap_err();
            assert_eq!(format!("{error:#}"), "invalid native identity JSON");
        }
        // Unknown native fields remain forward compatible; only ambiguous
        // objects are rejected. Preserve ordinary JSON values and bounds.
        fs::write(&path, r#"{"account":{"id":"subject"},"tenant":"org","extra":[null,true,false,-1,18446744073709551615,1.5,"text",{"id":"independent"}]}"#).unwrap();
        assert_eq!(
            read(home.path(), &recipe(), &named()).unwrap().subject,
            "subject"
        );
        fs::write(&path, format!("{}0{}", "[".repeat(130), "]".repeat(130))).unwrap();
        assert_eq!(
            read(home.path(), &recipe(), &named())
                .unwrap_err()
                .to_string(),
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
    fn a_large_live_login_file_still_yields_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let projects: serde_json::Map<String, serde_json::Value> = (0..20_000)
            .map(|index| {
                (
                    format!("/p/{index}"),
                    serde_json::json!({"history": "x".repeat(40)}),
                )
            })
            .collect();
        let document = serde_json::json!({"projects": projects, "oauthAccount":
            {"accountUuid": "otter", "organizationUuid": "org-a", "emailAddress": "o@x"}});
        let bytes = serde_json::to_vec(&document).unwrap();
        assert!(bytes.len() > LIMIT, "larger than the fresh-login bound");
        std::fs::write(home.join(".claude.json"), bytes).unwrap();
        let claude = NativeIdentityRecipe::FreshLoginJsonV1 {
            authority: "claude-ai".into(),
            relative_path: ".claude.json".into(),
            subject_path: vec!["oauthAccount".into(), "accountUuid".into()],
            tenant_path: Some(vec!["oauthAccount".into(), "organizationUuid".into()]),
            subject_claim: None,
        };
        let identity = read_live(&home, &claude, &named()).unwrap();
        assert_eq!(identity.subject, "otter");
        assert_eq!(identity.tenant.as_deref(), Some("org-a"));
        let email = vec!["oauthAccount".to_owned(), "emailAddress".to_owned()];
        let fields = read_live_fields(&home, ".claude.json", &[&email]).unwrap();
        assert_eq!(fields["oauthAccount"]["emailAddress"], "o@x");
        assert!(fields.get("projects").is_none());
        // Only the wanted fields are kept: the organization is not.
        assert!(fields["oauthAccount"].get("organizationUuid").is_none());
        // A sole-member recipe selects the only member of an object.
        std::fs::write(
            home.join("sole.json"),
            br#"{"accounts":{"https://issuer":{"sub":"lynx"}},"big":[1,2,3]}"#,
        )
        .unwrap();
        let sole = vec!["accounts".to_owned(), "*".to_owned(), "sub".to_owned()];
        let document = read_live_fields(&home, "sole.json", &[&sole]).unwrap();
        assert_eq!(field(&document, &sole).unwrap(), "lynx");
        std::fs::write(
            home.join("two.json"),
            br#"{"accounts":{"a":{"sub":"x"},"b":{"sub":"y"}}}"#,
        )
        .unwrap();
        assert!(read_live_fields(&home, "two.json", &[&sole]).is_err());
        // An oversized selected field is refused without keeping it.
        std::fs::write(
            home.join("long.json"),
            serde_json::to_vec(
                &serde_json::json!({"oauthAccount": {"emailAddress": "x".repeat(600)}}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(read_live_fields(&home, "long.json", &[&email]).is_err());
    }

    #[test]
    fn credential_files_are_checked_without_reading_them() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let path = ".claude/.credentials.json";
        assert!(!credential_file_safe(&home, path).unwrap());
        std::fs::write(home.join(path), b"{}").unwrap();
        assert!(credential_file_safe(&home, path).unwrap());
        // A second link to the same bytes is another holder of the token.
        std::fs::hard_link(home.join(path), dir.path().join("copy")).unwrap();
        assert!(credential_file_safe(&home, path).is_err());
        std::fs::remove_file(dir.path().join("copy")).unwrap();
        // A symlink to another subscription's credential is refused.
        std::fs::remove_file(home.join(path)).unwrap();
        std::fs::write(dir.path().join("other"), b"{}").unwrap();
        std::os::unix::fs::symlink(dir.path().join("other"), home.join(path)).unwrap();
        assert!(credential_file_safe(&home, path).is_err());
        // So is a linked parent directory.
        std::fs::remove_file(home.join(path)).unwrap();
        std::fs::remove_dir(home.join(".claude")).unwrap();
        std::fs::create_dir(dir.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), home.join(".claude")).unwrap();
        assert!(credential_file_safe(&home, path).is_err());
    }

    #[test]
    fn rejects_file_and_parent_aliases_and_oversized_input() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("profile.json"), "{}").unwrap();
        let path = home.path().join("profile.json");
        symlink(outside.path().join("profile.json"), &path).unwrap();
        assert!(read(home.path(), &recipe(), &named()).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, vec![b' '; LIMIT + 1]).unwrap();
        assert!(read(home.path(), &recipe(), &named()).is_err());
        symlink(outside.path(), home.path().join("alias")).unwrap();
        let mut nested = recipe();
        let NativeIdentityRecipe::FreshLoginJsonV1 { relative_path, .. } = &mut nested else {
            unreachable!()
        };
        *relative_path = "alias/profile.json".into();
        assert!(read(home.path(), &nested, &named()).is_err());
    }

    fn plugin_recipe(plugin: &str, method: &str) -> NativeIdentityRecipe {
        let source: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(format!(
                "{}/../../plugins/{plugin}/provider.json",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap(),
        )
        .unwrap();
        let method = source["authentication"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == method)
            .unwrap();
        serde_json::from_value(method["native_identity"].clone()).unwrap()
    }

    fn jwt(claims: &serde_json::Value) -> String {
        use base64::Engine as _;
        let encode = |value: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value);
        format!(
            "{}.{}.{}",
            encode(br#"{"alg":"RS256"}"#),
            encode(claims.to_string().as_bytes()),
            encode(b"signature")
        )
    }

    #[test]
    fn codex_subscription_reads_the_chatgpt_user_from_the_fresh_id_token() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".codex")).unwrap();
        let token = jwt(&serde_json::json!({
            "sub":"auth0|subject",
            "https://api.openai.com/auth":{"chatgpt_user_id":"user-1","chatgpt_account_id":"workspace-1"}
        }));
        let auth = serde_json::json!({"auth_mode":"chatgpt","tokens":{
            "id_token":token,"access_token":"a","refresh_token":"r","account_id":"workspace-1"}});
        fs::write(home.path().join(".codex/auth.json"), auth.to_string()).unwrap();
        let identity = read(
            home.path(),
            &plugin_recipe("codex", "chatgpt-subscription"),
            &named(),
        )
        .unwrap();
        assert_eq!(identity.authority, "chatgpt");
        assert_eq!(identity.subject, "user-1");
        assert_eq!(identity.tenant.as_deref(), Some("workspace-1"));
        for invalid in ["not-a-jwt", "a.b", "a.%%%.c", "a.b.c.d"] {
            let auth =
                serde_json::json!({"tokens":{"id_token":invalid,"account_id":"workspace-1"}});
            fs::write(home.path().join(".codex/auth.json"), auth.to_string()).unwrap();
            assert!(
                read(
                    home.path(),
                    &plugin_recipe("codex", "chatgpt-subscription"),
                    &named()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn grok_subscription_reads_the_sole_issuer_entry_only() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join(".grok")).unwrap();
        let entry = serde_json::json!({"key":"k","user_id":"user-1","team_id":"team-1"});
        let path = home.path().join(".grok/auth.json");
        fs::write(
            &path,
            serde_json::json!({"https://auth.x.ai::client": entry}).to_string(),
        )
        .unwrap();
        let recipe = plugin_recipe("grok", "xai-subscription");
        let identity = read(home.path(), &recipe, &named()).unwrap();
        assert_eq!(identity.authority, "x-ai");
        assert_eq!(identity.subject, "user-1");
        assert_eq!(identity.tenant.as_deref(), Some("team-1"));
        // Two cached logins are ambiguous; an empty cache has no identity.
        fs::write(
            &path,
            serde_json::json!({"a::one": entry, "b::two": entry}).to_string(),
        )
        .unwrap();
        assert!(read(home.path(), &recipe, &named()).is_err());
        fs::write(&path, "{}").unwrap();
        assert!(read(home.path(), &recipe, &named()).is_err());
    }
}
