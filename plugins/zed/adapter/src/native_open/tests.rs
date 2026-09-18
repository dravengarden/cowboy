use super::*;

pub(crate) async fn reply(
    zed: &ZedRuntime,
    request: &proto::Envelope,
    payload: proto::envelope::Payload,
) {
    zed.pending
        .lock()
        .await
        .remove(&request.id)
        .unwrap()
        .send(proto::Envelope {
            responding_to: Some(request.id),
            payload: Some(payload),
            ..Default::default()
        })
        .unwrap();
}

pub(crate) fn share(zed: &ZedRuntime, id: u64) {
    for variant in [
        proto::create_buffer_for_peer::Variant::State(proto::BufferState {
            id,
            base_text: "a🙂z\n".into(),
            ..Default::default()
        }),
        proto::create_buffer_for_peer::Variant::Chunk(proto::BufferChunk {
            buffer_id: id,
            is_last: true,
            ..Default::default()
        }),
    ] {
        let payload = proto::envelope::Payload::CreateBufferForPeer(proto::CreateBufferForPeer {
            variant: Some(variant),
            ..Default::default()
        });
        zed.diagnostics.lock().unwrap().observe(&payload);
        zed.events
            .send(proto::Envelope {
                payload: Some(payload),
                ..Default::default()
            })
            .unwrap();
    }
}

pub(crate) fn opened(id: u64) -> proto::envelope::Payload {
    proto::envelope::Payload::OpenBufferResponse(proto::OpenBufferResponse { buffer_id: id })
}

pub(crate) fn ack() -> proto::envelope::Payload {
    proto::envelope::Payload::Ack(proto::Ack {})
}

#[test]
fn only_the_one_use_commit_can_remove_the_fence() {
    let fence = Fence::default();
    let attempt = fence.begin().unwrap();
    assert!(fence.check().is_err());
    assert!(fence.begin().is_err());
    attempt.complete();
    assert!(fence.check().is_ok());
    {
        let _unobserved = fence.begin().unwrap();
    }
    assert!(fence.check().is_err());
    assert!(fence.begin().is_err());
}

#[tokio::test]
async fn initial_share_timeout_never_closes_reopens_or_registers() {
    let (zed, mut outbound) = crate::coordinate_queries::fixture().await;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.open_buffer(1, Path::new("source")).await })
    };
    let request = outbound.recv().await.unwrap();
    assert!(matches!(
        request.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    reply(&zed, &request, opened(8)).await;
    // Real production five-second deadline, not a shortened fixture timer.
    let error = tokio::time::timeout(Duration::from_secs(7), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("initial buffer state"));
    assert!(
        outbound.try_recv().is_err(),
        "timeout dispatched another native effect"
    );
}

#[tokio::test]
async fn lost_initial_stream_never_closes_or_reopens() {
    let (zed, mut outbound) = crate::coordinate_queries::fixture().await;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.open_buffer(1, Path::new("source")).await })
    };
    let request = outbound.recv().await.unwrap();
    // Overflow the real broadcast queue while the response is still held.
    for _ in 0..64 {
        zed.events.send(proto::Envelope::default()).unwrap();
    }
    reply(&zed, &request, opened(8)).await;
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("stream is unavailable")
    );
    assert!(outbound.try_recv().is_err());
}

#[tokio::test]
async fn initial_state_before_reply_is_not_lost_and_registration_is_once() {
    let (zed, mut outbound) = crate::coordinate_queries::fixture().await;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.open_buffer(1, Path::new("source")).await })
    };
    let request = outbound.recv().await.unwrap();
    share(&zed, 8);
    reply(&zed, &request, opened(8)).await;
    let registration = outbound.recv().await.unwrap();
    assert!(matches!(&registration.payload,
        Some(proto::envelope::Payload::RegisterBufferWithLanguageServers(value)) if value.buffer_id == 8));
    reply(&zed, &registration, ack()).await;
    assert_eq!(task.await.unwrap().unwrap().0, 8);
    assert!(outbound.try_recv().is_err());
}
