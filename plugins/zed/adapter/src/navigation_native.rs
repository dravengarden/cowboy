//! Exact-pair bounded acquisition. A refusal is not cleanup or permission to
//! replay, and a missing capability never falls back to the upstream query.
use crate::sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};
use crate::{ZedRuntime, *};
use anyhow::ensure;
use wire::cowboy_navigation_response::{Outcome, Refusal};

#[derive(Debug)]
pub(super) struct NativeRefusal(pub(super) Refusal);

impl std::fmt::Display for NativeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native navigation refused: {:?}", self.0)
    }
}
impl std::error::Error for NativeRefusal {}

fn decode(
    probe: bool,
    response: &wire::CowboyNavigationResponse,
) -> Result<Vec<proto::LspResponse>> {
    ensure!(
        response.protocol == 1,
        "unsupported native navigation protocol"
    );
    let outcome = Outcome::from_i32(response.outcome).context("invalid navigation outcome")?;
    let refusal = Refusal::from_i32(response.refusal).context("invalid navigation refusal")?;
    match outcome {
        Outcome::Supported if probe && refusal == Refusal::None && response.result.is_empty() => {
            Ok(Vec::new())
        }
        Outcome::Refused if !probe && refusal != Refusal::None && response.result.is_empty() => {
            Err(NativeRefusal(refusal).into())
        }
        Outcome::Complete
            if !probe && refusal == Refusal::None && response.result.len() <= 1024 * 1024 =>
        {
            let result = proto::LspQueryResponse::decode(response.result.as_slice())?;
            ensure!(
                result.project_id == proto::REMOTE_SERVER_PROJECT_ID
                    && result.lsp_request_id == 0
                    && result.responses.len() <= 4,
                "invalid native navigation result identity or budget"
            );
            let mut servers = std::collections::HashSet::new();
            ensure!(
                result
                    .responses
                    .iter()
                    .all(|response| servers.insert(response.server_id)),
                "duplicate native language server result"
            );
            Ok(result.responses)
        }
        _ => bail!("invalid native navigation response shape"),
    }
}

async fn exchange(zed: &ZedRuntime, query: Vec<u8>) -> Result<Vec<proto::LspResponse>> {
    ensure!(
        query.len() <= 8 * 1024,
        "native navigation request exceeds limit"
    );
    let probe = query.is_empty();
    let reply = zed
        .sync
        .exchange(
            zed,
            Payload::NavigationRequest(wire::CowboyNavigation {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                protocol: 1,
                query,
            }),
        )
        .await?;
    let Payload::NavigationResponse(response) = reply else {
        bail!("unexpected private navigation response kind");
    };
    decode(probe, &response)
}

pub(super) async fn support(zed: Option<&Zed>) -> Result<()> {
    exchange(zed.context("native runtime is unavailable")?, Vec::new()).await?;
    Ok(())
}

pub(super) async fn query(
    zed: &ZedRuntime,
    request: proto::lsp_query::Request,
) -> Result<Vec<proto::LspResponse>> {
    ensure!(
        matches!(
            &request,
            proto::lsp_query::Request::GetDefinition(_)
                | proto::lsp_query::Request::GetDeclaration(_)
                | proto::lsp_query::Request::GetTypeDefinition(_)
                | proto::lsp_query::Request::GetImplementation(_)
                | proto::lsp_query::Request::GetReferences(_)
        ),
        "unsupported bounded navigation kind"
    );
    exchange(
        zed,
        proto::LspQuery {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            request: Some(request),
            ..Default::default()
        }
        .encode_to_vec(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_is_typed_and_cannot_become_empty_or_partial_success() {
        for reason in [
            Refusal::Budget,
            Refusal::Source,
            Refusal::LanguageServer,
            Refusal::Target,
            Refusal::Deadline,
        ] {
            let reply = wire::CowboyNavigationResponse {
                protocol: 1,
                outcome: Outcome::Refused as i32,
                refusal: reason as i32,
                ..Default::default()
            };
            assert_eq!(
                decode(false, &reply)
                    .unwrap_err()
                    .downcast_ref::<NativeRefusal>()
                    .unwrap()
                    .0,
                reason
            );
            assert!(decode(true, &reply).is_err());
            assert!(
                decode(
                    false,
                    &wire::CowboyNavigationResponse {
                        result: vec![1],
                        ..reply
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn only_the_matching_capability_and_complete_result_are_accepted() {
        let probe = wire::CowboyNavigationResponse {
            protocol: 1,
            outcome: Outcome::Supported as i32,
            ..Default::default()
        };
        assert!(decode(true, &probe).is_ok());
        assert!(decode(false, &probe).is_err());
        let complete = wire::CowboyNavigationResponse {
            protocol: 1,
            outcome: Outcome::Complete as i32,
            result: proto::LspQueryResponse {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                ..Default::default()
            }
            .encode_to_vec(),
            ..Default::default()
        };
        assert!(decode(false, &complete).unwrap().is_empty());
        assert!(decode(true, &complete).is_err());
        assert!(
            decode(
                false,
                &wire::CowboyNavigationResponse {
                    refusal: 99,
                    ..complete.clone()
                }
            )
            .is_err()
        );
        assert!(
            decode(
                false,
                &wire::CowboyNavigationResponse {
                    outcome: 99,
                    ..complete
                }
            )
            .is_err()
        );
    }
}
