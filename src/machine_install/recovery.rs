//! Explicit offline restoration of missing selection pointers to the floor anchor.

use std::path::PathBuf;

use anyhow::{Result, ensure};
use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    restore_floor_selection: bool,
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long)]
    artifact_public_key: PathBuf,
    #[arg(long, requires = "anchor_artifact")]
    anchor_manifest: Option<PathBuf>,
    #[arg(long, requires = "anchor_manifest")]
    anchor_artifact: Option<PathBuf>,
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.restore_floor_selection,
        "explicit floor restoration required"
    );
    #[cfg(feature = "machine-host")]
    {
        if let Some(manifest) = &args.anchor_manifest {
            crate::machine_components::restore_portable_host_anchor(
                &args.state_dir,
                &args.artifact_public_key,
                manifest,
                args.anchor_artifact
                    .as_deref()
                    .expect("paired anchor artifact"),
            )?;
        }
        crate::machine_components::restore_portable_host_selection(
            &args.state_dir,
            &args.artifact_public_key,
        )?;
        println!("{{\"admitted\":true,\"writer\":false,\"selected\":\"floor-anchor\"}}");
        Ok(())
    }
    #[cfg(not(feature = "machine-host"))]
    anyhow::bail!("floor selection restoration requires a Machine-host installer")
}
