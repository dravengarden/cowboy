//! Hold only real, validated read-only preflight replies. No synthetic ACK,
//! uninstall effect, credential sync, or session command is permitted.
use super::*;
use crate::machine_protocol::plugin_step::StepLookup;
use tokio::sync::Notify;

#[derive(Clone, Copy)]
pub(in super::super) enum Kind {
    Install,
    Uninstall,
}

#[derive(Clone, Default)]
pub(in super::super) struct Gate {
    pub(super) reached: Arc<Notify>,
    pub(super) resume: Arc<Notify>,
}

impl Gate {
    pub async fn held(&self) -> Result<(), Failure> {
        tokio::time::timeout(DEADLINE, self.reached.notified())
            .await
            .map_err(|_| Failure::Timeout)
    }

    pub fn release(&self) {
        self.resume.notify_one();
    }
}

pub(super) struct Probe {
    kind: Kind,
    expected: Option<(String, Option<String>)>,
    pub gate: Gate,
    pub received: bool,
}

impl Probe {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            expected: None,
            gate: Gate::default(),
            received: false,
        }
    }

    pub fn command(&mut self, command: &MachineCommand) -> Result<bool, Failure> {
        let (id, digest) = match (self.kind, command) {
            (Kind::Install, MachineCommand::ObservePluginInstallation { request_id, query }) => {
                check(
                    query.service_id == SERVICE
                        && query.machine_id == MACHINE
                        && query.plugin_id == "victoria",
                )?;
                (
                    request_id,
                    Some(query.digest().map_err(|_| Failure::WrongObservation)?),
                )
            }
            (Kind::Uninstall, MachineCommand::QueryPluginUninstallStep { request_id, step }) => {
                check(
                    step.schema == 2
                        && step.service_id == SERVICE
                        && step.machine_id == MACHINE
                        && step.plugin_id == "victoria"
                        && step.plugin_version == "1.1.0",
                )?;
                step.validate().map_err(|_| Failure::WrongObservation)?;
                (request_id, None)
            }
            _ => return Ok(false),
        };
        check(self.expected.is_none() && !self.received)?;
        self.expected = Some((id.clone(), digest));
        Ok(true)
    }

    pub fn event(&mut self, event: &MachineEvent) -> Result<bool, Failure> {
        let (id, digest) = match (self.kind, event) {
            (
                Kind::Install,
                MachineEvent::PluginInstallationTarget {
                    request_id,
                    observation,
                },
            ) => {
                let InstallTargetObservation::Observed {
                    query_digest,
                    target,
                    admission_enabled: true,
                } = observation.as_ref()
                else {
                    return Err(Failure::WrongObservation);
                };
                target.validate().map_err(|_| Failure::WrongObservation)?;
                (request_id, Some(query_digest.clone()))
            }
            (
                Kind::Uninstall,
                MachineEvent::PluginUninstallStep {
                    request_id,
                    observation,
                },
            ) => {
                check(
                    observation.admission_enabled && observation.result == StepLookup::NotFound {},
                )?;
                // NotFound is correlated by request ID, not a receipt proof.
                (request_id, None)
            }
            _ => return Ok(false),
        };
        check(!self.received && self.expected.take().as_ref() == Some(&(id.clone(), digest)))?;
        self.received = true;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::plugin_install::InstallTargetQuery;
    use crate::machine_protocol::plugin_step::StepObservation;

    fn frames(kind: Kind) -> (MachineCommand, MachineEvent) {
        let request_id = "catalog-probe-test".to_owned();
        match kind {
            Kind::Install => {
                let query = InstallTargetQuery {
                    schema: 1,
                    service_id: SERVICE.into(),
                    machine_id: MACHINE.into(),
                    plugin_id: "victoria".into(),
                };
                let observation = InstallTargetObservation::Observed {
                    admission_enabled: true,
                    query_digest: query.digest().unwrap(),
                    target: InstallTarget::Vacant {},
                };
                (
                    MachineCommand::ObservePluginInstallation {
                        request_id: request_id.clone(),
                        query: Box::new(query),
                    },
                    MachineEvent::PluginInstallationTarget {
                        request_id,
                        observation: Box::new(observation),
                    },
                )
            }
            Kind::Uninstall => {
                let mut step = crate::machine_protocol::plugin_step::fixture();
                step.schema = 2;
                step.service_id = SERVICE.into();
                step.machine_id = MACHINE.into();
                step.plugin_version = "1.1.0".into();
                step.installation_revision = Some(
                    format!("installation-{}", "a".repeat(64))
                        .try_into()
                        .unwrap(),
                );
                (
                    MachineCommand::QueryPluginUninstallStep {
                        request_id: request_id.clone(),
                        step: Box::new(step),
                    },
                    MachineEvent::PluginUninstallStep {
                        request_id,
                        observation: Box::new(StepObservation {
                            admission_enabled: true,
                            result: StepLookup::NotFound {},
                        }),
                    },
                )
            }
        }
    }

    #[test]
    fn catalog_probe_only_holds_one_correlated_real_preflight_reply() {
        for kind in [Kind::Install, Kind::Uninstall] {
            let (command, event) = frames(kind);
            let mut probe = Probe::new(kind);
            assert!(probe.event(&event).is_err());
            assert!(probe.command(&command).unwrap());
            assert!(probe.command(&command).is_err());
            assert!(probe.event(&event).unwrap());
            assert!(probe.event(&event).is_err());
            assert!(probe.command(&command).is_err());
        }
    }

    #[test]
    fn catalog_probe_refuses_unmatched_and_reader_only_preflight_replies() {
        for kind in [Kind::Install, Kind::Uninstall] {
            for field in ["request_id", "admission_enabled"] {
                let (command, event) = frames(kind);
                let mut value = serde_json::to_value(event).unwrap();
                if field == "request_id" {
                    value[field] = json!("other-request");
                } else {
                    value["observation"][field] = json!(false);
                }
                let changed = serde_json::from_value(value).unwrap();
                let mut probe = Probe::new(kind);
                assert!(probe.command(&command).unwrap());
                assert!(probe.event(&changed).is_err());
            }
        }
    }
}
