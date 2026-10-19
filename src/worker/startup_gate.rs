//! A named systemd worker cannot initialize its Provider before its Machine
//! has durably observed the unit incarnation. EOF is cancellation, not consent.
use anyhow::{Context as _, Result, ensure};
use std::io::Read;

pub use crate::runtime_wire::WORKER_LAUNCH_ENV as LAUNCH_ENV;

pub fn wait(input: &mut impl Read, launch: &str) -> Result<()> {
    let launch: uuid::Uuid = launch.parse().context("invalid worker launch identity")?;
    ensure!(!launch.is_nil(), "empty worker launch identity");
    let mut token = [0; 37];
    input
        .read_exact(&mut token)
        .context("worker launch admission disconnected")?;
    ensure!(
        token.as_slice() == format!("{launch}\n").as_bytes(),
        "worker launch admission mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_gate_requires_exact_launch_and_complete_token() {
        let launch = uuid::Uuid::new_v4().to_string();
        let token = format!("{launch}\n");
        wait(&mut token.as_bytes(), &launch).unwrap();
        for length in 0..token.len() {
            assert!(wait(&mut &token.as_bytes()[..length], &launch).is_err());
        }
        assert!(wait(&mut token.as_bytes(), &uuid::Uuid::new_v4().to_string()).is_err());
        assert!(wait(&mut token.as_bytes(), &uuid::Uuid::nil().to_string()).is_err());
    }
}
