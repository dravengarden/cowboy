//! Actual bounded-diff refusal on the original native buffer, no partial apply.
use super::*;
use std::fmt::Write as _;

pub(super) async fn exercise(zed: &ZedRuntime, instance: &[u8], root: &Path, worktree: u64) {
    let mut original = String::new();
    let mut too_many = String::new();
    for i in 0..=1024 {
        writeln!(original, "old-{i}\nanchor-{i}").unwrap();
        writeln!(too_many, "new-{i}\nanchor-{i}").unwrap();
    }
    let path = root.join("replacement-diff.txt");
    tokio::fs::write(&path, &original).await.unwrap();
    let (buffer, _) = zed
        .open_buffer(worktree, Path::new("replacement-diff.txt"))
        .await
        .unwrap();
    let version = zed.diagnostics.lock().unwrap().version(buffer).unwrap();
    tokio::fs::write(&path, &too_many).await.unwrap();
    let error = reload::reload(zed, buffer)
        .await
        .expect_err("1025 native diff edits must refuse the whole reload");
    assert!(error.to_string().starts_with("Zed request failed:"));
    assert_native_mirror(zed, buffer, &original).await;
    // Prepare checks the original native version/clean state, not a cached mirror.
    let ticket = prepare(zed, instance, buffer, &version, too_many.as_bytes())
        .await
        .unwrap();
    action(zed, instance, ticket, Action::Apply).await.unwrap();
    let refusal = finish(zed, instance, ticket).await;
    assert_eq!(refusal.phase, Phase::Refused as i32);
    assert_eq!(refusal.refusal, Refusal::Budget as i32);
    assert_eq!(
        action(zed, instance, ticket, Action::Apply).await.unwrap(),
        refusal
    );
    assert_native_mirror(zed, buffer, &original).await;
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), too_many);
    action(zed, instance, ticket, Action::Retire).await.unwrap();
    // A separately requested smaller replacement is still usable. No retry/fallback
    // is added to the product; only this fixture explicitly invokes legacy reload.
    tokio::fs::write(&path, "separate🙂\n").await.unwrap();
    reload::reload(zed, buffer).await.unwrap();
    assert_native_mirror(zed, buffer, "separate🙂\n").await;
    zed.close_buffer(buffer).unwrap();
    println!(
        "native replacement: bounded whole-diff refusal, typed sync Budget, original-ID no replay, unchanged text/source and independent later reload passed"
    );
}
