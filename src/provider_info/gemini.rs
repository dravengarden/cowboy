use crate::usage::ProviderUsage;

pub(crate) fn overlay(provider: &mut ProviderUsage, empty: Option<&str>) {
    if provider.status != "available" {
        provider.error = Some(
            empty
                .unwrap_or("Account quota is not exposed by this Provider.")
                .to_owned(),
        );
    }
}
