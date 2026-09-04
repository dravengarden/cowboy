use crate::usage::ProviderUsage;

pub(crate) async fn collect(command: &str) -> anyhow::Result<ProviderUsage> {
    crate::usage::collect_codex(command).await
}
