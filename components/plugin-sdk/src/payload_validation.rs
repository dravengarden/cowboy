//! Validation of a Plugin package payload against its manifest: identity,
//! the contract rules of each Plugin kind and the Plugin SDK it requires.

use std::collections::BTreeSet;

use anyhow::{Context as _, Result, bail, ensure};
use semver::Version;
use url::Url;

use crate::{
    AUTHENTICATION_PROVIDER_SCHEMA_MIN_VERSION, AUTHENTICATION_PROVIDER_SCHEMA_VERSION,
    AuthenticationProtocol, AuthenticationProviderContract, CODE_INTELLIGENCE_SCHEMA_MIN_VERSION,
    CODE_INTELLIGENCE_SCHEMA_VERSION, MACHINE_BROWSER_SDK, MachineBrowserContract,
    OpenIdConnectContract, PluginKind, PluginManifest, PluginPayload, SESSION_EXTENSION_SDK,
    TelemetryBackendContract, code_intelligence, machine_browser,
};

fn validate_session_extension(manifest: &PluginManifest, payload: &PluginPayload) -> Result<()> {
    let (id, version) = match (manifest.kind, payload) {
        (PluginKind::MemoryBackend, PluginPayload::MemoryBackend(contract)) => {
            contract.validate()?;
            (&contract.id, &contract.version)
        }
        (PluginKind::CallProfile, PluginPayload::CallProfile(contract)) => {
            contract.validate()?;
            (&contract.id, &contract.version)
        }
        _ => bail!("plugin kind does not match its payload"),
    };
    ensure!(
        *id == manifest.id && *version == manifest.version,
        "session extension payload identity mismatch"
    );
    let (major, minor, patch) = SESSION_EXTENSION_SDK;
    ensure!(
        manifest
            .components
            .iter()
            .any(|component| component.id == "cowboy.plugin-sdk"
                && Version::parse(&component.version)
                    .is_ok_and(|version| version >= Version::new(major, minor, patch))),
        "session extensions require Plugin SDK 1.12 or newer"
    );
    Ok(())
}

fn validate_telemetry_payload(
    manifest: &PluginManifest,
    contract: &TelemetryBackendContract,
) -> Result<()> {
    ensure!(
        contract.id == manifest.id && contract.version == manifest.version,
        "telemetry payload identity mismatch"
    );
    contract.validate()?;
    ensure!(
        manifest
            .components
            .iter()
            .any(|component| component.id == "cowboy.plugin-sdk"
                && Version::parse(&component.version).is_ok_and(|version| version
                    >= Version::new(1, if contract.schema_version == 2 { 8 } else { 7 }, 0))),
        "telemetry backends require Plugin SDK 1.7 (schema 1) or 1.8 (schema 2)"
    );
    Ok(())
}

fn validate_machine_browser(
    manifest: &PluginManifest,
    contract: &MachineBrowserContract,
) -> Result<()> {
    ensure!(
        contract.id == manifest.id && contract.version == manifest.version,
        "machine browser payload identity mismatch"
    );
    let (major, minor, patch) = MACHINE_BROWSER_SDK;
    ensure!(
        manifest
            .components
            .iter()
            .any(|component| component.id == "cowboy.plugin-sdk"
                && Version::parse(&component.version)
                    .is_ok_and(|version| version >= Version::new(major, minor, patch))),
        "machine browsers require Plugin SDK 1.13 or newer"
    );
    machine_browser::validate_contract(contract)
}

pub(crate) fn validate_payload(manifest: &PluginManifest, payload: &PluginPayload) -> Result<()> {
    match (manifest.kind, payload) {
        (
            PluginKind::MemoryBackend | PluginKind::CallProfile,
            PluginPayload::MemoryBackend(_) | PluginPayload::CallProfile(_),
        ) => validate_session_extension(manifest, payload)?,
        (PluginKind::MachineBrowser, PluginPayload::MachineBrowser(contract)) => {
            validate_machine_browser(manifest, contract)?;
        }
        (PluginKind::WorkspaceExtension, PluginPayload::WorkspaceExtension(contract)) => {
            ensure!(
                contract.id == manifest.id && contract.version == manifest.version,
                "extension payload identity mismatch"
            );
            ensure!(
                manifest
                    .components
                    .iter()
                    .any(|c| c.id == "cowboy.plugin-sdk"
                        && Version::parse(&c.version).is_ok_and(|v| v
                            >= Version::new(
                                1,
                                if contract.schema_version >= 2 { 10 } else { 9 },
                                0
                            ))),
                "workspace extension schema requires a compatible Plugin SDK"
            );
            contract.validate()?;
        }
        (PluginKind::TelemetryBackend, PluginPayload::TelemetryBackend(contract)) => {
            validate_telemetry_payload(manifest, contract)?;
        }
        (PluginKind::AgentProvider, PluginPayload::AgentProvider(provider)) => {
            ensure!(
                provider.manifest.id == manifest.id,
                "agent payload id mismatch"
            );
            ensure!(
                provider.manifest.version == manifest.version,
                "agent payload version mismatch"
            );
            ensure!(
                provider.manifest.publisher == manifest.publisher,
                "agent payload publisher mismatch"
            );
        }
        (PluginKind::AuthenticationProvider, PluginPayload::AuthenticationProvider(contract)) => {
            validate_authentication_provider(manifest, contract)?;
        }
        (PluginKind::CodeIntelligence, PluginPayload::CodeIntelligence(contract)) => {
            ensure!(
                (CODE_INTELLIGENCE_SCHEMA_MIN_VERSION..=CODE_INTELLIGENCE_SCHEMA_VERSION)
                    .contains(&contract.schema_version),
                "unsupported code-intelligence contract"
            );
            ensure!(
                contract.id == manifest.id,
                "code-intelligence payload id mismatch"
            );
            ensure!(
                contract.version == manifest.version,
                "code-intelligence payload version mismatch"
            );
            ensure!(
                !contract.operations.is_empty(),
                "code-intelligence operations are empty"
            );
            ensure!(
                !contract.supported_platforms.is_empty(),
                "code-intelligence platforms are empty"
            );
            code_intelligence::validate_contract(contract)?;
            if contract.schema_version >= 2 {
                ensure!(
                    manifest.components.iter().any(|component| {
                        component.id == "cowboy.plugin-sdk"
                            && Version::parse(&component.version)
                                .is_ok_and(|version| version >= Version::new(1, 6, 0))
                    }),
                    "owned code runtimes require Plugin SDK 1.6 or newer"
                );
            }
        }
        _ => bail!("plugin kind and payload kind mismatch"),
    }
    Ok(())
}

fn validate_authentication_provider(
    manifest: &PluginManifest,
    contract: &AuthenticationProviderContract,
) -> Result<()> {
    ensure!(
        (AUTHENTICATION_PROVIDER_SCHEMA_MIN_VERSION..=AUTHENTICATION_PROVIDER_SCHEMA_VERSION)
            .contains(&contract.schema_version),
        "unsupported authentication contract"
    );
    ensure!(
        contract.id == manifest.id,
        "authentication payload id mismatch"
    );
    ensure!(
        contract.version == manifest.version,
        "authentication payload version mismatch"
    );
    ensure!(
        valid_label(&contract.display_name, 64) && valid_label(&contract.button_label, 80),
        "authentication labels are invalid"
    );
    match &contract.protocol {
        AuthenticationProtocol::OpenIdConnect(oidc) => validate_oidc_contract(oidc),
        AuthenticationProtocol::LocalPassword(_) | AuthenticationProtocol::Webauthn(_) => {
            ensure!(
                contract.schema_version >= 2,
                "local authentication protocols require schema 2"
            );
            ensure!(
                manifest.components.iter().any(|component| {
                    component.id == "cowboy.plugin-sdk"
                        && Version::parse(&component.version)
                            .is_ok_and(|version| version >= Version::new(1, 5, 0))
                }),
                "local authentication protocols require Plugin SDK 1.5 or newer"
            );
            Ok(())
        }
    }
}

fn validate_oidc_contract(contract: &OpenIdConnectContract) -> Result<()> {
    const RESERVED: [&str; 9] = [
        "response_type",
        "client_id",
        "redirect_uri",
        "scope",
        "state",
        "nonce",
        "code_challenge",
        "code_challenge_method",
        "request_uri",
    ];

    validate_oidc_issuer(&contract.issuer)?;
    validate_https_url(
        &contract.authorization_endpoint,
        "OIDC authorization endpoint",
    )?;
    if let Some(endpoint) = contract.pushed_authorization_request_endpoint.as_deref() {
        validate_https_url(endpoint, "OIDC pushed authorization request endpoint")?;
    }
    validate_https_url(&contract.token_endpoint, "OIDC token endpoint")?;
    validate_https_url(&contract.jwks_uri, "OIDC JWKS endpoint")?;
    if let Some(endpoint) = contract.end_session_endpoint.as_deref() {
        validate_https_url(endpoint, "OIDC end-session endpoint")?;
    }
    ensure!(
        contract
            .scopes
            .first()
            .is_some_and(|scope| scope == "openid"),
        "OIDC scopes must start with openid"
    );
    ensure!(
        !contract.scopes.is_empty()
            && contract.scopes.len() <= 32
            && contract.scopes.iter().all(|scope| valid_scope(scope))
            && unique_values(&contract.scopes),
        "OIDC scopes must be non-empty and unique"
    );
    ensure!(
        !contract.client_authentication_methods.is_empty()
            && unique_values(&contract.client_authentication_methods),
        "OIDC client authentication methods must be non-empty and unique"
    );
    ensure!(
        !contract.id_token_signing_algorithms.is_empty()
            && unique_values(&contract.id_token_signing_algorithms),
        "OIDC ID-token algorithms must be non-empty and unique"
    );
    ensure!(
        contract.authorization_parameters.len() <= 32,
        "too many OIDC authorization parameters"
    );
    for (name, value) in &contract.authorization_parameters {
        ensure!(
            valid_parameter_name(name) && !RESERVED.contains(&name.as_str()),
            "OIDC authorization parameter is reserved or invalid"
        );
        ensure!(
            valid_label(value, 256),
            "OIDC authorization parameter is invalid"
        );
    }
    Ok(())
}

fn validate_https_url(value: &str, label: &str) -> Result<()> {
    ensure!(value.len() <= 2_048, "{label} is too long");
    let url = Url::parse(value).with_context(|| format!("invalid {label}"))?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "{label} must use HTTPS without credentials or a fragment"
    );
    Ok(())
}

fn validate_oidc_issuer(value: &str) -> Result<()> {
    validate_https_url(value, "OIDC issuer")?;
    let issuer = Url::parse(value).context("invalid OIDC issuer")?;
    ensure!(
        issuer.query().is_none(),
        "OIDC issuer must not contain a query"
    );
    Ok(())
}

fn valid_label(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= maximum
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_parameter_name(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_scope(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.bytes().all(|byte| {
            byte == 0x21 || (0x23..=0x5b).contains(&byte) || (0x5d..=0x7e).contains(&byte)
        })
}

fn unique_values<T: Ord>(values: &[T]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
