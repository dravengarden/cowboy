//! Import an immutable Markdown bundle, never read a worker's mutable filesystem.
//! TOML frontmatter uses the already pinned parser (including duplicate-key checks).
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Informational Git/source revision. Content, not this claim, defines identity.
    pub revision: String,
    pub profile: String,
    /// Paths relative to one source root; closure includes canonical guidance.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    schema: String,
    id: String,
    scope: Scope,
    extends: Option<String>,
    #[serde(default)]
    guidance: Vec<String>,
    #[serde(default)]
    patterns: BTreeMap<String, String>,
    #[serde(default)]
    replace: BTreeMap<String, Replacement>,
    #[serde(default)]
    disable: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Scope {
    Global,
    Workspace,
    Project,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    from: String,
    file: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub schema: String,
    pub id: String,
    pub revision: u32,
    pub role: Role,
    pub summary: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default)]
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Supervisor,
    Procedure,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub origin: String,
    pub digest: String,
    pub metadata: Pattern,
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Catalog {
    pub digest: String,
    pub patterns: BTreeMap<String, Entry>,
    pub guidance: BTreeMap<String, String>,
}

impl Catalog {
    pub fn complete(&self) -> bool {
        self.patterns
            .values()
            .any(|entry| entry.metadata.role == Role::Supervisor)
            && self.patterns.values().all(|entry| {
                entry.metadata.uses.iter().all(|id| {
                    self.patterns
                        .get(id)
                        .is_some_and(|used| used.metadata.role == Role::Procedure)
                })
            })
    }

    pub fn discovery(&self) -> serde_json::Value {
        serde_json::json!({"digest":self.digest,"complete":self.complete(),
            "patterns":self.patterns.values().map(|p| serde_json::json!({
                "origin":p.origin,"digest":p.digest,"metadata":p.metadata
            })).collect::<Vec<_>>(), "guidance":self.guidance.keys().collect::<Vec<_>>()})
    }
}

pub struct Library {
    profiles: BTreeMap<String, (Profile, Source)>,
}

fn path_ok(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.contains('\\')
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && !p.contains(':'))
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<(T, String), &'static str> {
    let text = text
        .strip_prefix("+++\n")
        .ok_or("pattern_frontmatter_required")?;
    let (header, body) = text
        .split_once("\n+++\n")
        .ok_or("pattern_frontmatter_required")?;
    if header.len() > 8192 || body.trim().is_empty() {
        return Err("invalid_pattern_document");
    }
    Ok((
        toml::from_str(header).map_err(|_| "invalid_pattern_frontmatter")?,
        body.to_owned(),
    ))
}

impl Library {
    pub fn compile(sources: &[Source]) -> Result<Self, &'static str> {
        let mut profiles = BTreeMap::new();
        let mut bytes = 0;
        for source in sources {
            if source.revision.is_empty() || source.revision.len() > 128 || source.files.len() > 64
            {
                return Err("invalid_pattern_source");
            }
            for (path, text) in &source.files {
                bytes += text.len();
                if !path_ok(path) || text.len() > 32768 || bytes > 524288 || text.contains('\0') {
                    return Err("pattern_source_limit");
                }
            }
            let text = source
                .files
                .get(&source.profile)
                .ok_or("pattern_profile_missing")?;
            let (profile, _): (Profile, _) = parse(text)?;
            if profile.schema != "cowboy.pattern-profile/v1"
                || !super::valid_id(&profile.id)
                || (profile.scope != Scope::Global && profile.guidance.is_empty())
                || profile.guidance.len() > 16
            {
                return Err("invalid_pattern_profile");
            }
            for path in &profile.guidance {
                if !source.files.get(path).is_some_and(|s| !s.trim().is_empty()) {
                    return Err("pattern_guidance_missing");
                }
            }
            if profiles
                .insert(profile.id.clone(), (profile, source.clone()))
                .is_some()
            {
                return Err("duplicate_pattern_profile");
            }
        }
        let library = Self { profiles };
        for id in library.profiles.keys() {
            library.resolve(id)?;
        }
        Ok(library)
    }

    pub fn resolve(&self, id: &str) -> Result<Catalog, &'static str> {
        let mut catalog = self.walk(id, &mut BTreeSet::new())?;
        catalog.digest = digest(&serde_json::to_vec(&catalog).unwrap());
        Ok(catalog)
    }

    fn walk(&self, id: &str, seen: &mut BTreeSet<String>) -> Result<Catalog, &'static str> {
        if !seen.insert(id.to_owned()) {
            return Err("pattern_profile_cycle");
        }
        let (profile, source) = self.profiles.get(id).ok_or("pattern_profile_missing")?;
        let mut catalog = if let Some(parent) = &profile.extends {
            let (p, _) = self.profiles.get(parent).ok_or("pattern_parent_missing")?;
            if p.scope >= profile.scope {
                return Err("invalid_pattern_scope_chain");
            }
            self.walk(parent, seen)?
        } else {
            if profile.scope != Scope::Global {
                return Err("pattern_parent_required");
            }
            Catalog::default()
        };
        for path in &profile.guidance {
            catalog
                .guidance
                .insert(format!("{id}/{path}"), source.files[path].clone());
        }
        let mut touched = BTreeSet::new();
        for key in &profile.disable {
            if !touched.insert(key) || catalog.patterns.remove(key).is_none() {
                return Err("invalid_pattern_disable");
            }
        }
        for (key, replacement) in &profile.replace {
            if !touched.insert(key)
                || catalog
                    .patterns
                    .get(key)
                    .is_none_or(|old| old.origin != replacement.from)
            {
                return Err("stale_pattern_replacement");
            }
            catalog.patterns.insert(
                key.clone(),
                Self::entry(profile, source, key, &replacement.file)?,
            );
        }
        for (key, path) in &profile.patterns {
            if !touched.insert(key) || catalog.patterns.contains_key(key) {
                return Err("implicit_pattern_override");
            }
            catalog
                .patterns
                .insert(key.clone(), Self::entry(profile, source, key, path)?);
        }
        Ok(catalog)
    }

    fn entry(
        profile: &Profile,
        source: &Source,
        key: &str,
        path: &str,
    ) -> Result<Entry, &'static str> {
        if !path_ok(path) {
            return Err("invalid_pattern_path");
        }
        let dir = source.profile.rsplit_once('/').map_or("", |(dir, _)| dir);
        let path = if dir.is_empty() {
            path.to_owned()
        } else {
            format!("{dir}/{path}")
        };
        let text = source.files.get(&path).ok_or("pattern_file_missing")?;
        let (pattern, body): (Pattern, _) = parse(text)?;
        if pattern.schema != "cowboy.pattern/v1"
            || pattern.id != key
            || !super::valid_id(key)
            || pattern.revision == 0
            || pattern.summary.trim().is_empty()
            || pattern.summary.len() > 240
            || pattern.uses.len() > 8
            || pattern.uses.iter().any(|id| !super::valid_id(id))
            || pattern.events.iter().any(|e| e != "turn.completed")
            || pattern.effects.iter().any(|e| e != "session.follow_up")
        {
            return Err("invalid_pattern_metadata");
        }
        if pattern.role == Role::Procedure
            && (!pattern.events.is_empty()
                || !pattern.effects.is_empty()
                || !pattern.uses.is_empty())
        {
            return Err("invalid_pattern_procedure");
        }
        if pattern.role == Role::Supervisor && pattern.events.is_empty() {
            return Err("pattern_event_required");
        }
        Ok(Entry {
            origin: format!("{}/{key}@{}", profile.id, pattern.revision),
            digest: digest(text.as_bytes()),
            metadata: pattern,
            body,
        })
    }
}

pub fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
