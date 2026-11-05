//! The runtime protocol range the Controller core accepts from a Machine broker.

use anyhow::Result;

use crate::runtime_wire::PROTOCOL_VERSION;

/// The oldest runtime protocol the Controller core accepts from a Machine
/// broker. The wire crate's `MIN_PROTOCOL_VERSION` stays 1 because it is
/// pinned to the retained worker. Measured 2026-11-05: every retained worker
/// pin since 2026-10-19 and every Machine release speaks 5
/// (docs/compatibility-exits.md).
pub(super) const BROKER_PROTOCOL_FLOOR: u16 = 5;

pub(super) fn validate_protocol(protocol: u16) -> Result<()> {
    anyhow::ensure!(
        (BROKER_PROTOCOL_FLOOR..=PROTOCOL_VERSION).contains(&protocol),
        "Machine broker selected an unsupported runtime protocol"
    );
    Ok(())
}
