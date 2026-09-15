use super::*;

#[test]
fn long_utf8_lines_reassemble_without_cutting_a_character() {
    let root = tempfile::tempdir().unwrap();
    for character in ["é", "界", "🦀"] {
        // Shift each multibyte sequence across the 256 KiB page boundary.
        let content = format!("a{}", character.repeat(FILE_PAGE_BYTES + 7));
        std::fs::write(root.path().join("long.txt"), &content).unwrap();
        let provider = LocalCodeProvider::new(root.path());
        let mut joined = String::new();
        let mut cursor = None;
        for _ in 0..10 {
            let page = provider.file_page("long.txt", cursor.as_deref()).unwrap();
            assert!(!page.text.is_empty());
            joined.push_str(&page.text);
            if page.next_cursor.is_none() {
                break;
            }
            assert_ne!(page.next_cursor, cursor);
            cursor = page.next_cursor;
        }
        assert_eq!(joined, content);
    }
}

#[cfg(feature = "full")]
#[test]
fn cached_utf8_lines_reassemble_without_cutting_a_character() {
    for character in ["é", "界", "🦀"] {
        let content = format!("a{}", character.repeat(FILE_PAGE_BYTES + 7));
        let mut joined = String::new();
        let mut cursor = None;
        for _ in 0..10 {
            let page = cached_file_page(
                "long.txt",
                content.as_bytes().to_vec(),
                "a".repeat(64),
                cursor.as_deref(),
            )
            .unwrap();
            assert!(!page.text.is_empty());
            joined.push_str(&page.text);
            if page.next_cursor.is_none() {
                break;
            }
            assert_ne!(page.next_cursor, cursor);
            cursor = page.next_cursor;
        }
        assert_eq!(joined, content);
    }
}

#[test]
fn incomplete_utf8_at_eof_is_an_error_not_a_nonprogressing_cursor() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("invalid.txt"), b"a\xe7").unwrap();
    assert_eq!(
        LocalCodeProvider::new(root.path())
            .file("invalid.txt")
            .unwrap_err(),
        "file is not UTF-8"
    );
    #[cfg(feature = "full")]
    assert_eq!(
        cached_file_page("invalid.txt", b"a\xe7".to_vec(), "a".repeat(64), None).unwrap_err(),
        "file is not UTF-8"
    );
}

#[test]
fn read_boundaries_trim_only_partial_codepoints_and_preserve_complete_tails() {
    assert_eq!(
        file_page_text(b"ok\xe7", true).unwrap(),
        ("ok".into(), false)
    );
    assert_eq!(
        file_page_text(b"ok\nlast", false).unwrap(),
        ("ok\nlast".into(), false)
    );
    assert_eq!(
        file_page_text(b"ok\nlast", true).unwrap(),
        ("ok\n".into(), true)
    );
    assert_eq!(
        file_page_text(b"\xe7", false).unwrap_err(),
        "file is not UTF-8"
    );
    assert_eq!(file_page_text(b"a\0b", true).unwrap_err(), "binary file");
    assert_eq!(
        file_page_text(b"a\xffb", true).unwrap_err(),
        "file is not UTF-8"
    );
}

#[test]
fn file_view_limit_does_not_emit_a_cursor_into_a_partial_codepoint() {
    let root = tempfile::tempdir().unwrap();
    let mut content = vec![b'x'; MAX_FILE_BYTES - 1];
    content.extend_from_slice("界tail".as_bytes());
    std::fs::write(root.path().join("limited.txt"), &content).unwrap();
    let provider = LocalCodeProvider::new(root.path());
    let first = provider.file("limited.txt").unwrap();
    let cursor = format!("{}:{}", first.revision, MAX_FILE_BYTES - FILE_PAGE_BYTES);
    let last = provider.file_page("limited.txt", Some(&cursor)).unwrap();
    assert_eq!(last.text, "x".repeat(FILE_PAGE_BYTES - 1));
    assert!(last.limited && last.truncated);
    assert!(last.next_cursor.is_none());
    #[cfg(feature = "full")]
    {
        let cursor = format!("{}:{}", "a".repeat(64), MAX_FILE_BYTES - FILE_PAGE_BYTES);
        let cached =
            cached_file_page("limited.txt", content, "a".repeat(64), Some(&cursor)).unwrap();
        assert_eq!(cached.text, last.text);
        assert!(cached.limited && cached.truncated);
        assert!(cached.next_cursor.is_none());
    }
}
