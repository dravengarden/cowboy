use super::{Client, Path, Request};
use crate::legacy_release::{Action, Reference};
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;

pub(super) async fn exercise(client: &Client, root: &Path) {
    let workspace = root.join("legacy-release-worktree");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("file.txt"), "retained🙂\n").unwrap();
    client
        .request(Request::OpenWorktree {
            path: workspace.clone(),
            trusted: true,
        })
        .await;
    assert_eq!(
        client.request(Request::LegacyReleaseSupport {}).await["protocol"],
        1
    );
    let open = || Request::OpenBuffer {
        worktree: workspace.clone(),
        path: "file.txt".into(),
        lease_id: "automatic-fixture".into(),
    };
    client.request(open()).await;
    let prepared = client
        .request(Request::PrepareLegacyRelease {
            worktree: workspace.clone(),
            path: "file.txt".into(),
            lease_id: "automatic-fixture".into(),
        })
        .await;
    assert_eq!(prepared["state"], "prepared");
    let release: Reference = serde_json::from_value(prepared["release"].clone()).unwrap();
    let mut disconnected = tokio::net::UnixStream::connect(&client.0).await.unwrap();
    let mut bytes = serde_json::to_vec(&Request::LegacyRelease {
        release: release.clone(),
        action: Action::Execute,
    })
    .unwrap();
    bytes.push(b'\n');
    disconnected.write_all(&bytes).await.unwrap();
    drop(disconnected); // Discard the actual Close response, never invent it.
    await_release(client, &release).await;
    client.request(open()).await;
    assert_eq!(
        client
            .request(Request::LegacyRelease {
                release: release.clone(),
                action: Action::Execute
            })
            .await["state"],
        "released"
    );
    // Reusing the old release cannot touch the newly opened legacy pin.
    let language = client
        .request(Request::BufferLanguage {
            worktree: workspace.clone(),
            path: "file.txt".into(),
        })
        .await;
    assert_eq!(language["sourceState"], "matching");
    assert_eq!(
        client
            .request(Request::LegacyRelease {
                release: release.clone(),
                action: Action::Retire
            })
            .await["state"],
        "retired"
    );
    assert_eq!(
        client
            .request(Request::LegacyRelease {
                release,
                action: Action::Query
            })
            .await["state"],
        "retired"
    );
    client
        .request(Request::CloseBuffer {
            worktree: workspace,
            path: "file.txt".into(),
            lease_id: "automatic-fixture".into(),
        })
        .await;
    println!(
        "immutable legacy release: real discarded Close reply, original-reference Query, recreated owner survives old Execute, explicit receipt retirement passed"
    );
}

async fn await_release(client: &Client, release: &Reference) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let reply = client
                .request(Request::LegacyRelease {
                    release: release.clone(),
                    action: Action::Query,
                })
                .await;
            if reply["state"] == "released" {
                assert_eq!(reply["buffer"]["type"], "buffer");
                break;
            }
            assert!(matches!(
                reply["state"].as_str(),
                Some("prepared" | "unknown")
            ));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("original real Close did not settle");
}
