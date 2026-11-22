//! Durable per-recipient delivery acknowledgements of one credential scope:
//! the newest generation of each Provider every Machine applied. Every
//! successful delivery records one; the Controller's reconciler resends a
//! generation to each connected Machine whose acknowledgement is missing and
//! recomputes the scope's distribution state from them, and the legacy card
//! retirement's startup gate reads them (`docs/legacy-card-retirement.md`).
use super::*;

const DELIVERIES: &str = "deliveries.json";

type Acknowledgements = BTreeMap<String, BTreeMap<String, u64>>;

impl ProviderAuthService {
    fn read_deliveries(&self) -> Acknowledgements {
        fs::read(self.root.join(DELIVERIES))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// The generation of `provider_id` each Machine acknowledged.
    pub fn deliveries(&self, provider_id: &str) -> BTreeMap<String, u64> {
        let _serialized = self.providers.read();
        self.read_deliveries()
            .remove(provider_id)
            .unwrap_or_default()
    }

    /// `machine_id` applied `generation` of `provider_id` (never lowered).
    pub fn record_delivery(
        &self,
        provider_id: &str,
        machine_id: &str,
        generation: u64,
    ) -> Result<()> {
        let _serialized = self.providers.write();
        let mut deliveries = self.read_deliveries();
        let acknowledged = deliveries
            .entry(provider_id.to_owned())
            .or_default()
            .entry(machine_id.to_owned())
            .or_default();
        if *acknowledged >= generation {
            return Ok(());
        }
        *acknowledged = generation;
        atomic_write(
            &self.root.join(DELIVERIES),
            &serde_json::to_vec(&deliveries)?,
            0o600,
        )
    }

    /// The enrolled Machines (`recipients`) that have not acknowledged the
    /// current generation of `provider_id`.
    pub fn unacknowledged(&self, provider_id: &str, recipients: &[String]) -> Vec<String> {
        let Some(generation) = self
            .status(provider_id)
            .map(|status| status.auth_generation)
        else {
            return Vec::new();
        };
        let acknowledged = self.deliveries(provider_id);
        recipients
            .iter()
            .filter(|machine| {
                acknowledged
                    .get(*machine)
                    .is_none_or(|acked| *acked < generation)
            })
            .cloned()
            .collect()
    }

    /// Recompute the distribution state of `provider_id`'s current generation
    /// from the acknowledgements of `recipients` (the enrolled Machines);
    /// `failed` are recipients whose latest delivery failed. Written only on
    /// a change. Returns the status when it changed.
    pub fn reconcile_distribution(
        &self,
        provider_id: &str,
        recipients: &[String],
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
        let recipients = ["hawk".to_owned(), "ovh".to_owned()];
        assert_eq!(service.unacknowledged("grok", &recipients), recipients);
        // An older generation's acknowledgement is no acknowledgement.
        service.record_delivery("grok", "hawk", 1).unwrap();
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
            .record_delivery("grok", "hawk", wiped.auth_generation)
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
            .record_delivery("grok", "ovh", wiped.auth_generation)
            .unwrap();
        // Never lowered.
        service.record_delivery("grok", "ovh", 1).unwrap();
        assert_eq!(service.deliveries("grok")["ovh"], wiped.auth_generation);
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
    }
}
