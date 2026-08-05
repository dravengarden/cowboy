use crate::usage::ProviderUsage;

pub(crate) fn overlay(provider: &mut ProviderUsage) {
    if provider.status != "available" {
        provider.error = Some("Gemini ACP exposes session activity but not account quota".into());
    }
}
