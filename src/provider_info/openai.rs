use crate::usage::ProviderUsage;

pub(crate) async fn collect(command: &str) -> anyhow::Result<ProviderUsage> {
    let mut usage = crate::usage::collect_codex(command).await?;
    usage.provider = "openai";
    usage.source = "OpenAI Codex app-server";
    Ok(usage)
}
