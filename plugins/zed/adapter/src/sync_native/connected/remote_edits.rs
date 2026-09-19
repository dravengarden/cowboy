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

async fn native_version(
    zed: &ZedRuntime,
    buffer: u64,
    version: &clock::Global,
    anchor: &proto::Anchor,
) -> Result<()> {
    // Remote edits are not echoed to their sender. Check the actual native
    // version through an independent, exact-version read, not the local mirror.
    // This plaintext buffer has no LSP and cannot acquire any destinations.
    let version: Vec<_> = version
        .iter()
        .map(|entry| BufferVersionEntry {
            replica_id: u32::from(entry.replica_id.as_u16()),
            timestamp: entry.value,
        })
        .collect();
    let results = crate::navigation_native::query(
        zed,
        crate::navigation_request(buffer, &version, anchor.clone(), NavigationKind::Definition),
    )
    .await?;
    assert!(results.is_empty());
    Ok(())
}

async fn malformed_batches(zed: &ZedRuntime, buffer: u64, first: &proto::Operation) {
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
    let base_version = peer.version();
    let anchor = zed
        .diagnostics
        .lock()
        .unwrap()
        .position(buffer, 0, 0)
        .unwrap()
        .anchor;
    let first = coordinates::tests::wire(&peer.edit([(0..0, "first")]));
    malformed_batches(zed, buffer, &first).await;
    native_version(zed, buffer, &base_version, &anchor)
        .await
        .unwrap();
    update(zed, buffer, vec![first.clone()]).await.unwrap();
    native_version(zed, buffer, &peer.version(), &anchor)
        .await
        .unwrap();
    // The read must actually fence its input, not accept any supplied vector.
    let stale = native_version(zed, buffer, &base_version, &anchor)
        .await
        .unwrap_err();
    assert_eq!(
        stale
            .downcast_ref::<crate::navigation_native::NativeRefusal>()
            .unwrap()
            .0,
        wire::cowboy_navigation_response::Refusal::Source
    );
    update(zed, buffer, vec![first.clone()]).await.unwrap();
    native_version(zed, buffer, &peer.version(), &anchor)
        .await
        .unwrap();
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
    native_version(zed, buffer, &peer.version(), &anchor)
        .await
        .unwrap();
    let valid = coordinates::tests::wire(&peer.edit([(6..10, "")]));
    update(zed, buffer, vec![valid]).await.unwrap();
    native_version(zed, buffer, &peer.version(), &anchor)
        .await
        .unwrap();
    let before_next = peer.version();
    let next = coordinates::tests::wire(&peer.edit([(6..6, "ok")]));
    let mut bad = next.clone();
    let proto::operation::Variant::Edit(edit) = bad.variant.as_mut().unwrap() else {
        unreachable!()
    };
    edit.ranges[0] = proto::Range { start: 7, end: 8 }; // Inside the deleted emoji.
    native_refusal(&update(zed, buffer, vec![bad]).await.unwrap_err());
    native_version(zed, buffer, &before_next, &anchor)
        .await
        .unwrap();
    update(zed, buffer, vec![next]).await.unwrap();
    native_version(zed, buffer, &peer.version(), &anchor)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), base);
    zed.close_buffer(buffer).unwrap();
    // A following foreground request observes the earlier close; even if another
    // native holder remains, this peer can no longer edit the old resource.
    native_refusal(&update(zed, buffer, vec![]).await.unwrap_err());
    println!(
        "native edit ingress: actual whole-batch/clock/unknown-ID refusal, independent exact native-version reads, deduplication, Unicode tombstones, unchanged disk and closed-peer refusal passed (complete content checked in native GPUI tests)"
    );
}
