use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct Cipher(Aes256Gcm);

impl Cipher {
    pub fn from_base64(key: &str) -> Result<Self> {
        let raw = STANDARD
            .decode(key.trim())
            .context("STYK_SECRET_KEY is not base64")?;
        if raw.len() != 32 {
            bail!("STYK_SECRET_KEY must decode to 32 bytes (openssl rand -base64 32)");
        }
        Ok(Self(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw))))
    }

    /// Returns nonce || ciphertext.
    pub fn encrypt(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        out.extend(
            self.0
                .encrypt(&nonce, plain)
                .map_err(|_| anyhow!("encrypt failed"))?,
        );
        Ok(out)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            bail!("ciphertext too short");
        }
        let (nonce, ct) = data.split_at(12);
        self.0
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| anyhow!("decrypt failed (wrong STYK_SECRET_KEY?)"))
    }
}

pub fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Constant-time string comparison for secrets.
pub fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Cipher::from_base64(&STANDARD.encode([7u8; 32])).unwrap();
        let enc = c.encrypt(b"JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(c.decrypt(&enc).unwrap(), b"JBSWY3DPEHPK3PXP");
        assert!(Cipher::from_base64(&STANDARD.encode([1u8; 16])).is_err());
    }
}
