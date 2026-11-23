use super::*;
use crate::code_adapter::{CodeAdapterRequest, CodeAdapterResponse, CodeOperation};
use crate::code_review::{CodeProvider as _, LocalCodeProvider};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

#[tokio::test]
async fn scoped_browser_tokens_round_trip_through_the_existing_adapter_socket() {
    let root = tempfile::tempdir().unwrap();
    let content = format!("a{}", "界".repeat(FILE_PAGE_BYTES + 11));
    std::fs::write(root.path().join("a.txt"), &content).unwrap();
    let socket = root.path().join("code.sock");
    let served_socket = socket.clone();
    let roots = vec![root.path().to_owned()];
    let server =
        tokio::spawn(async move { crate::code_adapter::serve(&served_socket, roots).await });
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists());
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    let mut public = None;
    let mut joined = String::new();
    for _ in 0..10 {
        let continuation = cache.resolve(&owner, "a.txt", public.as_deref()).unwrap();
        let request = CodeAdapterRequest {
            root: root.path().to_str().unwrap().into(),
            operation: CodeOperation::File {
                path: "a.txt".into(),
                cursor: continuation
                    .as_ref()
                    .map(|cursor| cursor.native_cursor().to_owned()),
            },
        };
        let mut stream = UnixStream::connect(&socket).await.unwrap();
        let mut wire = serde_json::to_vec(&request).unwrap();
        wire.push(b'\n');
        stream.write_all(&wire).await.unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).await.unwrap();
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["ok"], true);
        let CodeAdapterResponse::File(document) =
            serde_json::from_value(reply["value"].clone()).unwrap()
        else {
            panic!("wrong adapter reply")
        };
        let page = cache
            .project(&owner, "a.txt", continuation.as_ref(), document)
            .unwrap();
        joined.push_str(&page.text);
        public = page.next_cursor;
        if public.is_none() {
            break;
        }
    }
    assert_eq!(joined, content);
    server.abort();
    let _ = server.await;
}

#[test]
fn cached_and_uncached_reader_cursors_are_not_silently_interchanged() {
    let root = tempfile::tempdir().unwrap();
    let content = "x".repeat(FILE_PAGE_BYTES + 30);
    std::fs::write(root.path().join("a.txt"), &content).unwrap();
    let provider = LocalCodeProvider::new(root.path());
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    let first = cache
        .project(
            &owner,
            "a.txt",
            None,
            provider.file_page("a.txt", None).unwrap(),
        )
        .unwrap();
    let continuation = cache
        .resolve(&owner, "a.txt", first.next_cursor.as_deref())
        .unwrap()
        .unwrap();
    // Physical-file revisions and content-cache digests remain independent.
    let cached =
        crate::code_review::cached_file_page("a.txt", content.into_bytes(), "b".repeat(64), None)
            .unwrap();
    assert_eq!(
        cache
            .project(&owner, "a.txt", Some(&continuation), cached)
            .unwrap_err(),
        CursorError::Changed
    );
}
