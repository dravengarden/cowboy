//! Activity indicator compilation and SDK floors for standard Providers.

use anyhow::{Result, ensure};

use super::{ActivityIndicator, StandardActivityIndicator, UiContract, UiNode};

impl StandardActivityIndicator {
    /// The renderer-owned indicator for this authored one; mark strategies
    /// animate the Provider's loading asset.
    pub(super) fn compile(&self, loading_asset: &str) -> ActivityIndicator {
        let asset = loading_asset.to_owned();
        match self {
            Self::ProgressRing => ActivityIndicator::ProgressRing,
            Self::GlyphCycle {
                frames,
                interval_ms,
            } => ActivityIndicator::GlyphCycle {
                frames: frames.clone(),
                interval_ms: *interval_ms,
            },
            Self::TerminalPrompt { interval_ms } => ActivityIndicator::TerminalPrompt {
                interval_ms: *interval_ms,
            },
            Self::MarkSignal { interval_ms } => ActivityIndicator::AssetSignal {
                asset,
                interval_ms: *interval_ms,
            },
            Self::MarkPulse { interval_ms } => ActivityIndicator::AssetPulse {
                asset,
                interval_ms: *interval_ms,
            },
            Self::MarkLevitate { interval_ms } => ActivityIndicator::AssetLevitate {
                asset,
                interval_ms: *interval_ms,
            },
        }
    }
}

impl UiContract {
    /// Whether any surface shows the levitating activity (Provider SDK 3.7.0).
    pub(super) fn uses_levitating_activity(&self) -> bool {
        fn walk(node: &UiNode) -> bool {
            match node {
                UiNode::Activity { indicator, .. } => {
                    matches!(indicator, ActivityIndicator::AssetLevitate { .. })
                }
                UiNode::Stack { children, .. } => children.iter().any(walk),
                _ => false,
            }
        }
        self.surfaces.values().any(walk)
    }
}

/// A package showing the levitating activity names SDK 3.7.0 or newer, so
/// older Controllers, Webs and Machines skip it instead of misreading it.
pub(super) fn validate_sdk_floor(ui: &UiContract, sdk_version: &str) -> Result<()> {
    if ui.uses_levitating_activity() {
        ensure!(
            semver::Version::parse(sdk_version)? >= semver::Version::new(3, 7, 0),
            "levitating activity requires Provider SDK 3.7.0 or newer"
        );
    }
    Ok(())
}
