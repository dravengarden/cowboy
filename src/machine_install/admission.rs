//! Signed refresh preserves an existing authenticated floor and cache selection.
//! Missing-cache recovery and committed deletion admission remain closed.

use std::ffi::OsString;
use std::path::Path;

use anyhow::{Result, ensure};

use super::InstallArgs;
use crate::session_deletion_admission::{self, reader_floor};

pub(super) struct Admission {
    floor: Option<Evidence>,
}

#[derive(PartialEq, Eq)]
struct Evidence {
    bytes: Vec<u8>,
    active: OsString,
    command: OsString,
}

impl Admission {
    pub(super) fn check(args: &InstallArgs, state: &Path) -> Result<Self> {
        session_deletion_admission::require_empty_portable_namespace(state)?;
        match std::fs::symlink_metadata(state.join(reader_floor::NAME)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { floor: None });
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        ensure!(
            args.refresh && args.bootstrap_manifest.is_some(),
            "portable reader floor requires an authenticated signed refresh"
        );
        #[cfg(not(feature = "machine-host"))]
        {
            anyhow::bail!("portable reader floor signed refresh requires a Machine-host installer");
        }
        #[cfg(feature = "machine-host")]
        {
            use anyhow::Context as _;
            reader_floor::read(state)?.context("portable reader floor disappeared")?;
            let key = args
                .artifact_public_key
                .as_deref()
                .context("portable reader floor requires publisher key")?;
            super::signed_bootstrap::authenticate(
                args.bootstrap_manifest.as_deref().expect("signed refresh"),
                args.bootstrap_artifact
                    .as_deref()
                    .context("signed refresh requires artifact")?,
                key,
            )?;
            // Checks publisher binding, retained anchor and both selected pointers
            // without creating stores, executing cached code or contacting peers.
            crate::machine_components::check_portable_host_cache(state, Some(key))?;
            Ok(Self {
                floor: Some(Evidence {
                    bytes: super::signed_bootstrap::read_regular(
                        &state.join(reader_floor::NAME),
                        8192,
                    )?,
                    active: std::fs::read_link(state.join("components/active/machine_host"))?
                        .into_os_string(),
                    command: std::fs::read_link(state.join("components/commands/cowboy-machine"))?
                        .into_os_string(),
                }),
            })
        }
    }

    pub(super) fn recheck(&self, args: &InstallArgs, state: &Path) -> Result<()> {
        let current = Self::check(args, state)?;
        ensure!(
            current.floor == self.floor,
            "portable reader floor or selection changed during bootstrap installation"
        );
        Ok(())
    }
}
