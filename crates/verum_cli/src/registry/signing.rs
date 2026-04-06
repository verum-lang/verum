// Cog signing with Ed25519 for package authenticity and integrity verification.

use super::types::CogSignature;
use crate::error::{CliError, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::path::Path;
use verum_common::Text;

/// Cog signer for cryptographic verification
pub struct CogSigner {
    pub signing_key: Option<SigningKey>,
}

impl CogSigner {
    /// Create new signer without key
    pub fn new() -> Self {
        Self { signing_key: None }
    }

    /// Load signing key from file
    pub fn load_key(&mut self, path: &Path) -> Result<()> {
        let key_bytes = std::fs::read(path)?;

        if key_bytes.len() != 32 {
            return Err(CliError::Custom(
                "Invalid signing key length (expected 32 bytes)".into(),
            ));
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&key_bytes);

        self.signing_key = Some(SigningKey::from_bytes(&key_array));
        Ok(())
    }

    /// Generate new signing key
    pub fn generate_key() -> SigningKey {
        // Generate random bytes and create signing key
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        SigningKey::from_bytes(&bytes)
    }

    /// Save signing key to file
    pub fn save_key(key: &SigningKey, path: &Path) -> Result<()> {
        let key_bytes = key.to_bytes();
        std::fs::write(path, key_bytes)?;
        Ok(())
    }

    /// Sign cog file
    pub fn sign_cog(&self, cog_path: &Path) -> Result<CogSignature> {
        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or_else(|| CliError::Custom("No signing key loaded".into()))?;

        // Read package file
        let package_bytes = std::fs::read(cog_path)?;

        // Sign the package
        let signature = signing_key.sign(&package_bytes);

        // Get public key
        let verifying_key = signing_key.verifying_key();

        Ok(CogSignature {
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            signature: hex::encode(signature.to_bytes()).into(),
            signed_at: chrono::Utc::now().timestamp(),
        })
    }

    /// Verify package signature
    pub fn verify_signature(
        cog_path: &Path,
        signature_data: &CogSignature,
    ) -> Result<bool> {
        // Read package file
        let package_bytes = std::fs::read(cog_path)?;

        // Decode public key
        let public_key_bytes = hex::decode(&signature_data.public_key)
            .map_err(|e| CliError::Custom(format!("Invalid public key hex: {}", e)))?;

        if public_key_bytes.len() != 32 {
            return Err(CliError::Custom("Invalid public key length".into()));
        }

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&public_key_bytes);

        let verifying_key = VerifyingKey::from_bytes(&key_array)
            .map_err(|e| CliError::Custom(format!("Invalid public key: {}", e)))?;

        // Decode signature
        let signature_bytes = hex::decode(&signature_data.signature)
            .map_err(|e| CliError::Custom(format!("Invalid signature hex: {}", e)))?;

        if signature_bytes.len() != 64 {
            return Err(CliError::Custom("Invalid signature length".into()));
        }

        let mut sig_array = [0u8; 64];
        sig_array.copy_from_slice(&signature_bytes);

        let signature = Signature::from_bytes(&sig_array);

        // Verify signature
        match verifying_key.verify(&package_bytes, &signature) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Get public key from signing key
    pub fn get_public_key(&self) -> Result<Text> {
        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or_else(|| CliError::Custom("No signing key loaded".into()))?;

        let verifying_key = signing_key.verifying_key();
        Ok(hex::encode(verifying_key.to_bytes()).into())
    }
}

impl Default for CogSigner {
    fn default() -> Self {
        Self::new()
    }
}
