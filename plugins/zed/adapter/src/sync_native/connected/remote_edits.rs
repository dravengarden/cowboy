//! The actual native RPC refuses whole batches; no product edit API is added.
use super::*;

async fn update(zed: &ZedRuntime, buffer: u64, operations: Vec<proto::Operation>) -> Result<()> {
    zed.request(proto::envelope::Payload::UpdateBuffer(
        proto::UpdateBuffer {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            buffer_id: buffer,
            operations,
        },
    ))
    .await?;
    Ok(())
}

fn native_refusal(error: &anyhow::Error) {
    assert!(
        error.to_string().starts_with("Zed request failed:"),
        "{error}"
    );
}

pub(super) async fn exercise(zed: &ZedRuntime, root: &Path, worktree: u64) {
    let path = root.join("remote-update.txt");
    let base = "a🙂z";
    tokio::fs::write(&path, base).await.unwrap();
    let (buffer, _) = zed
        .open_buffer(worktree, Path::new("remote-update.txt"))
        .await
        .unwrap();
    let mut peer = coordinates::tests::peer(base, 1);
    let first = coordinates::tests::wire(&peer.edit([(0..0, "first")]));
    let mut bad = first.clone();
    let proto::operation::Variant::Edit(edit) = bad.variant.as_mut().unwrap() else {
        unreachable!()
    };
    edit.replica_id = 65537;
    native_refusal(
        &update(zed, buffer, vec![first.clone(), bad])
            .await
            .unwrap_err(),
    );
    native_refusal(
        &update(zed, buffer, vec![first.clone(); 129])
            .await
            .unwrap_err(),
    );
    native_refusal(
        &update(zed, u64::MAX, vec![first.clone()])
            .await
            .unwrap_err(),
    );
    assert_native_mirror(zed, buffer, base).await;
    update(zed, buffer, vec![first.clone()]).await.unwrap();
    assert_native_mirror(zed, buffer, "firsta🙂z").await;
    let version =
        serde_json::to_value(zed.diagnostics.lock().unwrap().version(buffer).unwrap()).unwrap();
    update(zed, buffer, vec![first.clone()]).await.unwrap();
    assert_eq!(
        serde_json::to_value(zed.diagnostics.lock().unwrap().version(buffer).unwrap()).unwrap(),
        version
    );
    let proto::operation::Variant::Edit(edit) = first.variant.unwrap() else {
        unreachable!()
    };
    let mut conflict = edit;
    conflict.new_text[0] = "different".into();
    native_refusal(
        &update(
            zed,
            buffer,
            vec![proto::Operation {
                variant: Some(proto::operation::Variant::Edit(conflict)),
            }],
        )
        .await
        .unwrap_err(),
    );
    let valid = coordinates::tests::wire(&peer.edit([(6..10, "")]));
    update(zed, buffer, vec![valid]).await.unwrap();
    assert_native_mirror(zed, buffer, "firstaz").await;
    let next = coordinates::tests::wire(&peer.edit([(6..6, "ok")]));
    let mut bad = next.clone();
    let proto::operation::Variant::Edit(edit) = bad.variant.as_mut().unwrap() else {
        unreachable!()
    };
    edit.ranges[0] = proto::Range { start: 7, end: 8 }; // Inside the deleted emoji.
    native_refusal(&update(zed, buffer, vec![bad]).await.unwrap_err());
    update(zed, buffer, vec![next]).await.unwrap();
    assert_native_mirror(zed, buffer, "firstaokz").await;
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), base);
    zed.close_buffer(buffer).unwrap();
    // A following foreground request observes the earlier close; even if another
    // native holder remains, this peer can no longer edit the old resource.
    native_refusal(&update(zed, buffer, vec![]).await.unwrap_err());
    println!(
        "native edit ingress: actual whole-batch/clock/unknown-ID refusal, exact deduplication, Unicode tombstones, unchanged disk and closed-peer refusal passed"
    );
}
