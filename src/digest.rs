//! Leaf content digests shared by authentication, stores and grants. It names
//! no other module, so any caller can depend on it without joining a cycle.

/// Lower-case hexadecimal SHA-256 of `value`.
pub fn hex_sha256(value: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(value)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}
