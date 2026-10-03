//! Browser and WebView proof of possession, independent of login providers.
//! A boot nonce fences captured requests across Controller restarts. An
//! HttpOnly session remains necessary and is durably bound to the proven key.

use std::collections::HashMap;

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier as _};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};

pub(crate) const HEADER: &str = "x-cowboy-browser-proof";
pub(crate) const WS_PREFIX: &str = "cowboy-device.";
pub(crate) const CHALLENGE_PATH: &str = "/api/auth/browser/challenge";
const MAX_SKEW_MS: i64 = 90_000;
const MAX_NONCES: usize = 65_536;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proof {
    pub key: String,
    pub epoch: String,
    pub origin: String,
    pub time: i64,
    pub nonce: String,
    pub signature: String,
}

impl Proof {
    pub(crate) fn message(&self, method: &str, target: &str) -> String {
        format!(
            "cowboy-browser-proof-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.epoch, self.origin, self.key, method, target, self.time, self.nonce
        )
    }
}

pub(crate) struct BrowserDevices {
    pub epoch: String,
    nonces: parking_lot::Mutex<ReplayNonces>,
}

#[derive(Default)]
struct ReplayNonces {
    registered: HashMap<String, i64>,
    enrollment: HashMap<String, i64>,
}

impl BrowserDevices {
    pub(crate) fn new() -> Self {
        let mut epoch = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut epoch);
        Self {
            epoch: URL_SAFE_NO_PAD.encode(epoch),
            nonces: parking_lot::Mutex::new(ReplayNonces::default()),
        }
    }

    pub(crate) fn verify(
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
            proof.time.abs_diff(now) <= MAX_SKEW_MS as u64,
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

    pub(crate) fn consume(&self, proof: &Proof, registered: bool, now: i64) -> Result<()> {
        let replay = format!("{}:{}", proof.key, proof.nonce);
        let mut nonces = self.nonces.lock();
        nonces.registered.retain(|_, expires| *expires >= now);
        nonces.enrollment.retain(|_, expires| *expires >= now);
        ensure!(
            !nonces.registered.contains_key(&replay) && !nonces.enrollment.contains_key(&replay),
            "replayed device proof"
        );
        // Unregistered callers must not fill the active devices' replay budget.
        let (entries, limit) = if registered {
            (&mut nonces.registered, MAX_NONCES)
        } else {
            (&mut nonces.enrollment, 1024)
        };
        // Never evict live entries: that would make a captured proof reusable.
        ensure!(entries.len() < limit, "device proof capacity exhausted");
        entries.insert(replay, proof.time.saturating_add(MAX_SKEW_MS));
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, signature::Signer as _};

    pub(crate) fn sign(
        key: &SigningKey,
        epoch: &str,
        method: &str,
        target: &str,
        now: i64,
    ) -> String {
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
        assert!(devices.consume(&verified, false, now).is_ok());
        assert!(devices.consume(&verified, true, now).is_err());
        for i in 0..1024 {
            devices
                .nonces
                .lock()
                .enrollment
                .insert(format!("unknown:{i}"), now + MAX_SKEW_MS);
        }
        let second = devices
            .verify(
                &sign(&key, &devices.epoch, "GET", "/api/private", now),
                "GET",
                "/api/private",
                &origins,
                now,
            )
            .unwrap();
        assert!(devices.consume(&second, false, now).is_err());
        assert!(devices.consume(&second, true, now).is_ok());
    }
}
