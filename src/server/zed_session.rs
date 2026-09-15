//! Session-scoped Zed operation boundary. Native resource ownership and
//! recovery across separate HTTP requests remain a distinct protocol concern.

use std::time::Duration;

use anyhow::Context as _;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

use super::{FsPath, Hub, MachineControl, ZedAdapterResponse, validate_zed_adapter_response};
use crate::core::SessionCodeScope;
use crate::machine_control::ConnectionToken;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(35);
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
// Match the Machine-owned adapter exchange bound. Include the trailing newline.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

enum Transport {
    Local(BufReader<UnixStream>),
    Remote(ConnectionToken),
}

/// One call sequence, not a buffer lease. Never reconnects, retries or restores
/// from a path/epoch string. Taking the transport before I/O makes cancellation
/// terminal too: an unread reply cannot be consumed by a subsequent request.
pub(super) struct Operation<'a> {
    hub: &'a Hub,
    control: &'a MachineControl,
    scope: &'a SessionCodeScope,
    transport: Option<Transport>,
}

impl<'a> Operation<'a> {
    pub(super) async fn connect(
        hub: &'a Hub,
        control: &'a MachineControl,
        local_socket: Option<&FsPath>,
        scope: &'a SessionCodeScope,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(hub.code_scope_is_current(scope), "code context changed");
        let transport = if scope.machine_id() == "local" {
            let socket = local_socket.context("local Zed adapter is not configured")?;
            Transport::Local(connect_local(socket).await?)
        } else {
            Transport::Remote(
                control
                    .operation_connection(scope.machine_id())
                    .map_err(anyhow::Error::msg)?,
            )
        };
        anyhow::ensure!(hub.code_scope_is_current(scope), "code context changed");
        Ok(Self {
            hub,
            control,
            scope,
            transport: Some(transport),
        })
    }

    pub(super) async fn request(
        &mut self,
        request: serde_json::Value,
    ) -> anyhow::Result<ZedAdapterResponse> {
        let mut transport = self.transport.take().context("Zed operation has ended")?;
        anyhow::ensure!(
            self.hub.code_scope_is_current(self.scope),
            "code context changed"
        );
        let response = match &mut transport {
            Transport::Local(stream) => exchange_local(stream, request).await?,
            Transport::Remote(connection) => {
                let value = self
                    .control
                    .adapter_request_on_connection(connection, "zed", request)
                    .await
                    .map_err(anyhow::Error::msg)?;
                validate_zed_adapter_response(serde_json::from_value(value)?)?
            }
        };
        anyhow::ensure!(
            self.hub.code_scope_is_current(self.scope),
            "code context changed"
        );
        self.transport = Some(transport);
        Ok(response)
    }
}

async fn connect_local(socket: &FsPath) -> anyhow::Result<BufReader<UnixStream>> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(socket))
        .await
        .context("Zed adapter connect timed out")??;
    Ok(BufReader::new(stream))
}

/// Kept for the standalone socket contract fixtures. Production Session calls
/// use `Operation`, retaining the same connected peer for the whole sequence.
#[cfg(test)]
pub(super) async fn local_request(
    socket: &FsPath,
    request: serde_json::Value,
) -> anyhow::Result<ZedAdapterResponse> {
    exchange_local(&mut connect_local(socket).await?, request).await
}

async fn exchange_local(
    stream: &mut BufReader<UnixStream>,
    request: serde_json::Value,
) -> anyhow::Result<ZedAdapterResponse> {
    let mut bytes = serde_json::to_vec(&request)?;
    anyhow::ensure!(
        bytes.len() <= MAX_REQUEST_BYTES,
        "Zed adapter request exceeds byte limit"
    );
    bytes.push(b'\n');
    let exchange = async {
        // Bound writing too; a connected peer is not evidence that it reads.
        stream.get_mut().write_all(&bytes).await?;
        let mut line = Vec::new();
        (&mut *stream)
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .await?;
        anyhow::ensure!(
            line.len() <= MAX_RESPONSE_BYTES,
            "Zed adapter response exceeds byte limit"
        );
        anyhow::ensure!(
            line.last() == Some(&b'\n'),
            "Zed adapter response is incomplete"
        );
        validate_zed_adapter_response(serde_json::from_slice(&line)?)
    };
    tokio::time::timeout(EXCHANGE_TIMEOUT, exchange)
        .await
        .context("Zed adapter exchange timed out")?
}

pub(super) struct BufferRequest<'a> {
    pub worktree: &'a str,
    pub path: &'a str,
    pub lease_id: &'a str,
    pub open: bool,
}

#[derive(Debug)]
pub(super) enum BufferError {
    Unavailable(anyhow::Error),
    Request(anyhow::Error),
}

pub(super) async fn buffer_request(
    hub: &Hub,
    control: &MachineControl,
    local_socket: Option<&FsPath>,
    scope: &SessionCodeScope,
    request: BufferRequest<'_>,
) -> Result<ZedAdapterResponse, BufferError> {
    let mut operation = Operation::connect(hub, control, local_socket, scope)
        .await
        .map_err(|error| {
            if request.open {
                BufferError::Unavailable(error)
            } else {
                BufferError::Request(error)
            }
        })?;
    if request.open {
        let response = operation.request(
            serde_json::json!({"type":"ensureWorktree", "path":request.worktree, "trusted":true}),
        )
        .await
        .map_err(BufferError::Unavailable)?;
        if !matches!(response, ZedAdapterResponse::Worktree { state, .. } if state == "ready") {
            return Err(BufferError::Unavailable(anyhow::anyhow!(
                "Zed worktree is not ready"
            )));
        }
    }
    operation
        .request(serde_json::json!({
            "type": if request.open { "openBuffer" } else { "closeBuffer" },
            "worktree": request.worktree,
            "path": request.path,
            "leaseId": request.lease_id,
        }))
        .await
        .map_err(BufferError::Request)
}
