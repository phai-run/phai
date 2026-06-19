//! Self-contained, passphrase-encrypted activation invites (ADR-0034).
//!
//! An invite packs the BigQuery coordinates and a service-account key into a
//! single `PHAI1E-<base64url>` string. The payload is sealed with a passphrase
//! the owner chooses and shares out of band: Argon2id derives a key, and
//! XChaCha20-Poly1305 encrypts the JSON payload. The blob therefore *is* a
//! credential — treat it as a secret. See [`crate::invite::seal`] /
//! [`crate::invite::open`].

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::XChaCha20Poly1305;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Human-readable prefix marking an encrypted invite. The `E` distinguishes the
/// encrypted format; a future plaintext variant (not used today) would differ.
const TOKEN_PREFIX: &str = "PHAI1E-";

/// Binary envelope magic — guards against feeding unrelated base64 to [`open`].
const ENVELOPE_MAGIC: &[u8; 4] = b"PHV1";

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

/// The decrypted contents of an invite. `service_account` is the raw Google
/// service-account JSON, kept as an opaque value so phai-core does not need to
/// model Google's key schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Invite {
    /// Envelope/payload schema version. Currently always `1`.
    pub v: u32,
    pub project_id: String,
    pub dataset_id: String,
    /// Audit actor for the activated machine (e.g. `esposa`).
    pub actor_id: String,
    /// Capability hint for UX. `rw` lets the holder recategorize; the real
    /// authorization is whatever the embedded service account is granted in GCP.
    pub role: String,
    /// Raw Google service-account JSON key.
    pub service_account: serde_json::Value,
    /// Friendly label for the target device, surfaced during activation.
    pub label: String,
}

impl Invite {
    /// Build an invite with `v` pinned to the current schema version.
    pub fn new(
        project_id: impl Into<String>,
        dataset_id: impl Into<String>,
        actor_id: impl Into<String>,
        role: impl Into<String>,
        service_account: serde_json::Value,
        label: impl Into<String>,
    ) -> Self {
        Self {
            v: 1,
            project_id: project_id.into(),
            dataset_id: dataset_id.into(),
            actor_id: actor_id.into(),
            role: role.into(),
            service_account,
            label: label.into(),
        }
    }
}

/// Argon2id cost parameters stored in the envelope so [`open`] reproduces the
/// exact KDF without out-of-band agreement.
#[derive(Debug, Clone, Copy)]
struct KdfParams {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
}

impl KdfParams {
    fn default_params() -> Self {
        let p = argon2::Params::DEFAULT;
        Self {
            m_cost: p.m_cost(),
            t_cost: p.t_cost(),
            p_cost: p.p_cost(),
        }
    }

    fn derive(&self, passphrase: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
        let params = argon2::Params::new(self.m_cost, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(|e| anyhow::anyhow!("parâmetros Argon2 inválidos: {e}"))?;
        let argon =
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        argon
            .hash_password_into(passphrase, salt, key.as_mut_slice())
            .map_err(|e| anyhow::anyhow!("derivação Argon2 falhou: {e}"))?;
        Ok(key)
    }
}

/// Seal an invite into a `PHAI1E-…` token using `passphrase`.
pub fn seal(invite: &Invite, passphrase: &str) -> Result<String> {
    if passphrase.is_empty() {
        bail!("a senha do convite não pode ser vazia");
    }
    let plaintext = serde_json::to_vec(invite).context("serializar convite")?;
    let kdf = KdfParams::default_params();

    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let key = kdf.derive(passphrase.as_bytes(), &salt)?;

    let cipher = XChaCha20Poly1305::new(key.as_slice().into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_slice())
        .map_err(|_| anyhow::anyhow!("falha ao cifrar convite"))?;

    let mut envelope =
        Vec::with_capacity(ENVELOPE_MAGIC.len() + 12 + SALT_LEN + NONCE_LEN + ciphertext.len());
    envelope.extend_from_slice(ENVELOPE_MAGIC);
    envelope.extend_from_slice(&kdf.m_cost.to_le_bytes());
    envelope.extend_from_slice(&kdf.t_cost.to_le_bytes());
    envelope.extend_from_slice(&kdf.p_cost.to_le_bytes());
    envelope.extend_from_slice(&salt);
    envelope.extend_from_slice(nonce.as_slice());
    envelope.extend_from_slice(&ciphertext);

    Ok(format!(
        "{TOKEN_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(envelope)
    ))
}

/// Open a `PHAI1E-…` token with `passphrase`. Fails on a wrong passphrase, a
/// tampered blob, or a malformed token — the three are intentionally
/// indistinguishable to the caller.
pub fn open(token: &str, passphrase: &str) -> Result<Invite> {
    let body = token
        .strip_prefix(TOKEN_PREFIX)
        .context("convite não reconhecido (prefixo PHAI1E- ausente)")?;
    let envelope = URL_SAFE_NO_PAD
        .decode(body.trim())
        .context("convite corrompido (base64 inválido)")?;

    let header_len = ENVELOPE_MAGIC.len() + 12 + SALT_LEN + NONCE_LEN;
    if envelope.len() <= header_len {
        bail!("convite corrompido (tamanho insuficiente)");
    }
    let (magic, rest) = envelope.split_at(ENVELOPE_MAGIC.len());
    if magic != ENVELOPE_MAGIC {
        bail!("convite não reconhecido (assinatura inválida)");
    }
    let (m_cost, rest) = take_u32(rest);
    let (t_cost, rest) = take_u32(rest);
    let (p_cost, rest) = take_u32(rest);
    let (salt, rest) = rest.split_at(SALT_LEN);
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);

    let kdf = KdfParams {
        m_cost,
        t_cost,
        p_cost,
    };
    let key = kdf.derive(passphrase.as_bytes(), salt)?;
    let cipher = XChaCha20Poly1305::new(key.as_slice().into());
    let plaintext = cipher
        .decrypt(nonce.into(), ciphertext)
        .map_err(|_| anyhow::anyhow!("senha incorreta ou convite adulterado"))?;

    let invite: Invite =
        serde_json::from_slice(&plaintext).context("convite decifrado, mas conteúdo inválido")?;
    if invite.v != 1 {
        bail!("versão de convite não suportada: {}", invite.v);
    }
    Ok(invite)
}

/// Read a little-endian `u32` off the front of `bytes`, returning the rest.
/// The caller guarantees `bytes` holds at least 4 bytes via the length check.
fn take_u32(bytes: &[u8]) -> (u32, &[u8]) {
    let (head, tail) = bytes.split_at(4);
    let value = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    (value, tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Invite {
        Invite::new(
            "gcp-proj",
            "phai",
            "esposa",
            "rw",
            json!({"type": "service_account", "private_key": "FAKE", "client_email": "x@y.iam"}),
            "MacBook Esposa",
        )
    }

    #[test]
    fn round_trips_through_seal_and_open() {
        let token = seal(&sample(), "correct horse battery").unwrap();
        assert!(token.starts_with("PHAI1E-"));
        let opened = open(&token, "correct horse battery").unwrap();
        assert_eq!(opened, sample());
    }

    #[test]
    fn each_seal_is_unique_thanks_to_random_salt_and_nonce() {
        let a = seal(&sample(), "pw").unwrap();
        let b = seal(&sample(), "pw").unwrap();
        assert_ne!(a, b, "salt/nonce must randomize the ciphertext");
    }

    #[test]
    fn wrong_passphrase_fails() {
        let token = seal(&sample(), "right").unwrap();
        assert!(open(&token, "wrong").is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let token = seal(&sample(), "pw").unwrap();
        let body = token.strip_prefix("PHAI1E-").unwrap();
        let mut bytes = URL_SAFE_NO_PAD.decode(body).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let tampered = format!("PHAI1E-{}", URL_SAFE_NO_PAD.encode(bytes));
        assert!(open(&tampered, "pw").is_err());
    }

    #[test]
    fn rejects_foreign_or_malformed_tokens() {
        assert!(open("not-an-invite", "pw").is_err());
        assert!(open("PHAI1E-!!!notbase64!!!", "pw").is_err());
        assert!(open("PHAI1E-QUJD", "pw").is_err()); // valid base64, too short
    }

    #[test]
    fn empty_passphrase_is_rejected_on_seal() {
        assert!(seal(&sample(), "").is_err());
    }
}
