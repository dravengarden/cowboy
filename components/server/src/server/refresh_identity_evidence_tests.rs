use super::refresh_identity_evidence;
use crate::provider_account::{NativeIdentity, ProviderAccountId};
use cowboy_provider_sdk::NativeIdentityRecipe;

#[test]
fn default_refresh_refuses_evidence_outside_the_signed_contract() {
    let default = ProviderAccountId::default();
    let recipe = NativeIdentityRecipe::FreshLoginJsonV1 {
        authority: "claude-ai".into(),
        relative_path: ".claude.json".into(),
        subject_path: vec!["oauthAccount".into(), "accountUuid".into()],
        tenant_path: None,
    };
    let identity = |authority: &str| NativeIdentity {
        authority: authority.into(),
        subject: "subject".into(),
        tenant: None,
    };
    let check = |recipe, identity: Option<&NativeIdentity>| {
        refresh_identity_evidence("claude-code", &default, recipe, identity)
    };
    // No Machine reports identity for the default account today.
    assert!(check(Some(&recipe), None).is_ok());
    assert!(check(None, None).is_ok());
    assert!(check(Some(&recipe), Some(&identity("claude-ai"))).is_ok());
    assert!(check(Some(&recipe), Some(&identity("elsewhere"))).is_err());
    assert!(check(None, Some(&identity("claude-ai"))).is_err());
}
