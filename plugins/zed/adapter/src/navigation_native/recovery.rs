//! A query observes the saved original operation; it never repeats execution.
use super::*;
use wire::cowboy_navigation::Action;

pub(crate) struct Ticket(wire::CowboyNavigation);

impl Ticket {
    pub(crate) async fn prepare(
        zed: &ZedRuntime,
        query: proto::lsp_query::Request,
    ) -> Result<Self> {
        let Payload::NavigationResponse(probe) = zed
            .sync
            .exchange(
                zed,
                Payload::NavigationRequest(wire::CowboyNavigation {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    protocol: 2,
                    ..Default::default()
                }),
            )
            .await?
        else {
            bail!("unexpected native navigation probe");
        };
        ensure!(
            probe.protocol == 2 && probe.instance.len() == 16 && probe.operation_id == 0,
            "native navigation recovery unavailable"
        );
        decode(true, &probe)?;
        let query = proto::LspQuery {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            request: Some(query),
            ..Default::default()
        }
        .encode_to_vec();
        ensure!(query.len() <= 8192, "navigation request exceeds limit");
        Ok(Self(wire::CowboyNavigation {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            protocol: 2,
            query,
            instance: probe.instance,
            operation_id: zed.sync.navigation_id()?,
            action: Action::Execute as i32,
        }))
    }

    pub(crate) async fn observe(
        &self,
        zed: &ZedRuntime,
        execute: bool,
    ) -> Result<Option<Vec<proto::LspResponse>>> {
        let mut request = self.0.clone();
        request.action = if execute {
            Action::Execute
        } else {
            Action::Query
        } as i32;
        let Payload::NavigationResponse(reply) = zed
            .sync
            .exchange(zed, Payload::NavigationRequest(request))
            .await?
        else {
            bail!("unexpected native navigation reply");
        };
        ensure!(
            reply.protocol == 2
                && reply.instance == self.0.instance
                && reply.operation_id == self.0.operation_id,
            "native navigation result identity changed"
        );
        if matches!(
            Outcome::from_i32(reply.outcome),
            Some(Outcome::Pending | Outcome::Unknown)
        ) {
            ensure!(
                reply.refusal == Refusal::None as i32 && reply.result.is_empty(),
                "invalid uncertain navigation result"
            );
            return Ok(None);
        }
        decode(false, &reply).map(Some)
    }

    pub(crate) fn forget(&self, zed: &ZedRuntime) -> Result<()> {
        zed.sync.forget_navigation(zed, self.0.clone())
    }
}
