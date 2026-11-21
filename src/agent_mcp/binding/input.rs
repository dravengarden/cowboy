//! A blocking Tokio stdin read can outlive cancellation and stall process exit.
use std::os::unix::fs::FileTypeExt as _;
use tokio::io::AsyncRead;

pub(super) fn stdin() -> std::io::Result<Box<dyn AsyncRead + Send + Unpin>> {
    let file = std::fs::File::from(rustix::io::dup(std::io::stdin())?);
    if file.metadata()?.file_type().is_fifo() {
        return Ok(Box::new(tokio::net::unix::pipe::Receiver::from_file(file)?));
    }
    Ok(Box::new(tokio::io::stdin()))
}
