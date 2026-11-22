//! Durable per-recipient delivery acknowledgements of one credential scope:
//! the newest generation of each Provider every Machine applied. Every
//! successful delivery records one; the Controller's reconciler resends a
//! generation to each connected Machine whose acknowledgement is missing and
//! recomputes the scope's distribution state from them, and the legacy card
//! retirement's startup gate reads them (`docs/legacy-card-retirement.md`).
use super::*;

const DELIVERIES: &str = "deliveries.json";

/// A credential recipient: an enrolled Machine and its enrollment identity
/// (re-enrolling a Machine id gives it a new one, so its acknowledgements
/// do not carry over).
pub type Recipient = (String, String);

/// One Machine's acknowledgement: the generation it applied and the
/// enrollment that applied it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub generation: u64,
    pub enrollment: String,
}

type Acknowledgements = BTreeMap<String, BTreeMap<String, Delivery>>;

impl ProviderAuthService {
    fn read_deliveries(&self) -> Acknowledgements {
        fs::read(self.root.join(DELIVERIES))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write_deliveries(&self, deliveries: &Acknowledgements) -> Result<()> {
        atomic_write(
            &self.root.join(DELIVERIES),
            &serde_json::to_vec(deliveries)?,
            0o600,
        )
    }

    /// Each Machine's acknowledgement of `provider_id`.
    pub fn deliveries(&self, provider_id: &str) -> BTreeMap<String, Delivery> {
        let _serialized = self.providers.read();
        self.read_deliveries()
            .remove(provider_id)
            .unwrap_or_default()
    }

    /// `machine_id` (enrolled as `enrollment`) applied `generation` of
    /// `provider_id`. A same-enrollment acknowledgement is never lowered.
    pub fn record_delivery(
        &self,
        provider_id: &str,
        (machine_id, enrollment): (&str, &str),
        generation: u64,
    ) -> Result<()> {
        let _serialized = self.providers.write();
        let mut deliveries = self.read_deliveries();
        let entries = deliveries.entry(provider_id.to_owned()).or_default();
        if entries
            .get(machine_id)
            .is_some_and(|known| known.enrollment == enrollment && known.generation >= generation)
        {
            return Ok(());
        }
        entries.insert(
            machine_id.to_owned(),
            Delivery {
                generation,
                enrollment: enrollment.to_owned(),
            },
        );
        self.write_deliveries(&deliveries)
    }

    /// Fresh evidence contradicts `machine_id`'s acknowledgement of
    /// `provider_id` (it reports the generation failed, or an older one):
    /// forget it, so the generation is delivered again.
    pub fn forget_delivery(&self, provider_id: &str, machine_id: &str) -> Result<()> {
        let _serialized = self.providers.write();
        let mut deliveries = self.read_deliveries();
        let Some(entries) = deliveries.get_mut(provider_id) else {
            return Ok(());
        };
        if entries.remove(machine_id).is_none() {
            return Ok(());
        }
        self.write_deliveries(&deliveries)
    }

    /// The recipients that have not acknowledged the current generation of
    /// `provider_id` with their current enrollment.
    pub fn unacknowledged(&self, provider_id: &str, recipients: &[Recipient]) -> Vec<String> {
        let Some(generation) = self
            .status(provider_id)
            .map(|status| status.auth_generation)
        else {
            return Vec::new();
        };
        let acknowledged = self.deliveries(provider_id);
        recipients
            .iter()
            .filter(|(machine, enrollment)| {
                acknowledged.get(machine).is_none_or(|known| {
                    &known.enrollment != enrollment || known.generation < generation
                })
            })
            .map(|(machine, _)| machine.clone())
            .collect()
    }

    /// Recompute the distribution state of `provider_id`'s current generation
    /// from the acknowledgements of `recipients` (the enrolled Machines);
    /// `failed` are recipients whose latest delivery failed. Written only on
    /// a change. Returns the status when it changed.
    pub fn reconcile_distribution(
        &self,
        provider_id: &str,
        recipients: &[Recipient],
        failed: &BTreeSet<String>,
    ) -> Result<Option<ProviderAuthenticationStatus>> {
        let Some(status) = self.status(provider_id) else {
            return Ok(None);
        };
        let missing = self.unacknowledged(provider_id, recipients);
        let acknowledged = recipients.len() - missing.len();
        let state = if missing.is_empty() {
            ServiceDistributionState::Current
        } else if missing.iter().any(|machine| failed.contains(machine)) {
            if acknowledged == 0 {
                ServiceDistributionState::Failed
            } else {
                ServiceDistributionState::Partial
            }
        } else if acknowledged == 0 {
            ServiceDistributionState::Pending
        } else {
            ServiceDistributionState::Partial
        };
        if status.distribution_state == state {
            return Ok(None);
        }
        self.mark_distribution(provider_id, status.auth_generation, state)
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grok() -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../../../plugins/grok/provider.json")).unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    #[test]
    fn acknowledgements_drive_the_distribution_state() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let package = grok();
        let bundle = PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: "xai-account".into(),
            values: BTreeMap::from([(
                "auth_json".into(),
                base64::engine::general_purpose::STANDARD.encode(b"{}"),
            )]),
        };
        service.commit(&package, &bundle, None, Some(0)).unwrap();
        let wiped = service.logout("grok").unwrap();
        let recipients = [
            ("hawk".to_owned(), "k1".to_owned()),
            ("ovh".to_owned(), "k2".to_owned()),
        ];
        assert_eq!(service.unacknowledged("grok", &recipients), ["hawk", "ovh"]);
        // An older generation's acknowledgement is no acknowledgement.
        service.record_delivery("grok", ("hawk", "k1"), 1).unwrap();
        let failed = BTreeSet::from(["ovh".to_owned()]);
        assert_eq!(
            service
                .reconcile_distribution("grok", &recipients, &failed)
                .unwrap()
                .unwrap()
                .distribution_state,
            ServiceDistributionState::Failed
        );
        service
            .record_delivery("grok", ("hawk", "k1"), wiped.auth_generation)
            .unwrap();
        assert_eq!(
            service
                .reconcile_distribution("grok", &recipients, &failed)
                .unwrap()
                .unwrap()
                .distribution_state,
            ServiceDistributionState::Partial
        );
        service
            .record_delivery("grok", ("ovh", "k2"), wiped.auth_generation)
            .unwrap();
        // Never lowered.
        service.record_delivery("grok", ("ovh", "k2"), 1).unwrap();
        assert_eq!(
            service.deliveries("grok")["ovh"].generation,
            wiped.auth_generation
        );
        let current = service
            .reconcile_distribution("grok", &recipients, &BTreeSet::new())
            .unwrap()
            .unwrap();
        assert_eq!(
            current.distribution_state,
            ServiceDistributionState::Current
        );
        // Unchanged: nothing written.
        assert!(
            service
                .reconcile_distribution("grok", &recipients, &BTreeSet::new())
                .unwrap()
                .is_none()
        );
        // Durable across reopen.
        drop(service);
        let reopened = ProviderAuthService::open(root.path()).unwrap();
        assert!(reopened.unacknowledged("grok", &recipients).is_empty());
        // A re-enrolled Machine id starts unacknowledged.
        let reenrolled = [("ovh".to_owned(), "k3".to_owned())];
        assert_eq!(reopened.unacknowledged("grok", &reenrolled), ["ovh"]);
        // Contradicting evidence forgets an acknowledgement.
        reopened.forget_delivery("grok", "hawk").unwrap();
        assert_eq!(reopened.unacknowledged("grok", &recipients), ["hawk"]);
    }
}
