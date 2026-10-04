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
    #[arg(long, requires_all = ["anchor_manifest", "anchor_artifact"])]
    quarantine_damaged_anchor: bool,
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.restore_floor_selection,
        "explicit floor restoration required"
    );
    #[cfg(feature = "machine-host")]
    {
        let mut quarantined = None;
        if let Some(manifest) = &args.anchor_manifest {
            if args.quarantine_damaged_anchor {
                quarantined =
                    crate::machine_components::restore_portable_host_anchor_with_quarantine(
                        &args.state_dir,
                        &args.artifact_public_key,
                        manifest,
                        args.anchor_artifact
                            .as_deref()
                            .expect("paired anchor artifact"),
                    )?;
            } else {
                crate::machine_components::restore_portable_host_anchor(
                    &args.state_dir,
                    &args.artifact_public_key,
                    manifest,
                    args.anchor_artifact
                        .as_deref()
                        .expect("paired anchor artifact"),
                )?;
            }
        }
        crate::machine_components::restore_portable_host_selection(
            &args.state_dir,
            &args.artifact_public_key,
        )?;
        let mut receipt =
            serde_json::json!({"admitted":true,"writer":false,"selected":"floor-anchor"});
        if let Some(path) = quarantined {
            receipt["quarantine"] = serde_json::json!(path);
        }
        println!("{receipt}");
        Ok(())
    }
    #[cfg(not(feature = "machine-host"))]
    anyhow::bail!("floor selection restoration requires a Machine-host installer")
}
