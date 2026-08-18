//! Product-account password hashing, username rules, and cookie helpers.

use std::collections::HashMap;
use std::io::Read as _;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context as _, Result};
use argon2::Argon2;
use axum::http::{HeaderMap, Uri, header};
use password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

/// Product login cookie. Distinct from `cowboy_admin`.
pub const USER_SESSION_COOKIE: &str = "cowboy_user";
/// Absolute cookie / `user_sessions` TTL.
pub const USER_SESSION_TTL_SECS: i64 = 14 * 86_400;
pub const USER_SESSION_TTL_MS: i64 = USER_SESSION_TTL_SECS * 1_000;

/// Stored in `users.password_algo`. The PHC string is the only hash material.
pub const PASSWORD_ALGO_ARGON2ID: &str = "argon2id";

const USER_ID_HEX_LEN: usize = 32;

/// Trim, lowercase, and accept 1–64 of `[a-z0-9._-]`.
///
/// # Errors
/// Returns when the handle is empty, longer than 64, or uses other characters.
pub fn normalize_username(username: &str) -> Result<String> {
    let username = username.trim().to_ascii_lowercase();
    anyhow::ensure!(
        (1..=64).contains(&username.len())
            && username.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'.'
                    || byte == b'_'
                    || byte == b'-'
            }),
        "username must be 1-64 lowercase letters, digits, '.', '_' or '-'"
    );
    Ok(username)
}

/// Product passwords are 10–128 characters and must not equal the username.
///
/// # Errors
/// Returns when the password is too short, too long, or matches the username.
pub fn ensure_password(password: &str, username: &str) -> Result<()> {
    anyhow::ensure!(
        (10..=128).contains(&password.len()),
        "password must be 10-128 characters"
    );
    anyhow::ensure!(password != username, "password cannot be the username");
    Ok(())
}

/// 32-hex identifier from `/dev/urandom`.
///
/// # Errors
/// Returns when the OS random device cannot be read.
pub fn new_user_id() -> Result<String> {
    hex_from_urandom(USER_ID_HEX_LEN / 2)
}

/// Hash `password` to an `argon2id` PHC string. Salt lives inside the PHC.
///
/// # Errors
/// Returns when randomness cannot be read or argon2id hashing fails.
pub fn hash_password(password: &str) -> Result<String> {
    let salt_bytes = random_bytes(16)?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| anyhow::anyhow!("encoding argon2 salt: {error}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow::anyhow!("hashing password: {error}"))
}

/// Verify `password` against a stored PHC string.
#[must_use]
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Run argon2id verify against a process-lifetime dummy hash.
///
/// Unknown-user login calls this so the 401 is not enumerable by hash timing.
#[must_use]
pub fn verify_unknown_user_password(password: &str) -> bool {
    verify_password(password, dummy_password_hash())
}

/// 32-byte hex cookie secret (64 hex chars).
///
/// # Errors
/// Returns when the OS random device cannot be read.
pub fn new_session_token() -> Result<String> {
    hex_from_urandom(32)
}

#[must_use]
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{name}=");
    cookie.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(&prefix).map(ToOwned::to_owned)
    })
}

#[must_use]
pub fn user_cookie_token(headers: &HeaderMap) -> Option<String> {
    cookie_value(headers, USER_SESSION_COOKIE)
}

#[must_use]
pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{USER_SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={USER_SESSION_TTL_SECS}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn clear_session_cookie(secure: bool) -> String {
    let mut cookie = format!("{USER_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[must_use]
pub fn request_is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .unwrap_or(value)
                .trim()
                .eq_ignore_ascii_case("https")
        })
}

/// Last `X-Forwarded-For` hop when the peer is loopback; otherwise the peer.
#[must_use]
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    if peer.ip().is_loopback()
        && let Some(xff) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        && let Some(last) = xff.split(',').map(str::trim).rfind(|part| !part.is_empty())
        && let Ok(ip) = last.parse()
    {
        return ip;
    }
    peer.ip()
}

/// Same-origin allow-list for cookie POST/PUT/DELETE (and later cookie `/ws`).
#[must_use]
pub fn origin_allowed(headers: &HeaderMap, peer: SocketAddr, public_origins: &[String]) -> bool {
    let Some(candidate) = request_candidate_origin(headers) else {
        return false;
    };
    allowed_origins(headers, peer, public_origins)
        .iter()
        .any(|allowed| allowed == &candidate)
}

#[must_use]
pub fn load_public_origins() -> Vec<String> {
    std::env::var("COWBOY_PUBLIC_ORIGIN")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| normalize_origin(part.trim()))
                .collect()
        })
        .unwrap_or_default()
}

fn request_candidate_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_origin)
        .or_else(|| {
            headers
                .get(header::REFERER)
                .and_then(|value| value.to_str().ok())
                .and_then(normalize_origin)
        })
}

fn allowed_origins(
    headers: &HeaderMap,
    peer: SocketAddr,
    public_origins: &[String],
) -> Vec<String> {
    let https = request_is_https(headers);
    let mut origins = Vec::new();
    if let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|host| !host.is_empty())
    {
        let scheme = if https { "https" } else { "http" };
        if let Some(origin) = normalize_origin(&format!("{scheme}://{host}")) {
            origins.push(origin);
        }
    }
    if peer.ip().is_loopback()
        && let Some(forwarded_host) = first_forwarded_host(headers)
        && let Some(origin) = normalize_origin(&format!("https://{forwarded_host}"))
    {
        origins.push(origin);
    }
    origins.extend(public_origins.iter().cloned());
    // Vite Origins are a debug exception only. COWBOY_DEV_CSRF is ignored.
    #[cfg(debug_assertions)]
    {
        origins.push("http://localhost:5173".to_owned());
        origins.push("http://127.0.0.1:5173".to_owned());
    }
    origins
}

fn first_forwarded_host(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-host")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .find(|part| !part.is_empty())
        })
        .map(ToOwned::to_owned)
}

fn normalize_origin(value: &str) -> Option<String> {
    let uri = value.parse::<Uri>().ok()?;
    let scheme = uri.scheme_str()?.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = uri.authority()?;
    let host = authority.host().to_ascii_lowercase();
    Some(match authority.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

/// In-process failure map. Reset on success. Delay after 5 failures.
#[derive(Debug, Default)]
pub struct AuthRateLimiter {
    failures: parking_lot::Mutex<HashMap<(String, String), u32>>,
}

impl AuthRateLimiter {
    #[must_use]
    pub fn delay(&self, username: &str, ip: &str) -> Option<Duration> {
        let count = self
            .failures
            .lock()
            .get(&(username.to_owned(), ip.to_owned()))
            .copied()
            .unwrap_or(0);
        if count < 5 {
            return None;
        }
        let shift = (count - 5).min(4);
        Some(Duration::from_secs(1_u64 << shift))
    }

    pub fn record_failure(&self, username: &str, ip: &str) {
        let mut failures = self.failures.lock();
        *failures
            .entry((username.to_owned(), ip.to_owned()))
            .or_insert(0) += 1;
    }

    pub fn reset(&self, username: &str, ip: &str) {
        self.failures
            .lock()
            .remove(&(username.to_owned(), ip.to_owned()));
    }
}

fn dummy_password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let secret = hex_from_urandom(32).expect("reading dummy password secret");
        hash_password(&secret).expect("hashing dummy unknown-user password")
    })
}

fn hex_from_urandom(byte_len: usize) -> Result<String> {
    let bytes = random_bytes(byte_len)?;
    Ok(bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    ))
}

fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0_u8; len];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut bytes)
        .context("reading OS randomness")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, SocketAddr};
    use std::time::Duration;

    use axum::http::HeaderMap;

    #[test]
    fn normalize_username_trims_and_lowercases() {
        assert_eq!(normalize_username(" Draven._-1 ").unwrap(), "draven._-1");
    }

    #[test]
    fn normalize_username_rejects_empty_long_and_invalid() {
        assert!(normalize_username("").is_err());
        assert!(normalize_username("   ").is_err());
        assert!(normalize_username(&"a".repeat(65)).is_err());
        assert!(normalize_username("Draven!").is_err());
        assert!(normalize_username("draven@x").is_err());
    }

    #[test]
    fn ensure_password_rejects_short_and_username() {
        assert!(ensure_password("short", "draven").is_err());
        assert!(ensure_password("draven", "draven").is_err());
        assert!(ensure_password("long-enough-password", "draven").is_ok());
    }

    #[test]
    fn hash_password_is_argon2id_phc_and_round_trips() {
        let hash = hash_password("long-enough-password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("long-enough-password", &hash));
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn unknown_user_dummy_verify_does_not_accept_caller_password() {
        assert!(!verify_unknown_user_password("long-enough-password"));
        assert!(!verify_unknown_user_password("another-password"));
    }

    #[test]
    fn new_user_id_is_32_hex() {
        let id = new_user_id().unwrap();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(id, new_user_id().unwrap());
    }

    fn loopback() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 3333))
    }

    fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    #[test]
    fn last_xff_hop_is_used_only_from_loopback_peer() {
        let headers = header_map(&[("x-forwarded-for", "1.2.3.4, 127.0.0.1")]);
        assert_eq!(
            client_ip(&headers, loopback()),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_ip(&headers, SocketAddr::from(([10, 0, 0, 8], 3333))),
            "10.0.0.8".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn origin_allow_list_accepts_host_and_rejects_evil() {
        let headers = header_map(&[
            ("host", "cowboy.example"),
            ("x-forwarded-proto", "https"),
            ("origin", "https://cowboy.example"),
        ]);
        assert!(origin_allowed(&headers, loopback(), &[]));
        let evil = header_map(&[
            ("host", "cowboy.example"),
            ("x-forwarded-proto", "https"),
            ("origin", "https://evil.example"),
        ]);
        assert!(!origin_allowed(&evil, loopback(), &[]));
        let missing = header_map(&[("host", "cowboy.example")]);
        assert!(!origin_allowed(&missing, loopback(), &[]));
    }

    #[test]
    #[cfg(debug_assertions)]
    fn vite_origin_is_allowed_only_in_debug() {
        let headers = header_map(&[
            ("host", "127.0.0.1:3333"),
            ("origin", "http://localhost:5173"),
        ]);
        assert!(origin_allowed(&headers, loopback(), &[]));
    }

    #[test]
    fn rate_limiter_delays_after_five_failures_and_resets() {
        let limiter = AuthRateLimiter::default();
        for _ in 0..5 {
            limiter.record_failure("draven", "127.0.0.1");
        }
        assert_eq!(
            limiter.delay("draven", "127.0.0.1"),
            Some(Duration::from_secs(1))
        );
        limiter.record_failure("draven", "127.0.0.1");
        assert_eq!(
            limiter.delay("draven", "127.0.0.1"),
            Some(Duration::from_secs(2))
        );
        limiter.reset("draven", "127.0.0.1");
        assert_eq!(limiter.delay("draven", "127.0.0.1"), None);
    }
}
