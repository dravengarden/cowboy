use super::*;
use axum::body::to_bytes;

#[tokio::test]
async fn etags_cover_exact_pages_and_continuation_lifetimes() {
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    let first = cache
        .project(&owner, "a.txt", None, page("a.txt", 0, true))
        .unwrap();
    let first_response = response(&HeaderMap::new(), first.clone());
    let etag = first_response.headers()[header::ETAG].clone();
    let body = to_bytes(first_response.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["nextCursor"], first.next_cursor.as_deref().unwrap());
    assert_eq!(json["apiVersion"], 1);
    assert_eq!(json["text"], first.text);
    assert_eq!(json["revision"], first.revision);

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, etag.clone());
    let same = cache
        .project(&owner, "a.txt", None, page("a.txt", 0, true))
        .unwrap();
    let unchanged = response(&headers, same);
    assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(unchanged.headers()[header::CACHE_CONTROL], CACHE_CONTROL);
    assert!(
        to_bytes(unchanged.into_body(), 4096)
            .await
            .unwrap()
            .is_empty()
    );

    let cursor = cache
        .resolve(&owner, "a.txt", first.next_cursor.as_deref())
        .unwrap()
        .unwrap();
    // Even repeated identical text under one revision is a different page when
    // it has a different continuation; the old revision-only ETag hid this.
    let next = cache
        .project(&owner, "a.txt", Some(&cursor), page("a.txt", 5, true))
        .unwrap();
    let next_response = response(&headers, next);
    assert_eq!(next_response.status(), StatusCode::OK);
    assert_ne!(next_response.headers()[header::ETAG], etag);

    cache
        .entries
        .lock()
        .iter_mut()
        .for_each(|entry| entry.touched = Instant::now() - TTL);
    let regenerated = cache
        .project(&owner, "a.txt", None, page("a.txt", 0, true))
        .unwrap();
    let fresh = response(&headers, regenerated);
    assert_eq!(fresh.status(), StatusCode::OK);
    assert_ne!(fresh.headers()[header::ETAG], etag);
}

#[tokio::test]
async fn conditional_reads_match_exact_strong_weak_list_or_wildcard_tags() {
    let document = page("a.txt", 0, false);
    let original = response(&HeaderMap::new(), document.clone());
    let etag = original.headers()[header::ETAG].to_str().unwrap();
    for matching in [
        etag.to_owned(),
        format!("W/{etag}"),
        format!("\"other\", W/{etag}"),
        "*".into(),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, matching.parse().unwrap());
        assert_eq!(
            response(&headers, document.clone()).status(),
            StatusCode::NOT_MODIFIED
        );
    }
    for nonmatching in [
        format!("junk{etag}"),
        format!("{etag}junk"),
        format!("\"{}\"", document.revision),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, nonmatching.parse().unwrap());
        assert_eq!(
            response(&headers, document.clone()).status(),
            StatusCode::OK
        );
    }
    let mut multiple = HeaderMap::new();
    multiple.append(header::IF_NONE_MATCH, "\"other\"".parse().unwrap());
    multiple.append(header::IF_NONE_MATCH, etag.parse().unwrap());
    assert_eq!(
        response(&multiple, document).status(),
        StatusCode::NOT_MODIFIED
    );
}

#[tokio::test]
async fn all_cursor_failures_are_closed_and_not_cacheable() {
    for (error, status) in [
        (CursorError::Invalid, StatusCode::BAD_REQUEST),
        (CursorError::Expired, StatusCode::GONE),
        (CursorError::Changed, StatusCode::CONFLICT),
        (CursorError::InvalidPage, StatusCode::BAD_GATEWAY),
        (CursorError::Capacity, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let response = error.into_response();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!response.headers().contains_key(header::ETAG));
    }
}

#[tokio::test]
async fn scope_change_after_projection_still_discards_a_file_page_response() {
    let cache = PageCursors::default();
    let hub = Hub::new();
    let owner = create(&hub, "one");
    let CodeReadScope::Session(session) = &owner else {
        unreachable!()
    };
    let result = crate::server::code_reads::guarded_response(
        || async { hub.code_scope_is_current(session) },
        || async {
            let first = cache
                .project(&owner, "a.txt", None, page("a.txt", 0, true))
                .unwrap();
            hub.update_session_cwd("one", "/work/b".into()).unwrap();
            response(&HeaderMap::new(), first)
        },
    )
    .await;
    assert_eq!(result.status(), StatusCode::GONE);
    assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
    assert!(!result.headers().contains_key(header::ETAG));
    assert_eq!(
        to_bytes(result.into_body(), 1024).await.unwrap(),
        "code context changed"
    );
}
