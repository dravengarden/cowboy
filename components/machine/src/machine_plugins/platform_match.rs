//! Select the payload and runtime artifacts built for this Machine's platform.

use super::*;

pub(super) fn matching_payload<'a>(
    package: &'a ProviderPackage,
    platform: &Platform,
    architecture: &str,
) -> Result<&'a cowboy_provider_sdk::PlatformPayload> {
    let os = match platform {
        Platform::Linux => OperatingSystem::Linux,
        Platform::Macos => OperatingSystem::Macos,
    };
    let architecture = match architecture {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Aarch64,
        other => bail!("unsupported Machine architecture {other:?}"),
    };
    package
        .manifest
        .runtime
        .platforms
        .iter()
        .find(|payload| payload.os == os && payload.architecture == architecture)
        .context("Provider has no payload for this Machine platform")
}

pub(super) fn matching_runtime_artifacts<'a>(
    release: &'a cowboy_provider_sdk::AgentRuntimeBinding,
    platform: &Platform,
    architecture: &str,
) -> Result<&'a PlatformRuntimeArtifacts> {
    let os = match platform {
        Platform::Linux => OperatingSystem::Linux,
        Platform::Macos => OperatingSystem::Macos,
    };
    let architecture = match architecture {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Aarch64,
        other => bail!("unsupported Machine architecture {other:?}"),
    };
    release
        .runtime_artifacts
        .iter()
        .find(|artifacts| artifacts.os == os && artifacts.architecture == architecture)
        .context("Agent Plugin binding has no runtime artifacts for this Machine platform")
}
