//! The synchronized Review state of a session (`mobile-review:<sid>`):
//! its shape, bounds and the validation applied to stored and written values.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub(super) const MOBILE_REVIEW_TAB_CAP: usize = 12;
pub(super) const MOBILE_REVIEW_PROGRESS_CAP: usize = 512;
pub(super) const MOBILE_REVIEW_BINDING_CAP: usize = 16;
pub(super) const MOBILE_REVIEW_POSITION_CAP: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MobileReviewTab {
    pub(super) path: String,
    #[serde(default)]
    pub(super) pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MobileReviewPosition {
    pub(super) line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MobileReviewState {
    /// Reopening the active path is still a new navigation intent.
    #[serde(default)]
    pub(super) open_revision: u64,
    #[serde(default)]
    pub(super) reveal_revision: u64,
    /// What the latest reveal named, bound to the `reveal_revision` it set,
    /// so a reader never takes a later `active` for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reveal: Option<MobileReviewReveal>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::review_directory::deserialize_override"
    )]
    pub(super) directory: Option<Option<String>>,
    // Preserve opaque settings when an older Controller reads and rewrites
    // state produced by a newer release. Mutations remain a closed protocol.
    #[serde(default, flatten)]
    pub(super) future: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) remote_review: Option<crate::workspace_extensions::ReviewBinding>,
    #[serde(default)]
    pub(super) remote_selected: bool,
    /// The chosen comparison: `""` is uncommitted work, otherwise a Git
    /// reference. Absent means the repository's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) comparison: Option<String>,
    /// Code contexts this session also works in, as `review_directory` values.
    /// A binding names an existing context and grants no file access.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) bindings: Vec<String>,
    #[serde(default = "default_mobile_review_mode")]
    pub(super) mode: String,
    #[serde(default)]
    pub(super) tabs: Vec<MobileReviewTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) active: Option<String>,
    #[serde(default)]
    pub(super) progress: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub(super) positions: std::collections::BTreeMap<String, MobileReviewPosition>,
}

pub(super) fn default_mobile_review_mode() -> String {
    "git".to_owned()
}

impl Default for MobileReviewState {
    fn default() -> Self {
        Self {
            open_revision: 0,
            reveal: None,
            reveal_revision: 0,
            directory: None,
            future: std::collections::BTreeMap::new(),
            remote_review: None,
            remote_selected: false,
            comparison: None,
            bindings: Vec::new(),
            mode: default_mobile_review_mode(),
            tabs: Vec::new(),
            active: None,
            progress: std::collections::BTreeMap::new(),
            positions: std::collections::BTreeMap::new(),
        }
    }
}

impl MobileReviewState {
    pub(super) fn from_stored(value: serde_json::Value) -> Self {
        let mut state = serde_json::from_value::<Self>(value).unwrap_or_default();
        if state
            .directory
            .as_ref()
            .and_then(Option::as_deref)
            .is_some_and(|value| !crate::review_directory::valid(value))
        {
            state.directory = None;
        }
        if state
            .remote_review
            .as_ref()
            .is_some_and(|binding| !binding.valid())
        {
            state.remote_review = None;
            state.remote_selected = false;
        }
        if state
            .comparison
            .as_deref()
            .is_some_and(|value| !valid_mobile_review_comparison(value))
        {
            state.comparison = None;
        }
        let mut bound = HashSet::new();
        state.bindings.retain(|context| {
            valid_mobile_review_binding(context) && bound.insert(context.clone())
        });
        state.bindings.truncate(MOBILE_REVIEW_BINDING_CAP);
        if !matches!(state.mode.as_str(), "files" | "git") {
            state.mode = default_mobile_review_mode();
        }
        if state.reveal.as_ref().is_some_and(|reveal| {
            !valid_mobile_review_path(&reveal.path)
                || reveal.line == Some(0)
                || reveal.revision != state.reveal_revision
        }) {
            state.reveal = None;
        }
        state.tabs.retain(|tab| valid_mobile_review_path(&tab.path));
        let mut seen = HashSet::new();
        state.tabs.retain(|tab| seen.insert(tab.path.clone()));
        if state.tabs.len() > MOBILE_REVIEW_TAB_CAP {
            state.tabs = state
                .tabs
                .split_off(state.tabs.len() - MOBILE_REVIEW_TAB_CAP);
        }
        if state
            .active
            .as_ref()
            .is_some_and(|path| !state.tabs.iter().any(|tab| &tab.path == path))
        {
            state.active = None;
        }
        state.progress.retain(|key, revision| {
            !key.is_empty() && key.len() <= 2048 && !revision.is_empty() && revision.len() <= 512
        });
        while state.progress.len() > MOBILE_REVIEW_PROGRESS_CAP {
            if let Some(key) = state.progress.keys().next().cloned() {
                state.progress.remove(&key);
            }
        }
        state.positions.retain(|path, position| {
            valid_mobile_review_path(path)
                && position.line > 0
                && position
                    .revision
                    .as_ref()
                    .is_none_or(|revision| !revision.is_empty() && revision.len() <= 512)
        });
        while state.positions.len() > MOBILE_REVIEW_POSITION_CAP {
            if let Some(path) = state.positions.keys().next().cloned() {
                state.positions.remove(&path);
            }
        }
        state
    }
}

/// An agent's reveal (`sessions.review_open`): the path and, optionally, the
/// 1-based line to show, with the `reveal_revision` it was written at.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MobileReviewReveal {
    pub(super) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) line: Option<u32>,
    pub(super) revision: u64,
}

/// The optional `line` of an `open`: only a reveal may name one.
pub(super) fn mobile_review_reveal_line(
    args: &serde_json::Value,
    reveal: bool,
) -> Result<Option<u32>, String> {
    match args.get("line") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(_) if !reveal => Err("line needs reveal".to_owned()),
        Some(value) => value
            .as_u64()
            .filter(|line| *line >= 1)
            .and_then(|line| u32::try_from(line).ok())
            .map(Some)
            .ok_or_else(|| "invalid line".to_owned()),
    }
}

pub(super) fn valid_mobile_review_binding(context: &str) -> bool {
    context != "project" && crate::review_directory::valid(context)
}

pub(super) fn valid_mobile_review_comparison(value: &str) -> bool {
    let reference = value.strip_prefix("committed:").unwrap_or(value);
    value.is_empty()
        || (reference.starts_with("refs/")
            && value.len() <= 512
            && !value.chars().any(char::is_control))
}

pub(super) fn valid_mobile_review_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.split('/').any(|part| matches!(part, "" | "." | ".."))
        && !path.contains('\0')
}

pub(super) fn mobile_review_string_arg(
    args: &serde_json::Value,
    name: &str,
    max_len: usize,
) -> Result<String, String> {
    let value = args
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("missing {name}"))?;
    if value.is_empty() || value.len() > max_len || value.contains('\0') {
        return Err(format!("invalid {name}"));
    }
    Ok(value.to_owned())
}

impl MobileReviewState {
    /// `open`: show `path` as the active tab; with `reveal`, record what an
    /// agent asked to show at the `reveal_revision` it bumps.
    pub(super) fn open(&mut self, args: &serde_json::Value) -> Result<(), String> {
        let path = mobile_review_string_arg(args, "path", 4096)?;
        if !valid_mobile_review_path(&path) {
            return Err("invalid mobile review path".to_owned());
        }
        let reveal = args.get("reveal") == Some(&serde_json::Value::Bool(true));
        let line = mobile_review_reveal_line(args, reveal)?;
        if !self.tabs.iter().any(|tab| tab.path == path) {
            if self.tabs.len() >= MOBILE_REVIEW_TAB_CAP {
                let evict = self.tabs.iter().position(|tab| !tab.pinned).unwrap_or(0);
                let removed = self.tabs.remove(evict);
                if self.active.as_deref() == Some(&removed.path) {
                    self.active = None;
                }
            }
            self.tabs.push(MobileReviewTab {
                path: path.clone(),
                pinned: false,
            });
        }
        self.active = Some(path.clone());
        self.mode = "files".to_owned();
        self.open_revision = self.open_revision.saturating_add(1);
        if reveal {
            self.reveal_revision = self.reveal_revision.saturating_add(1);
            self.remote_selected = false;
            self.reveal = Some(MobileReviewReveal {
                path,
                line,
                revision: self.reveal_revision,
            });
        }
        Ok(())
    }
}
