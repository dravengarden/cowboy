//! Cancellation-safe bounded native output. A partial line survives select!
//! cancellation, while a process without newlines cannot grow memory forever.
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};

const MAX_LINE: usize = 64 * 1024;
const MAX_TOTAL: usize = 1024 * 1024;

pub(super) struct Output<R> {
    reader: BufReader<R>,
    pending: Vec<u8>,
    total: usize,
}

impl<R: AsyncRead + Unpin> Output<R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            pending: Vec::new(),
            total: 0,
        }
    }

    pub(super) async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        loop {
            let bytes = self.reader.fill_buf().await?;
            if bytes.is_empty() {
                return if self.pending.is_empty() {
                    Ok(None)
                } else {
                    self.line().map(Some)
                };
            }
            let newline = bytes.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(bytes.len(), |index| index + 1);
            if self.total + count > MAX_TOTAL || self.pending.len() + count > MAX_LINE {
                return Err(std::io::Error::other(
                    "native login output exceeds its limit",
                ));
            }
            self.pending.extend_from_slice(&bytes[..count]);
            self.total += count;
            self.reader.consume(count);
            if newline.is_some() {
                self.pending.pop();
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                return self.line().map(Some);
            }
        }
    }

    fn line(&mut self) -> std::io::Result<String> {
        String::from_utf8(std::mem::take(&mut self.pending))
            .map_err(|_| std::io::Error::other("invalid native login output encoding"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt as _;

    #[tokio::test]
    async fn partial_line_survives_cancelled_read_and_eof() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let mut output = Output::new(reader);
        writer.write_all(b"first").await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), output.next_line())
                .await
                .is_err()
        );
        writer.write_all(b" second\r\nlast").await.unwrap();
        drop(writer);
        assert_eq!(
            output.next_line().await.unwrap().as_deref(),
            Some("first second")
        );
        assert_eq!(output.next_line().await.unwrap().as_deref(), Some("last"));
        assert!(output.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn both_unterminated_lines_and_many_small_lines_are_bounded() {
        let bytes = vec![b'x'; MAX_LINE + 1];
        let mut output = Output::new(bytes.as_slice());
        assert!(output.next_line().await.is_err());
        assert!(output.pending.len() <= MAX_LINE);
        let bytes = vec![b'\n'; MAX_TOTAL + 1];
        let mut output = Output::new(bytes.as_slice());
        for _ in 0..MAX_TOTAL {
            assert_eq!(output.next_line().await.unwrap(), Some(String::new()));
        }
        assert!(output.next_line().await.is_err());
    }
}
