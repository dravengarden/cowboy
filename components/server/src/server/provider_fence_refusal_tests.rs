
use super::*;

#[test]
fn refusal_names_the_actual_lifecycle_state() {
    assert_eq!(provider_fence_refusal(PluginFenceState::Installing), None);
    assert!(
        provider_fence_refusal(PluginFenceState::Uninstalling)
            .is_some_and(|refusal| refusal.contains("is uninstalling"))
    );
    assert!(
        provider_fence_refusal(PluginFenceState::Uninstalled)
            .is_some_and(|refusal| refusal.contains("was uninstalled"))
    );
    // An interrupted install must not be reported as an uninstall.
    assert!(
        provider_fence_refusal(PluginFenceState::NeedsReconcile).is_some_and(|refusal| refusal
            .contains("unconfirmed")
            && !refusal.contains("uninstalling"))
    );
}
