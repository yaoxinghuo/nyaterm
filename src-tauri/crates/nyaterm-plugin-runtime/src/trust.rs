//! Repository keys are pinned in the client. Catalogs cannot extend this trust store.
use crate::{Result, invalid};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const OFFICIAL_PLUGIN_KEYS: &[(&str, &str)] = &[];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSignature {
    pub algorithm: String,
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SignatureStatus {
    #[default]
    Unsigned,
    Untrusted {
        #[serde(rename = "keyId")]
        key_id: String,
    },
    Verified {
        #[serde(rename = "keyId")]
        key_id: String,
    },
}

pub fn verify(bytes: &[u8], signature: &PackageSignature, keys: &[(&str, &str)]) -> Result<()> {
    if signature.algorithm != "ed25519" {
        return Err(invalid("Unsupported plugin signature algorithm"));
    }
    let public = keys
        .iter()
        .find(|(id, _)| *id == signature.key_id)
        .ok_or_else(|| invalid("Plugin signing key is not trusted"))?
        .1;
    let public: [u8; 32] = STANDARD
        .decode(public)
        .map_err(|_| invalid("Invalid trusted public key"))?
        .try_into()
        .map_err(|_| invalid("Invalid trusted public key length"))?;
    let key =
        VerifyingKey::from_bytes(&public).map_err(|_| invalid("Invalid trusted public key"))?;
    let raw = STANDARD
        .decode(&signature.signature)
        .map_err(|_| invalid("Invalid plugin signature encoding"))?;
    let signature =
        Signature::from_slice(&raw).map_err(|_| invalid("Invalid plugin signature length"))?;
    key.verify_strict(bytes, &signature)
        .map_err(|_| invalid("Plugin signature verification failed"))
}

pub fn inspect(root: &Path, keys: &[(&str, &str)]) -> Result<SignatureStatus> {
    let path = root.join("signature.json");
    if !path.exists() {
        return Ok(SignatureStatus::Unsigned);
    }
    let signature: PackageSignature = serde_json::from_slice(&std::fs::read(path)?)?;
    if signature.algorithm != "ed25519"
        || signature.key_id.is_empty()
        || signature.key_id.len() > 128
    {
        return Err(invalid("Invalid plugin signature metadata"));
    }
    let raw = STANDARD
        .decode(&signature.signature)
        .map_err(|_| invalid("Invalid plugin signature encoding"))?;
    if raw.len() != 64 {
        return Err(invalid("Invalid plugin signature length"));
    }
    if !keys.iter().any(|(id, _)| *id == signature.key_id) {
        return Ok(SignatureStatus::Untrusted {
            key_id: signature.key_id,
        });
    }
    verify(
        &std::fs::read(root.join("checksums.json"))?,
        &signature,
        keys,
    )?;
    Ok(SignatureStatus::Verified {
        key_id: signature.key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn verifies_exact_bytes_and_rejects_wrong_key_algorithm_and_signature() {
        let key = SigningKey::from_bytes(&[42; 32]);
        let public = STANDARD.encode(key.verifying_key().to_bytes());
        let keys = [("test", public.as_str())];
        let mut signature = PackageSignature {
            algorithm: "ed25519".into(),
            key_id: "test".into(),
            signature: STANDARD.encode(key.sign(b"checksums").to_bytes()),
        };
        verify(b"checksums", &signature, &keys).unwrap();
        assert!(verify(b"checksums\n", &signature, &keys).is_err());
        assert!(verify(b"checksums", &signature, &[]).is_err());
        signature.algorithm = "rsa".into();
        assert!(verify(b"checksums", &signature, &keys).is_err());
        signature.algorithm = "ed25519".into();
        signature.signature = STANDARD.encode([0; 64]);
        assert!(verify(b"checksums", &signature, &keys).is_err());
    }
}
