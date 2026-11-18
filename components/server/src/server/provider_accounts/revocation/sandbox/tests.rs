//! The sandbox removal of a revoked named-only account: required on every
//! Machine that may hold it, recorded only from a confirmed receipt of
//! exactly this revocation's request, and required for completion.
use super::*;
use crate::store::provider_accounts::revocation::account_removals::{missing, required_machines};

#[derive(Default)]
struct Fake {
    connected: parking_lot::Mutex<bool>,
    answer: parking_lot::Mutex<Option<Result<RemoveObservation, String>>>,
    asked: parking_lot::Mutex<Vec<(String, RemoveRequest)>>,
    enrollment: parking_lot::Mutex<Option<String>>,
}

impl Remover for Fake {
    fn connection(&self, _machine: &str) -> Option<Box<dyn Fn() -> bool + Send + Sync>> {
        (*self.connected.lock()).then(|| Box::new(|| true) as Box<dyn Fn() -> bool + Send + Sync>)
    }

    fn enrollment(&self, _machine: &str) -> futures::future::BoxFuture<'static, Option<String>> {
        Box::pin(std::future::ready(self.enrollment.lock().clone()))
    }

    fn remove(
        &self,
        machine: &str,
        request: &RemoveRequest,
    ) -> futures::future::BoxFuture<'static, Result<RemoveObservation, String>> {
        self.asked
            .lock()
            .push((machine.to_owned(), request.clone()));
        let answer = self
            .answer
            .lock()
            .clone()
            .unwrap_or_else(|| Err("account_remove_unsupported".into()));
        Box::pin(std::future::ready(answer))
    }
}

async fn revoking(provider: &str) -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    super::super::native::tests::fixture_for(&store, provider).await;
    (store, dir)
}

#[tokio::test]
async fn a_revoked_named_only_account_completes_only_after_its_sandbox_is_removed() {
    let (store, _dir) = revoking("antigravity").await;
    let receipt = store
        .provider_account_receipt("owner", "revoke")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        required_machines(&receipt).into_iter().collect::<Vec<_>>(),
        ["machine"]
    );
    let fake = Fake::default();
    let key = store
        .machine_encryption_public_key("machine")
        .await
        .unwrap()
        .unwrap();
    *fake.enrollment.lock() = Some(crate::admin::hex_sha256(key.as_bytes()));
    // Offline, or a Machine that cannot remove accounts yet: pending.
    let pending = run(&store, &fake, "owner", "revoke").await.unwrap();
    assert_eq!(pending[0].state, State::Pending);
    *fake.connected.lock() = true;
    let pending = run(&store, &fake, "owner", "revoke").await.unwrap();
    assert_eq!(pending[0].state, State::Pending);
    let request = fake.asked.lock()[0].1.clone();
    // Every retry repeats the same journaled request.
    run(&store, &fake, "owner", "revoke").await.unwrap();
    assert!(
        fake.asked
            .lock()
            .iter()
            .all(|(machine, asked)| machine == "machine" && *asked == request)
    );
    let readiness = store
        .provider_revocation_readiness("owner", "revoke")
        .await
        .unwrap();
    assert!(
        readiness
            .missing
            .iter()
            .any(|missing| missing.kind == "account_removal" && missing.target == "machine")
    );
    // A receipt that does not confirm both removals, or answers another
    // request, records nothing.
    for observation in [
        RemoveObservation {
            request: request.clone(),
            home_removed: true,
            user_removed: false,
        },
        RemoveObservation {
            request: RemoveRequest {
                operation_id: "account-remove-other".into(),
                ..request.clone()
            },
            home_removed: true,
            user_removed: true,
        },
    ] {
        *fake.answer.lock() = Some(Ok(observation));
        let unconfirmed = run(&store, &fake, "owner", "revoke").await.unwrap();
        assert_eq!(unconfirmed[0].state, State::Unconfirmed);
    }
    let receipt = store
        .provider_account_receipt("owner", "revoke")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(missing(&receipt), ["machine"]);
    assert!(
        !store
            .complete_provider_revocation("owner", "revoke")
            .await
            .unwrap()
    );
    // A re-enrolled Machine never certifies the removal.
    *fake.answer.lock() = Some(Ok(RemoveObservation {
        request: request.clone(),
        home_removed: true,
        user_removed: true,
    }));
    let enrolled = fake.enrollment.lock().replace("another-enrollment".into());
    let refused = run(&store, &fake, "owner", "revoke").await.unwrap();
    assert_eq!(refused[0].state, State::Pending);
    assert!(
        refused[0]
            .detail
            .as_deref()
            .unwrap()
            .contains("machine_enrollment_changed")
    );
    *fake.enrollment.lock() = enrolled;
    // The confirmed receipt is recorded once; later passes do not ask again.
    let removed = run(&store, &fake, "owner", "revoke").await.unwrap();
    assert_eq!(removed[0].state, State::Removed);
    let asked = fake.asked.lock().len();
    run(&store, &fake, "owner", "revoke").await.unwrap();
    assert_eq!(fake.asked.lock().len(), asked);
    let receipt = store
        .provider_account_receipt("owner", "revoke")
        .await
        .unwrap()
        .unwrap();
    assert!(missing(&receipt).is_empty());
}

#[tokio::test]
async fn other_providers_need_no_sandbox_removal() {
    let (store, _dir) = revoking("claude-code").await;
    let receipt = store
        .provider_account_receipt("owner", "revoke")
        .await
        .unwrap()
        .unwrap();
    assert!(required_machines(&receipt).is_empty());
    let fake = Fake::default();
    assert!(
        run(&store, &fake, "owner", "revoke")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(fake.asked.lock().is_empty());
}
