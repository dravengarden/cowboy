//! The kill switch's history-preserving logout against a real signed-in
//! native login: refused before a confirmed stop, durable and pending while
//! the Machine is offline, idempotent per block, and recorded on the login
//! receipt so the generation admits no session afterwards.
use super::*;
use crate::account_controls::LogoutTargetState;
use crate::store::provider_accounts::{
    Change, Mutation, MutationResult,
    logins::{DispatchScope, NativeCompletion, Request},
};

const OWNER: &str = "owner";
const MACHINE: &str = "ovh";

/// A bound native sign-in of a new account on [`MACHINE`].
async fn signed_in(store: &Store) -> (ProviderAccountId, NativeCompletion) {
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(
            OWNER,
            "create",
            &Mutation {
                expected_revision: 0,
                change: Change::Create {
                    provider_id: "claude-code".into(),
                    label: "Personal".into(),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("create")
    };
    let key = base64::engine::general_purpose::STANDARD.encode(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([23; 32])).as_bytes(),
    );
    let enrollment = store
        .create_machine_enrollment(MACHINE, "Native", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    let input = Request {
        account_id: created.account_id.clone(),
        machine_id: MACHINE.into(),
        provider_version: "1".into(),
        generation_digest: "digest".into(),
        method: "native".into(),
    };
    let claim = store
        .claim_native_provider_login(OWNER, "login", &input)
        .await
        .unwrap();
    let fingerprint = crate::admin::hex_sha256(key.as_bytes());
    store
        .with_provider_login_dispatch(
            OWNER,
            "login",
            &claim,
            &DispatchScope {
                registry_revision: created.registry.revision,
                recipient_fingerprint: fingerprint.clone(),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    let completion = NativeCompletion {
        machine_id: MACHINE.into(),
        recipient_fingerprint: fingerprint,
        profile: crate::provider_account::NativeProfileReceipt {
            account_id: input.account_id.clone(),
            provider_id: "claude-code".into(),
            request_id: claim.receipt.request_id.clone(),
            generation: claim.receipt.native_generation.unwrap(),
            method: input.method.clone(),
            provider_version: input.provider_version.clone(),
            generation_digest: input.generation_digest.clone(),
            auth_contract_fingerprint: "contract".into(),
            identity: crate::provider_account::NativeIdentity {
                authority: "fixture".into(),
                subject: "personal".into(),
                tenant: None,
            },
        },
    };
    store
        .record_native_login_completion(OWNER, "login", &completion)
        .await
        .unwrap();
    store
        .bind_native_login_identity(OWNER, "login", &completion, created.registry.revision)
        .await
        .unwrap();
    (created.account_id, completion)
}

async fn settle_logouts(controls: &Controls) {
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        if controls.cache.fencing.lock().is_empty() {
            break;
        }
    }
    controls.refresh().await.unwrap();
}

#[tokio::test]
async fn logout_after_a_confirmed_kill_switch_keeps_history_and_waits_for_the_machine() {
    let (fake, controls, _root) = fixture().await;
    let (account, completion) = signed_in(&controls.store).await;
    let store = controls.store.clone();
    assert!(
        store
            .native_login_for_session(OWNER, &account, MACHINE)
            .await
            .unwrap()
            .is_some()
    );
    let ok = initial(account.as_str(), OWNER, "claude-code");
    // Not blocked: refused.
    assert_eq!(
        controls.request_logout(&ok, OWNER, true).await,
        Err("account_not_blocked")
    );
    let blocked = controls
        .enter(&ok, AccountState::Blocked, "owner_block", None, None, false)
        .await
        .unwrap();
    // Blocked, but the Machine is offline: no confirmed stop yet.
    assert_eq!(
        controls.request_logout(&blocked, OWNER, true).await,
        Err("account_stop_unconfirmed")
    );
    fake.connected.lock().insert(MACHINE.into());
    controls.run_fences();
    settle(&controls).await;
    assert!(controls.stop_confirmed(account.as_str()));
    assert_eq!(
        controls.request_logout(&blocked, OWNER, false).await,
        Err("confirmation_required")
    );
    // The Machine drops off: the request is durable and pending.
    fake.connected.lock().clear();
    controls
        .request_logout(&blocked, OWNER, true)
        .await
        .unwrap();
    settle_logouts(&controls).await;
    assert!(fake.logouts.lock().is_empty());
    let pending = controls.logout_value(account.as_str());
    assert_eq!(pending["state"], "pending");
    assert_eq!(pending["targets"][0]["machine_id"], MACHINE);
    assert!(pending["targets"][0].get("recipient_fingerprint").is_none());
    let operation = pending["operation_id"].as_str().unwrap().to_owned();
    // A repeated request is the same operation.
    controls
        .request_logout(&blocked, OWNER, true)
        .await
        .unwrap();
    assert_eq!(
        controls.logout_value(account.as_str())["operation_id"],
        operation
    );
    // The Machine reconnects: the reconciler delivers it once.
    fake.connected.lock().insert(MACHINE.into());
    controls.tick().await;
    settle_logouts(&controls).await;
    let logouts = fake.logouts.lock().clone();
    assert_eq!(logouts.len(), 1);
    let (machine, request) = &logouts[0];
    assert_eq!(machine, MACHINE);
    assert_eq!(request.operation_id, operation);
    assert_eq!(request.generation, completion.profile.generation);
    assert_eq!(request.login_request_id, completion.profile.request_id);
    let done = controls.logout_value(account.as_str());
    assert_eq!(done["state"], "signed_out");
    assert_eq!(done["targets"][0]["credential_cleared"], true);
    assert_eq!(done["targets"][0]["history_retained"], true);
    // The generation admits no session; the account itself remains.
    assert!(
        store
            .native_login_for_session(OWNER, &account, MACHINE)
            .await
            .unwrap()
            .is_none()
    );
    let login = store.provider_login_receipt(OWNER, "login").await.unwrap();
    assert_eq!(
        login.unwrap().native_logout.unwrap().revocation_operation,
        operation
    );
    let registry = store.provider_accounts(OWNER).await.unwrap();
    assert!(
        registry
            .accounts
            .iter()
            .any(|card| card.id == account && card.enabled && !card.revoking)
    );
    // Later ticks never repeat a finished logout.
    controls.tick().await;
    settle_logouts(&controls).await;
    assert_eq!(fake.logouts.lock().len(), 1);
}

#[tokio::test]
async fn unblocking_before_the_machine_is_reached_cancels_the_logout() {
    let (fake, controls, _root) = fixture().await;
    let (account, _) = signed_in(&controls.store).await;
    let ok = initial(account.as_str(), OWNER, "claude-code");
    let blocked = controls
        .enter(&ok, AccountState::Blocked, "owner_block", None, None, false)
        .await
        .unwrap();
    fake.connected.lock().insert(MACHINE.into());
    controls.run_fences();
    settle(&controls).await;
    fake.connected.lock().clear();
    controls
        .request_logout(&blocked, OWNER, true)
        .await
        .unwrap();
    let blocked = controls.cache.control(account.as_str()).unwrap();
    controls
        .enter(
            &blocked,
            AccountState::Ok,
            "owner_unblock",
            None,
            Some(blocked.revision),
            false,
        )
        .await
        .unwrap();
    // Cancelled by the unblock itself, before any Machine reconnects.
    let record = controls.logout_record(account.as_str()).unwrap();
    assert_eq!(record.targets[0].state, LogoutTargetState::Cancelled);
    // A later block never fires the lifted block's unsent logout.
    let unblocked = controls.cache.control(account.as_str()).unwrap();
    controls
        .enter(
            &unblocked,
            AccountState::Blocked,
            "owner_block",
            None,
            None,
            false,
        )
        .await
        .unwrap();
    fake.connected.lock().insert(MACHINE.into());
    controls.tick().await;
    settle_logouts(&controls).await;
    assert!(fake.logouts.lock().is_empty());
    let record = controls.logout_record(account.as_str()).unwrap();
    assert_eq!(record.targets[0].state, LogoutTargetState::Cancelled);
    assert_eq!(
        controls.logout_value(account.as_str())["state"],
        "cancelled"
    );
    assert!(
        controls
            .store
            .native_login_for_session(OWNER, &account, MACHINE)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_re_enrolled_machine_never_certifies_a_logout() {
    let (fake, controls, _root) = fixture().await;
    let (account, _) = signed_in(&controls.store).await;
    let ok = initial(account.as_str(), OWNER, "claude-code");
    let blocked = controls
        .enter(&ok, AccountState::Blocked, "owner_block", None, None, false)
        .await
        .unwrap();
    fake.connected.lock().insert(MACHINE.into());
    controls.run_fences();
    settle(&controls).await;
    *fake.enrollment.lock() = Some("another-enrollment".into());
    controls
        .request_logout(&blocked, OWNER, true)
        .await
        .unwrap();
    settle_logouts(&controls).await;
    assert!(fake.logouts.lock().is_empty());
    let value = controls.logout_value(account.as_str());
    assert_eq!(value["state"], "pending");
    assert_eq!(value["targets"][0]["detail"], "machine_enrollment_changed");
}

#[tokio::test]
async fn a_refused_unblock_keeps_a_valid_pending_logout() {
    let (fake, controls, _root) = fixture().await;
    let (account, _) = signed_in(&controls.store).await;
    let ok = initial(account.as_str(), OWNER, "claude-code");
    let blocked = controls
        .enter(&ok, AccountState::Blocked, "owner_block", None, None, false)
        .await
        .unwrap();
    fake.connected.lock().insert(MACHINE.into());
    controls.run_fences();
    settle(&controls).await;
    fake.connected.lock().clear();
    controls
        .request_logout(&blocked, OWNER, true)
        .await
        .unwrap();
    // An unblock against a stale revision is refused and cancels nothing.
    let current = controls.cache.control(account.as_str()).unwrap();
    assert_eq!(
        controls
            .enter(
                &current,
                AccountState::Ok,
                "owner_unblock",
                None,
                Some(current.revision - 1),
                false,
            )
            .await,
        Err("revision_conflict")
    );
    let record = controls.logout_record(account.as_str()).unwrap();
    assert_eq!(record.targets[0].state, LogoutTargetState::Pending);
}
