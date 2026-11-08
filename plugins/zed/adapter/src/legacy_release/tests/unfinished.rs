//! Unfinished Open recovery keeps both original native operation identities.
use super::*;
use crate::{Zed, sync_native::wire};
use prost::Message;
use wire::cowboy_open_attempt_response::Outcome as OpenOutcome;
type Outbound = tokio::sync::mpsc::UnboundedReceiver<proto::Envelope>;
type Private = tokio::sync::mpsc::Receiver<wire::CowboyBufferSyncEnvelope>;

async fn abandoned(f: &Fixture, zed: &Zed, outbound: &mut Outbound) -> u32 {
    crate::legacy_buffers::close_buffer(
        f.root.clone(),
        "file".into(),
        "owner".into(),
        &f.buffers,
        None,
    )
    .await
    .unwrap();
    let task = {
        let (root, trees, buffers, zed) = (
            f.root.clone(),
            f.trees.clone(),
            f.buffers.clone(),
            zed.clone(),
        );
        tokio::spawn(async move {
            crate::legacy_buffers::open_buffer(
                root,
                "file".into(),
                "owner".into(),
                &trees,
                &buffers,
                Some(&zed),
            )
            .await
        })
    };
    let opening = outbound.recv().await.unwrap();
    assert!(matches!(
        opening.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.buffers.native_open.check().is_err());
    assert!(f.buffers.active.read().await.is_empty());
    opening.id
}

async fn original(
    zed: &Zed,
    outbound: &mut Outbound,
    private: &mut Private,
    id: u32,
    outcome: OpenOutcome,
) {
    let ping = outbound.recv().await.unwrap();
    assert!(matches!(
        ping.payload,
        Some(proto::envelope::Payload::Ping(_))
    ));
    crate::native_open::tests::reply(zed, &ping, proto::envelope::Payload::Ack(proto::Ack {}))
        .await;
    let request = private.recv().await.unwrap();
    assert!(matches!(&request.payload, Some(Payload::OpenAttemptRequest(q)) if q.message_id == id));
    let bytes = wire::CowboyBufferSyncEnvelope {
        responding_to: Some(request.id),
        payload: Some(Payload::OpenAttemptResponse(
            wire::CowboyOpenAttemptResponse {
                protocol: 1,
                outcome: outcome as i32,
                buffer_id: 77,
            },
        )),
        ..Default::default()
    }
    .encode_to_vec();
    assert!(zed.sync.response(request.id, &bytes));
}

fn act(
    f: &Fixture,
    zed: &Zed,
    release: &Reference,
    action: Action,
) -> tokio::task::JoinHandle<Result<Response>> {
    let (buffers, zed, release) = (f.buffers.clone(), zed.clone(), release.clone());
    tokio::spawn(async move {
        respond(
            Request::LegacyRelease { release, action },
            &buffers,
            Some(&zed),
        )
        .await
    })
}

#[tokio::test]
async fn unfinished_open_prepares_without_effect_and_recovers_lost_original_cleanup() {
    let f = Fixture::new().await;
    let (transport, mut private) = crate::sync_native::Transport::new();
    let (zed, mut outbound) = crate::coordinate_queries::fixture_with(transport).await;
    let opening = abandoned(&f, &zed, &mut outbound).await;
    let release = f.prepare().await;
    assert_eq!(f.prepare().await.id, release.id);
    assert!(private.try_recv().is_err());
    assert!(outbound.try_recv().is_err());
    // Repeating Open cannot replace the evidence this preparation captured.
    assert!(
        crate::legacy_buffers::open_buffer(
            f.root.clone(),
            "file".into(),
            "owner".into(),
            &f.trees,
            &f.buffers,
            Some(&zed)
        )
        .await
        .is_err()
    );
    for outcome in [OpenOutcome::Pending, OpenOutcome::Forgotten] {
        let task = act(&f, &zed, &release, Action::Execute);
        original(&zed, &mut outbound, &mut private, opening, outcome).await;
        assert!(task.await.unwrap().is_err());
        assert_eq!(
            state(&f.act(&release, Action::Query).await.unwrap()),
            State::Prepared
        );
        assert!(private.try_recv().is_err());
    }
    let task = act(&f, &zed, &release, Action::Execute);
    original(
        &zed,
        &mut outbound,
        &mut private,
        opening,
        OpenOutcome::Opened,
    )
    .await;
    let probe = private.recv().await.unwrap();
    native_reply(&zed.sync, probe, Outcome::Supported);
    let close = private.recv().await.unwrap();
    let Some(Payload::CloseRequest(close)) = close.payload else {
        panic!("not Close")
    };
    assert_eq!(close.buffer_ids, [77]);
    assert_eq!(close.action, CloseAction::Close as i32);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        state(&f.act(&release, Action::Execute).await.unwrap()),
        State::Unknown
    );
    assert!(f.act(&release, Action::Retire).await.is_err());
    let task = act(&f, &zed, &release, Action::Query);
    original(
        &zed,
        &mut outbound,
        &mut private,
        opening,
        OpenOutcome::Opened,
    )
    .await;
    let query = private.recv().await.unwrap();
    assert!(
        matches!(&query.payload, Some(Payload::CloseRequest(q)) if q.action == CloseAction::Query as i32 && q.operation_id == close.operation_id && q.instance == close.instance && q.buffer_ids == close.buffer_ids)
    );
    native_reply(&zed.sync, query, Outcome::Closed);
    assert_eq!(state(&task.await.unwrap().unwrap()), State::Released);
    assert!(f.buffers.native_open.check().is_ok());
    // Later owner acquisition cannot be affected by the completed reference.
    f.open().await;
    assert_eq!(
        state(&f.act(&release, Action::Execute).await.unwrap()),
        State::Released
    );
    assert_eq!(f.buffers.active.read().await.len(), 1);
    assert!(outbound.try_recv().is_err());
}

#[tokio::test]
async fn failed_original_open_releases_without_native_close_or_replay() {
    let f = Fixture::new().await;
    let (transport, mut private) = crate::sync_native::Transport::new();
    let (zed, mut outbound) = crate::coordinate_queries::fixture_with(transport).await;
    let opening = abandoned(&f, &zed, &mut outbound).await;
    let release = f.prepare().await;
    let task = act(&f, &zed, &release, Action::Execute);
    original(
        &zed,
        &mut outbound,
        &mut private,
        opening,
        OpenOutcome::Failed,
    )
    .await;
    assert_eq!(state(&task.await.unwrap().unwrap()), State::Released);
    assert!(f.buffers.native_open.check().is_ok());
    assert!(outbound.try_recv().is_err());
    assert!(private.try_recv().is_err());
}
