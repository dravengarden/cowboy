//! Browser and WebView proof of possession, independent of login providers.
//! A boot nonce fences captured requests across Controller restarts. An
//! HttpOnly session remains necessary and is durably bound to the proven key.

use std::collections::HashMap;
use std::net::IpAddr;

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier as _};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};

pub const HEADER: &str = "x-cowboy-browser-proof";
pub const WS_PREFIX: &str = "cowboy-device.";
pub const CHALLENGE_PATH: &str = "/api/auth/browser/challenge";
const MAX_SKEW_MS: i64 = 90_000;
/// Browsers stamp `Date.now()` plus the offset from their challenge, so a proof
/// is ahead only by local clock movement since then, and a rejected one
/// refreshes the challenge and retries. A future stamp also extends how long
/// its nonce stays in the replay budget.
const MAX_FUTURE_SKEW_MS: i64 = 30_000;
const MAX_NONCES: usize = 65_536;
/// One network's unregistered proofs must not spend another's sign-in budget.
const MAX_ENROLLMENT_NONCES_PER_CLIENT: usize = 512;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub key: String,
    pub epoch: String,
    pub origin: String,
    pub time: i64,
    pub nonce: String,
    pub signature: String,
}

impl Proof {
    pub fn message(&self, method: &str, target: &str) -> String {
        format!(
            "cowboy-browser-proof-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.epoch, self.origin, self.key, method, target, self.time, self.nonce
        )
    }
}

pub struct BrowserDevices {
    pub epoch: String,
    nonces: parking_lot::Mutex<ReplayNonces>,
}

#[derive(Default)]
struct ReplayNonces {
    registered: HashMap<String, i64>,
    enrollment: HashMap<String, Enrollment>,
    enrollment_per_client: HashMap<IpAddr, usize>,
}

struct Enrollment {
    expires: i64,
    client: IpAddr,
}

/// An IPv6 client controls its whole /64, so one address is not one client.
fn client_bucket(client: IpAddr) -> IpAddr {
    match client.to_canonical() {
        IpAddr::V6(address) => IpAddr::V6((u128::from(address) & !u128::from(u64::MAX)).into()),
        address @ IpAddr::V4(_) => address,
    }
}

impl BrowserDevices {
    pub fn new() -> Self {
        let mut epoch = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut epoch);
        Self {
            epoch: URL_SAFE_NO_PAD.encode(epoch),
            nonces: parking_lot::Mutex::new(ReplayNonces::default()),
        }
    }

    pub fn verify(
        &self,
        encoded: &str,
        method: &str,
        target: &str,
        origins: &[String],
        now: i64,
    ) -> Result<Proof> {
        ensure!(encoded.len() <= 2048, "device proof too large");
        let proof: Proof = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded)?)?;
        ensure!(proof.epoch == self.epoch, "device proof epoch changed");
        ensure!(
            origins.contains(&proof.origin),
            "device proof origin mismatch"
        );
        ensure!(
            proof.time >= now.saturating_sub(MAX_SKEW_MS)
                && proof.time <= now.saturating_add(MAX_FUTURE_SKEW_MS),
            "expired device proof"
        );
        ensure!(
            URL_SAFE_NO_PAD.decode(&proof.nonce)?.len() == 32,
            "invalid device nonce"
        );
        let key = URL_SAFE_NO_PAD.decode(&proof.key)?;
        ensure!(key.len() == 65 && key[0] == 4, "invalid device public key");
        let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(&proof.signature)?)?;
        VerifyingKey::from_sec1_bytes(&key)?
            .verify(proof.message(method, target).as_bytes(), &signature)
            .context("device signature rejected")?;
        Ok(proof)
    }

    pub fn consume(&self, proof: &Proof, registered: bool, now: i64, client: IpAddr) -> Result<()> {
        let replay = format!("{}:{}", proof.key, proof.nonce);
        let mut guard = self.nonces.lock();
        let nonces = &mut *guard;
        nonces.registered.retain(|_, expires| *expires >= now);
        let counts = &mut nonces.enrollment_per_client;
        nonces.enrollment.retain(|_, entry| {
            let live = entry.expires >= now;
            if !live && let Some(count) = counts.get_mut(&entry.client) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&entry.client);
                }
            }
            live
        });
        ensure!(
            !nonces.registered.contains_key(&replay) && !nonces.enrollment.contains_key(&replay),
            "replayed device proof"
        );
        let expires = proof.time.saturating_add(MAX_SKEW_MS);
        // Never evict live entries: that would make a captured proof reusable.
        if registered {
            // Unregistered callers cannot fill the active devices' budget.
            ensure!(
                nonces.registered.len() < MAX_NONCES,
                "device proof capacity exhausted"
            );
            nonces.registered.insert(replay, expires);
            return Ok(());
        }
        let client = client_bucket(client);
        ensure!(
            nonces.enrollment.len() < MAX_NONCES
                && nonces
                    .enrollment_per_client
                    .get(&client)
                    .copied()
                    .unwrap_or(0)
                    < MAX_ENROLLMENT_NONCES_PER_CLIENT,
            "device proof capacity exhausted"
        );
        nonces
            .enrollment
            .insert(replay, Enrollment { expires, client });
        *nonces.enrollment_per_client.entry(client).or_insert(0) += 1;
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
#[cfg_attr(not(test), allow(unused_imports, dead_code))]
pub mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, signature::Signer as _};

    pub fn sign(key: &SigningKey, epoch: &str, method: &str, target: &str, now: i64) -> String {
        let mut nonce = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut proof = Proof {
            key: URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(false).as_bytes()),
            epoch: epoch.to_owned(),
            origin: "https://cowboy.example".to_owned(),
            time: now,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            signature: String::new(),
        };
        let signature: Signature = key.sign(proof.message(method, target).as_bytes());
        proof.signature = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&proof).unwrap())
    }

    #[test]
    fn proofs_bind_method_target_origin_time_key_and_controller_incarnation() {
        let devices = BrowserDevices::new();
        let key = SigningKey::random(&mut rand::rngs::OsRng);
        let now = 1_000_000;
        let origins = vec!["https://cowboy.example".to_owned()];
        let proof = sign(&key, &devices.epoch, "POST", "/api/sessions?a=1", now);
        assert!(
            devices
                .verify(&proof, "GET", "/api/sessions?a=1", &origins, now)
                .is_err()
        );
        assert!(
            devices
                .verify(&proof, "POST", "/api/sessions?a=2", &origins, now)
                .is_err()
        );
        assert!(
            devices
                .verify(&proof, "POST", "/api/sessions?a=1", &[], now)
                .is_err()
        );
        assert!(
            devices
                .verify(&proof, "POST", "/api/sessions?a=1", &origins, now + 90_001)
                .is_err()
        );
        assert!(
            BrowserDevices::new()
                .verify(&proof, "POST", "/api/sessions?a=1", &origins, now)
                .is_err()
        );
        let verified = devices
            .verify(&proof, "POST", "/api/sessions?a=1", &origins, now)
            .unwrap();
        let client: IpAddr = "192.0.2.1".parse().unwrap();
        assert!(devices.consume(&verified, false, now, client).is_ok());
        assert!(devices.consume(&verified, true, now, client).is_err());
    }

    fn proof_at(devices: &BrowserDevices, key: &SigningKey, now: i64) -> Proof {
        devices
            .verify(
                &sign(key, &devices.epoch, "GET", "/api/private", now),
                "GET",
                "/api/private",
                &["https://cowboy.example".to_owned()],
                now,
            )
            .unwrap()
    }

    #[test]
    fn one_clients_unregistered_proofs_cannot_lock_out_other_sign_ins() {
        let devices = BrowserDevices::new();
        let key = SigningKey::random(&mut rand::rngs::OsRng);
        let now = 1_000_000;
        let attacker: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..MAX_ENROLLMENT_NONCES_PER_CLIENT {
            let proof = proof_at(&devices, &key, now);
            devices.consume(&proof, false, now, attacker).unwrap();
        }
        let proof = proof_at(&devices, &key, now);
        assert!(devices.consume(&proof, false, now, attacker).is_err());
        // A registered device and every other client keep their budgets.
        assert!(devices.consume(&proof, true, now, attacker).is_ok());
        let proof = proof_at(&devices, &key, now);
        let owner: IpAddr = "198.51.100.7".parse().unwrap();
        assert!(devices.consume(&proof, false, now, owner).is_ok());
        // The budget returns when the proofs expire, including its counter.
        let later = now + MAX_SKEW_MS + 1;
        let proof = proof_at(&devices, &key, later);
        assert!(devices.consume(&proof, false, later, attacker).is_ok());
        assert_eq!(devices.nonces.lock().enrollment_per_client.len(), 1);
    }

    #[test]
    fn an_ipv6_client_shares_one_budget_across_its_prefix() {
        let first: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let second: IpAddr = "2001:db8:1:2:bbbb::2".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(client_bucket(first), client_bucket(second));
        assert_ne!(client_bucket(first), client_bucket(other));
        let mapped: IpAddr = "::ffff:192.0.2.1".parse().unwrap();
        assert_eq!(
            client_bucket(mapped),
            "192.0.2.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn a_proof_stamped_far_ahead_is_refused_but_ordinary_skew_is_not() {
        let devices = BrowserDevices::new();
        let key = SigningKey::random(&mut rand::rngs::OsRng);
        let origins = vec!["https://cowboy.example".to_owned()];
        let now = 1_000_000;
        let verify = |time| {
            devices.verify(
                &sign(&key, &devices.epoch, "GET", "/x", time),
                "GET",
                "/x",
                &origins,
                now,
            )
        };
        assert!(verify(now + MAX_FUTURE_SKEW_MS).is_ok());
        assert!(verify(now + MAX_FUTURE_SKEW_MS + 1).is_err());
        assert!(verify(now - MAX_SKEW_MS).is_ok());
        assert!(verify(now - MAX_SKEW_MS - 1).is_err());
    }
}
