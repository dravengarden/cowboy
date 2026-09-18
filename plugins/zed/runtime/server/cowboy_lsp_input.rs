// SPDX-License-Identifier: GPL-3.0-or-later
//! Private headless distribution input limits, before allocation/deserialization.
//! These bound individual inputs, not total process memory or LSP query fanout.
use anyhow::{Context as _, Result, ensure};
use futures::{AsyncRead, AsyncReadExt as _, io::BufReader};

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

pub(super) async fn read_headers<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    // Do not use read_until: a single unterminated line can allocate without a
    // bound before the caller gets a chance to check its length. BufReader
    // amortizes these single-byte reads; it does not grow with the header.
    loop {
        ensure!(
            buffer.len() <= MAX_HEADER_BYTES,
            "LSP headers exceed budget"
        );
        if buffer.ends_with(b"\r\n\r\n") {
            return Ok(());
        }
        ensure!(buffer.len() < MAX_HEADER_BYTES, "LSP headers exceed budget");
        let mut byte = [0];
        reader
            .read_exact(&mut byte)
            .await
            .context("incomplete LSP headers")?;
        buffer.push(byte[0]);
    }
}

pub(super) fn content_length(headers: &[u8]) -> Result<usize> {
    ensure!(
        headers.len() <= MAX_HEADER_BYTES && headers.ends_with(b"\r\n\r\n"),
        "invalid LSP headers"
    );
    let headers = std::str::from_utf8(headers).context("invalid LSP header encoding")?;
    let mut length = None;
    for line in headers[..headers.len() - 4].split("\r\n") {
        let (name, value) = line.split_once(':').context("invalid LSP header field")?;
        ensure!(
            !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && value
                    .bytes()
                    .all(|b| b == b'\t' || (b' '..=b'~').contains(&b)),
            "invalid LSP header field"
        );
        if name.eq_ignore_ascii_case("Content-Length") {
            ensure!(length.is_none(), "duplicate LSP length");
            let value = value.trim_matches([' ', '\t']);
            ensure!(
                !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                "invalid LSP length"
            );
            let value: usize = value.parse().context("invalid LSP length")?;
            ensure!(
                value > 0 && value <= MAX_MESSAGE_BYTES,
                "LSP message exceeds budget"
            );
            length = Some(value);
        }
    }
    length.context("LSP length missing")
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;

    #[test]
    fn cowboy_lsp_length_is_checked_before_body_allocation() {
        for length in [1, MAX_MESSAGE_BYTES] {
            assert_eq!(
                content_length(format!("Content-Length: {length}\r\n\r\n").as_bytes()).unwrap(),
                length
            );
        }
        for value in [
            "0",
            "2097153",
            "18446744073709551616",
            "-1",
            "+1",
            "1 2",
            "",
        ] {
            assert!(content_length(format!("Content-Length: {value}\r\n\r\n").as_bytes()).is_err());
        }
        assert_eq!(
            content_length(
                b"Content-Type: application/vscode-jsonrpc\r\ncontent-length:\t7 \r\n\r\n"
            )
            .unwrap(),
            7
        );
    }

    #[test]
    fn cowboy_lsp_ambiguous_or_malformed_headers_refuse_without_echoing_input() {
        for headers in [
            &b"Content-Length: 1\r\nContent-Length: 1\r\n\r\n"[..],
            b"Content-Length: 1\r\ncontent-length: 2\r\n\r\n",
            b"Content-Type: secret-fixture-value\r\n\r\n",
            b"Content-Length: 2\n\n",
            b"Content-Length: secret-fixture-value\r\n\r\n",
            b"Content-Length: 1\r\nX-Test: bad\0value\r\n\r\n",
            b" Content-Length: 1\r\n\r\n",
        ] {
            let error = content_length(headers).unwrap_err();
            assert!(!format!("{error:#}").contains("secret-fixture-value"));
        }
    }

    #[gpui::test]
    async fn cowboy_lsp_handler_refuses_oversize_before_reading_body() {
        let (sender, _receiver) = futures::channel::mpsc::channel(1);
        // No body exists. The actual dispatcher must report the declared size,
        // not an EOF from trying to read/allocate that body first.
        let error = super::super::LspStdoutHandler::handler(
            Cursor::new(b"Content-Length: 2097153\r\n\r\n"),
            sender,
            Default::default(),
            Default::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "LSP message exceeds budget");
    }

    #[gpui::test]
    async fn cowboy_lsp_headers_have_an_inclusive_streaming_budget() {
        let prefix = b"Content-Length: 2\r\nX-Padding: ";
        let mut exact = prefix.to_vec();
        exact.resize(MAX_HEADER_BYTES - 4, b'x');
        exact.extend_from_slice(b"\r\n\r\n{}");
        let mut reader = BufReader::new(Cursor::new(exact));
        let mut headers = Vec::new();
        read_headers(&mut reader, &mut headers).await.unwrap();
        assert_eq!(headers.len(), MAX_HEADER_BYTES);
        assert_eq!(content_length(&headers).unwrap(), 2);
        let mut body = [0; 2];
        reader.read_exact(&mut body).await.unwrap();
        assert_eq!(body, *b"{}");

        // Both one long line and many short lines stop at the same bound.
        for bytes in [vec![b'x'; MAX_HEADER_BYTES + 1], b"X: y\r\n".repeat(2000)] {
            let mut reader = BufReader::new(Cursor::new(bytes));
            let mut headers = Vec::new();
            assert!(read_headers(&mut reader, &mut headers).await.is_err());
            assert_eq!(headers.len(), MAX_HEADER_BYTES);
        }
        let mut reader = BufReader::new(Cursor::new(b"Content-Length: 2\r\n"));
        assert!(read_headers(&mut reader, &mut Vec::new()).await.is_err());
    }
}
