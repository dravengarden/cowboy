//! Upgrade headers advertising host-layer transport and broker capabilities.

pub(super) fn attach(
    request: &mut tokio_tungstenite::tungstenite::http::Request<()>,
) -> anyhow::Result<()> {
    request.headers_mut().insert(
        crate::machine_transport::HEADER,
        crate::machine_transport::CHUNKED.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::COMPRESSION_HEADER,
        crate::machine_transport::DEFLATE.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::PACING_HEADER,
        crate::machine_transport::CREDIT.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::STREAM_HEADER,
        crate::machine_transport::STREAM.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_lanes::LANES_HEADER,
        crate::machine_lanes::LANES.parse()?,
    );
    request.headers_mut().insert(
        crate::machine_transport::HIBERNATION_EPOCH_HEADER,
        crate::machine_transport::HIBERNATION_EPOCH_V1.parse()?,
    );
    Ok(())
}
