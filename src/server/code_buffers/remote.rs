//! Closed optional Machine/private-Code protocol. Native references never
//! leave the Controller or become browser-supplied execution authority.

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::code_buffer_read;
use crate::machine_control::{ConnectionToken, MachineControl};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeRef {
    instance: String,
    id: String,
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LeaseState {
    Prepared,
    Open,
    Released,
    Unknown,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(rename = "type")]
    kind: String,
    api_version: u8,
    lease: NativeRef,
    state: LeaseState,
}

impl Reply {
    fn parse(value: Value) -> Result<Self> {
        let reply: Self = serde_json::from_value(value).context("invalid buffer reply")?;
        ensure!(
            reply.kind == "bufferLease" && reply.api_version == 1,
            "unsupported buffer reply"
        );
        ensure!(
            hex(&reply.lease.instance, 32)
                && hex(&reply.lease.id, 16)
                && reply.lease.id != "0000000000000000",
            "invalid native buffer reference"
        );
        Ok(reply)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Action {
    Open,
    Query,
    Release,
}

pub(super) async fn support(control: &MachineControl, connection: &ConnectionToken) -> Result<()> {
    let value = control
        .adapter_request_on_connection(connection, "zed", json!({"type":"bufferLeaseSupport"}))
        .await
        .map_err(anyhow::Error::msg)?;
    // This pathless probe is answered by Machine core. An adapter's health
    // field cannot establish the host routing/retention capability.
    ensure!(
        value == json!({"type":"bufferLeaseSupport", "api_version":1}),
        "owned buffers unavailable"
    );
    Ok(())
}

pub(super) async fn prepare(
    control: &MachineControl,
    connection: &ConnectionToken,
    worktree: &str,
    path: &str,
) -> Result<NativeRef> {
    let value = control
        .adapter_request_on_connection(
            connection,
            "zed",
            json!({"type":"prepareBuffer", "worktree":worktree, "path":path}),
        )
        .await
        .map_err(anyhow::Error::msg)?;
    let reply = Reply::parse(value)?;
    ensure!(
        reply.state == LeaseState::Prepared,
        "buffer was not prepared"
    );
    Ok(reply.lease)
}

pub(super) async fn request(
    control: &MachineControl,
    connection: &ConnectionToken,
    lease: &NativeRef,
    action: Action,
) -> Result<LeaseState> {
    let kind = match action {
        Action::Open => "openBufferLease",
        Action::Query => "queryBufferLease",
        Action::Release => "releaseBufferLease",
    };
    let value = control
        .adapter_request_on_connection(connection, "zed", json!({"type":kind, "lease":lease}))
        .await
        .map_err(anyhow::Error::msg)?;
    let reply = Reply::parse(value)?;
    ensure!(&reply.lease == lease, "buffer reply owner changed");
    ensure!(
        match action {
            Action::Open => matches!(reply.state, LeaseState::Open | LeaseState::Unknown),
            Action::Release => matches!(reply.state, LeaseState::Released | LeaseState::Unknown),
            Action::Query => true,
        },
        "unexpected buffer state"
    );
    Ok(reply.state)
}

pub(super) async fn read_support(
    control: &MachineControl,
    connection: &ConnectionToken,
) -> Result<()> {
    let value = control
        .adapter_request_on_connection(connection, "zed", json!({"type":"bufferLeaseReadSupport"}))
        .await
        .map_err(anyhow::Error::msg)?;
    ensure!(
        value == json!({"type":"bufferLeaseReadSupport", "api_version":1}),
        "owned buffer reads unavailable"
    );
    Ok(())
}

pub(super) async fn read(
    control: &MachineControl,
    connection: &ConnectionToken,
    lease: &NativeRef,
    request: code_buffer_read::Request,
) -> Result<code_buffer_read::Reply<NativeRef>> {
    let value = control
        .adapter_request_on_connection(
            connection,
            "zed",
            json!({"type":"readBufferLease", "lease":lease, "request":request}),
        )
        .await
        .map_err(anyhow::Error::msg)?;
    code_buffer_read::Reply::parse(&value, lease, request)
}
