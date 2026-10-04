use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Harness {
    _root: tempfile::TempDir,
    store: Store,
    hub: Hub,
    user: crate::store::ProductUser,
    authentication: crate::auth_plugins::ProductAuthentication,
    devices: crate::client_auth::DeviceAccessSessions,
    freeze: Arc<crate::machine_convergence::ConvergenceFreeze>,
}
impl Harness {
    async fn revoke_device(&self) -> AuthResult<()> {
        let id = self
            .authentication
            .provider("cardea")
            .unwrap()
            .cardea_product_device_id("11111111-1111-4111-8111-111111111111")
            .map_err(unavailable)?;
        let now = auth_now_ms();
        let device = crate::store::ProductDevice {
            id: id.clone(),
            user_id: self.user.id.clone(),
            name: "Fixture".into(),
            public_key: "A".repeat(43),
            created_at_ms: now,
            last_used_at_ms: None,
            revoked_at_ms: None,
        };
        let refresh = crate::store::ProductDeviceRefreshToken {
            token_hash: "test-only".into(),
            device_id: id.clone(),
            family_id: id.clone(),
            created_at_ms: now,
            expires_at_ms: now + 1000,
            used_at_ms: None,
            revoked_at_ms: None,
        };
        self.store
            .admit_user_device(&device, &refresh, 8, true)
            .await
            .map_err(unavailable)?;
        self.store
            .revoke_user_device(&id, now)
            .await
            .map_err(unavailable)?;
        Ok(())
    }
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let provider = Arc::new(crate::oidc::OidcProvider::cardea_device_fixture(
            "https://cardea.example",
        ));
        let user = crate::store::ProductUser {
            id: "c".repeat(32),
            username: "draven".into(),
            password_algo: "fixture".into(),
            password_hash: "unused".into(),
            created_at_ms: auth_now_ms(),
            updated_at_ms: auth_now_ms(),
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let hub = Hub::new();
        hub.set_setting(
            crate::admin::PERMISSIONS_SETTING.into(),
            serde_json::json!({"default_role":"operator","grants":[]}),
        );
        let freeze = Arc::new(crate::machine_convergence::ConvergenceFreeze::new(
            root.path(),
        ));
        Self {
            _root: root,
            store,
            hub,
            user,
            authentication: crate::auth_plugins::ProductAuthentication::test_default(Some(
                provider,
            )),
            devices: crate::client_auth::DeviceAccessSessions::default(),
            freeze,
        }
    }
    fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: true,
            store: Some(&self.store),
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }
    fn desired() -> DesiredPlugin {
        serde_json::from_value(serde_json::json!({"release":{"release_schema":1,"plugin_id":"victoria","plugin_version":"1.1.0","plugin_kind":"telemetry_backend","package_digest":"sha256:fixture-package","artifact_digest":format!("sha256:{}","a".repeat(64)),"artifact_url":"https://example.invalid/plugin","publisher":"fixture","contract_fingerprint":format!("sha256:{}","b".repeat(64)),"component_release":"2.9.0","host_bundle_digest":null,"signature":"fixture","supported_platforms":[],"runtime_artifacts":[]},"package_base64":"e30=","publisher_public_key":"fixture","host_bundle_base64":null})).unwrap()
    }
    fn plan(&self, catalog: &Catalog, at: u64) -> Plan {
        let desired = Self::desired();
        let selected = Input {
            machine: "machine-test".into(),
            plugin: "victoria".into(),
            version: "1.1.0".into(),
            digest: desired.release.artifact_digest.clone(),
            target: Some(InstallTarget::Vacant {}),
            envelope_digest: Some(format!(
                "sha256:{}",
                crate::admin::hex_sha256(&serde_json::to_vec(&desired).unwrap())
            )),
        };
        Plan {
            schema: wire::OPERATION_SCHEMA.into(),
            operation_id: "A".repeat(43),
            application_id: catalog.application_id.clone(),
            action: ACTION.into(),
            catalog_digest: catalog.digest(),
            policy_revision: revision(
                &self.hub,
                &self.user,
                self.authentication.provider("cardea").unwrap(),
            )
            .unwrap(),
            subject_id: "draven".into(),
            grant_id: "11111111-1111-4111-8111-111111111111".into(),
            binding_key: "A".repeat(43),
            resources: vec![wire::Resource {
                kind: "machine-plugin".into(),
                id: "machine-test".into(),
                scope: version(&serde_json::json!("service-test")),
                expected_revision: "v1".into(),
            }],
            input: serde_json::to_value(selected).unwrap(),
            review: cardea_core::ApprovalDisplay {
                title: "Install fixture".into(),
                summary: "Synthetic exact release".into(),
                facts: vec![],
            },
            created_at: at - 10,
            review_until: at + 540,
            execute_until: at + 600,
        }
    }
}
struct Backend {
    catalog: Catalog,
    op: wire::Operation,
    at: u64,
    expiry: u64,
}
impl TransportContract for Backend {
    fn issuer(&self) -> &str {
        "https://cardea.example"
    }
    fn application_id(&self) -> &str {
        "cowboy-production"
    }
    fn exchange_identity(&mut self, _: &DeviceCredential) -> AuthResult<serde_json::Value> {
        Ok(
            serde_json::json!({"schema":"dravengarden.cardea.device-identity/v1","client_id":"cowboy-production","subject_id":"draven","grant_id":self.op.plan.grant_id,"public_key":self.op.plan.binding_key,"expires_at":self.expiry}),
        )
    }
    fn command(&mut self, c: &Command) -> AuthResult<serde_json::Value> {
        match c {
            Command::Catalog => Ok(serde_json::to_value(&self.catalog).unwrap()),
            Command::View { .. } => Ok(serde_json::to_value(&self.op).unwrap()),
            Command::Claim { claim_id, .. } => Ok(serde_json::to_value(
                self.op
                    .claim(claim_id, self.at)
                    .ok_or(Error::ApprovalRequired)?,
            )
            .unwrap()),
            Command::Receipt { receipt } => {
                if !self.op.record_receipt(receipt.clone()) {
                    return Err(Error::Conflict);
                }
                Ok(serde_json::to_value(receipt).unwrap())
            }
            _ => Err(Error::InvalidContract),
        }
    }
}
#[derive(Default)]
struct MemoryJournal {
    started: bool,
    receipt: Option<Receipt>,
}
impl JournalContract for MemoryJournal {
    fn prepare_claim(&mut self, _: &Plan, _: &str) -> AuthResult<Recovery> {
        Ok(if let Some(r) = &self.receipt {
            Recovery::Completed(r.clone())
        } else if self.started {
            Recovery::Uncertain
        } else {
            Recovery::Ready
        })
    }
    fn start_effect(&mut self, _: &Plan, _: &wire::Admission, _: u64) -> AuthResult<bool> {
        if self.started {
            return Ok(false);
        }
        self.started = true;
        Ok(true)
    }
    fn record(&mut self, r: &Receipt) -> AuthResult<()> {
        self.receipt = Some(r.clone());
        Ok(())
    }
}
struct App {
    h: Arc<Harness>,
    runtime: Handle,
    change: &'static str,
    effects: Arc<AtomicUsize>,
}
impl App {
    fn change_authority(
        &self,
        authority: &operator_approval::InstallationAuthority,
        step: &crate::machine_protocol::plugin_install::InstallStep,
    ) -> AuthResult<()> {
        match self.change {
            "device_revoked" => self.runtime.block_on(self.h.revoke_device())?,
            "expired_start_after_bind" => {
                assert!(authority.matches_auth_sync(true));
                authority.expire_cardea_start_for_test();
                assert!(
                    authority.matches_auth_sync(false),
                    "already-admitted post-install continuation retains its identity budget"
                );
                assert!(
                    !authority.matches_auth_sync(true),
                    "initial auth sync cannot start after its original deadline"
                );
            }
            "policy" | "policy_auth_sync" => {
                self.h.hub.set_setting(
                    crate::admin::PERMISSIONS_SETTING.into(),
                    serde_json::json!({"default_role":"operator","grants":[{"account":"another-account","role":"viewer"}]}),
                );
                if self.change == "policy" {
                    assert!(
                        !authority.matches_live_step(step),
                        "policy must be checked before any async revalidation"
                    );
                } else {
                    assert!(
                        !authority.matches_auth_sync(true),
                        "auth sync must check policy before any async revalidation"
                    );
                }
            }
            "role_aba" => {
                self.h.hub.set_setting(
                    crate::admin::PERMISSIONS_SETTING.into(),
                    serde_json::json!({"default_role":"viewer","grants":[]}),
                );
                self.h.hub.set_setting(
                    crate::admin::PERMISSIONS_SETTING.into(),
                    serde_json::json!({"default_role":"operator","grants":[]}),
                );
                assert!(
                    !authority.matches_auth_sync(true),
                    "role revoke/regrant must invalidate auth sync at dispatch"
                );
            }
            "disabled" => {
                self.runtime
                    .block_on(
                        self.h
                            .store
                            .set_user_disabled_at(&self.h.user.id, Some(auth_now_ms())),
                    )
                    .map_err(unavailable)?;
            }
            "freeze" => {
                self.h
                    .freeze
                    .freeze("fixture", Some("stop fixture"), auth_now_ms())
                    .map_err(unavailable)?;
            }
            _ => {}
        }
        Ok(())
    }
}
impl Application for App {
    fn evaluate(&self, p: &Plan, _: &Requester, _: u64) -> AuthResult<Eligibility> {
        Ok(Eligibility::Eligible {
            policy_revision: p.policy_revision.clone(),
        })
    }
    fn approver_eligible(&self, _: &Plan, _: &str, _: u64) -> AuthResult<bool> {
        Ok(true)
    }
    fn commit(&mut self, ctx: &ExecutionContext, _: u64) -> AuthResult<String> {
        let grant = Grant::capture(self.h.auth(), self.h.freeze.clone(), ctx, &self.h.user)?;
        let desired = Harness::desired();
        let target = InstallTarget::Vacant {};
        assert!(grant.matches("machine-test", &desired, &ctx.plan().operation_id, &target));
        assert!(!grant.matches("other-machine", &desired, &ctx.plan().operation_id, &target));
        assert!(!grant.matches("machine-test", &desired, "other-operation", &target));
        let mut changed = desired.clone();
        changed.release.plugin_version = "1.2.0".into();
        assert!(!grant.matches("machine-test", &changed, &ctx.plan().operation_id, &target));
        let mut wrong = ctx.plan().clone();
        wrong.input["target"] = serde_json::json!({"kind":"unknown"});
        assert!(decode_input(&wrong).is_err());
        let approval = operator_approval::OperatorApproval::capture_cardea("service-test", grant);
        // This authority cannot turn into uninstall/recovery/export permission.
        let other = operator_approval::OperatorApproval::capture_cardea(
            "service-test",
            Grant::capture(self.h.auth(), self.h.freeze.clone(), ctx, &self.h.user)?,
        );
        let mut uninstall = crate::plugin_operation::fixture("different-purpose");
        uninstall.actor = other.actor().clone();
        assert!(other.bind(&uninstall).is_err());
        let other_service = operator_approval::OperatorApproval::capture_cardea(
            "different-service",
            Grant::capture(self.h.auth(), self.h.freeze.clone(), ctx, &self.h.user)?,
        );
        assert!(
            other_service
                .bind_installation(
                    "machine-test",
                    &desired,
                    ctx.plan().operation_id.clone(),
                    target.clone()
                )
                .is_err()
        );
        let authority = approval
            .bind_installation(
                "machine-test",
                &desired,
                ctx.plan().operation_id.clone(),
                target,
            )
            .map_err(unavailable)?;
        let step = authority.intent().machine_step().map_err(unavailable)?;
        self.change_authority(&authority, &step)?;
        if !self.runtime.block_on(authority.check(
            self.h.auth(),
            "service-test",
            "machine-test",
            &desired,
        )) || !authority.matches_auth_sync(true)
            || !authority.matches_live_step(&step)
        {
            return Err(Error::PermissionDenied);
        }
        self.effects.fetch_add(1, Ordering::SeqCst);
        Ok("Fixture native boundary passed".into())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verified_sdk_context_obeys_native_purpose_role_aba_disabled_freeze_and_deadlines() {
    for change in [
        "none",
        "policy",
        "policy_auth_sync",
        "role_aba",
        "disabled",
        "device_revoked",
        "freeze",
        "expired_identity",
        "expired_start",
        "expired_start_after_bind",
    ] {
        let h = Arc::new(Harness::new().await);
        let runtime = Handle::current();
        let effects = Arc::new(AtomicUsize::new(0));
        let observed = effects.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let actual = clock().unwrap();
            let at = if change == "expired_start" {
                actual - 40
            } else if change == "expired_identity" {
                actual - 2
            } else {
                actual
            };
            let expiry = if change == "expired_identity" {
                actual - 1
            } else {
                actual + 300
            };
            let sdk_clock = if change == "expired_identity" {
                actual - 2
            } else {
                at
            };
            let catalog = crate::cli::cardea::proposal(
                "cowboy-production",
                "https://cowboy.example",
                &"A".repeat(43),
                0,
            )
            .unwrap()
            .catalog;
            let plan = h.plan(&catalog, at);
            assert!(plan.validate(&catalog, at));
            let mut op = wire::Operation::new(plan);
            assert!(op.decide(true, "draven", at));
            let identity = DeviceCredential {
                schema: "dravengarden.cardea.device-identity-credential/v1".into(),
                issuer: "https://cardea.example".into(),
                client_id: "cowboy-production".into(),
                grant_id: op.plan.grant_id.clone(),
                expires_at: expiry,
                access_token: "synthetic".into(),
                assertion: "synthetic".into(),
            };
            let id = op.plan.operation_id.clone();
            let mut cardea = Cardea::new(Backend {
                catalog,
                op,
                at,
                expiry,
            });
            let requester = cardea.verify_requester(&identity, sdk_clock).unwrap();
            let mut app = App {
                h,
                runtime,
                change,
                effects,
            };
            let mut journal = MemoryJournal::default();
            cardea.execute(
                &mut app,
                &mut journal,
                &requester,
                &id,
                &"Q".repeat(43),
                &|| Ok(sdk_clock),
            )
        })
        .await
        .unwrap();
        assert!(outcome.is_ok());
        let result = outcome.unwrap();
        assert_eq!(
            observed.load(Ordering::SeqCst),
            usize::from(change == "none"),
            "{change}"
        );
        assert_eq!(
            result.receipt.outcome,
            if change == "none" {
                Status::Succeeded
            } else {
                Status::OutcomeUnknown
            },
            "{change}"
        );
    }
}
#[test]
fn protocol_capacity_and_clock_rollback_are_bounded() {
    let mut window = Window::default();
    for _ in 0..120 {
        assert!(window.admit(600));
    }
    assert!(!window.admit(600));
    assert!(!window.admit(599));
    assert!(window.admit(660));
}
