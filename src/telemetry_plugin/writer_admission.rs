//! Independent host admission for one exact Service/Machine pair. This policy
//! is neither Operator authority nor a connection lease, binding or export grant.
use super::{PrivateSnapshot, read_private_snapshot};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(feature = "machine-host")]
pub(crate) const MACHINE_POLICY_FILE: &str = "telemetry-writer-policy.json";

#[cfg(feature = "machine-host")]
pub(crate) mod preflight;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    schema: u16,
    service_id: String,
    machine_id: String,
    purposes: Purposes,
    legacy_fence: FenceAcknowledgment,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "machine-host", derive(Debug, Clone, serde::Serialize))]
#[serde(deny_unknown_fields)]
struct Purposes {
    binding: bool,
    machine_recovery: bool,
    service_resolution: bool,
}

struct FenceAcknowledgment;
impl<'de> Deserialize<'de> for FenceAcknowledgment {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        if String::deserialize(decoder)? == "retain_managed_namespace" {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(
                "explicit managed namespace acknowledgment required",
            ))
        }
    }
}

// Closed nominal purposes: no caller-supplied string or cast between scopes.
mod sealed {
    pub trait Sealed {}
}
pub(crate) trait Purpose: sealed::Sealed + Send + Sync {
    const INDEX: usize;
}
pub(crate) struct BindingWrites;
pub(crate) struct MachineRecovery;
#[cfg_attr(not(feature = "full"), allow(dead_code))]
pub(crate) struct ServiceResolution;
impl sealed::Sealed for BindingWrites {}
impl sealed::Sealed for MachineRecovery {}
impl sealed::Sealed for ServiceResolution {}
impl Purpose for BindingWrites {
    const INDEX: usize = 0;
}
impl Purpose for MachineRecovery {
    const INDEX: usize = 1;
}
impl Purpose for ServiceResolution {
    const INDEX: usize = 2;
}

pub(crate) struct WriterAdmission {
    path: PathBuf,
    config: Configuration,
    snapshot: PrivateSnapshot,
    stopped: AtomicBool,
}

impl WriterAdmission {
    pub(crate) fn load(path: &Path) -> Result<Arc<Self>> {
        ensure!(
            path.is_absolute(),
            "telemetry writer policy must be absolute"
        );
        let (config, snapshot) = read_private_snapshot::<Configuration>(path)?;
        ensure!(
            config.schema == 1
                && crate::machine_protocol::telemetry_binding::valid_service(&config.service_id)
                && (1..=128).contains(&config.machine_id.len())
                && config
                    .machine_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
            "invalid telemetry writer policy owner or schema"
        );
        let FenceAcknowledgment = config.legacy_fence;
        Ok(Arc::new(Self {
            path: path.to_owned(),
            config,
            snapshot,
            stopped: AtomicBool::new(false),
        }))
    }

    #[cfg(feature = "machine-host")]
    pub(crate) fn load_optional(path: &Path) -> Result<Option<Arc<Self>>> {
        // Only actual absence is optional. A symlink, corrupt policy or access
        // failure is not silently treated as a missing host instruction.
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
            Ok(_) => Self::load(path).map(Some),
        }
    }

    pub(crate) fn owns_service(&self, service: &str) -> bool {
        self.config.service_id == service
    }

    pub(crate) fn target_matches(&self, machine: &str) -> bool {
        self.config.machine_id == machine
    }

    pub(crate) fn allows<P: Purpose>(&self, service: &str, machine: Option<&str>) -> bool {
        self.owns_service(service)
            && machine.is_none_or(|m| self.target_matches(m))
            && [
                self.config.purposes.binding,
                self.config.purposes.machine_recovery,
                self.config.purposes.service_resolution,
            ][P::INDEX]
            && self.current()
    }

    fn current(&self) -> bool {
        let valid = !self.stopped.load(Ordering::Acquire)
            && read_private_snapshot::<Configuration>(&self.path)
                .is_ok_and(|(_, current)| self.snapshot.matches(&current));
        if !valid {
            self.stopped.store(true, Ordering::Release);
        }
        valid
    }

    pub(crate) fn scope<P: Purpose>(
        self: &Arc<Self>,
        service: &str,
        machine: &str,
    ) -> Option<WriteScope<P>> {
        self.allows::<P>(service, Some(machine))
            .then(|| WriteScope {
                owner: ScopeOwner::Policy(self.clone()),
                purpose: PhantomData,
            })
    }
}

enum ScopeOwner {
    Policy(Arc<WriterAdmission>),
    #[cfg(test)]
    Fixture,
}

// Intentionally not Clone, Debug, Serialize or Deserialize. It never renews the
// separately required original Operator/connection/monotonic execution budget.
pub(crate) struct WriteScope<P: Purpose> {
    owner: ScopeOwner,
    purpose: PhantomData<P>,
}
impl<P: Purpose> WriteScope<P> {
    #[cfg(feature = "full")]
    pub(crate) fn check_for(&self, service: &str, machine: &str) -> bool {
        match &self.owner {
            ScopeOwner::Policy(policy) => policy.allows::<P>(service, Some(machine)),
            #[cfg(test)]
            ScopeOwner::Fixture => true,
        }
    }

    #[cfg(any(test, feature = "machine-host"))]
    pub(crate) fn check(&self) -> bool {
        match &self.owner {
            ScopeOwner::Policy(policy) => policy.current(),
            #[cfg(test)]
            ScopeOwner::Fixture => true,
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        Self {
            owner: ScopeOwner::Fixture,
            purpose: PhantomData,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
