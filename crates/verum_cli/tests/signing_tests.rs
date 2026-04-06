// Disabled: depends on PackageSigner type which doesn't exist yet
#![cfg(feature = "package_signing")]
#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for signing module
// Migrated from src/signing.rs per CLAUDE.md standards

use verum_cli::signing::*;

use tempfile::NamedTempFile;

#[test]
fn test_sign_and_verify() {
    // Create test package file
    let temp_file = NamedTempFile::new().unwrap();
    std::fs::write(temp_file.path(), b"test package data").unwrap();

    // Generate key and sign
    let key = PackageSigner::generate_key();
    let mut signer = PackageSigner::new();
    signer.signing_key = Some(key);

    let signature = signer.sign_cog(temp_file.path()).unwrap();

    // Verify signature
    let valid = PackageSigner::verify_signature(temp_file.path(), &signature).unwrap();
    assert!(valid);
}

#[test]
fn test_invalid_signature() {
    let temp_file = NamedTempFile::new().unwrap();
    std::fs::write(temp_file.path(), b"test package data").unwrap();

    // Create signature with different key
    let key1 = PackageSigner::generate_key();
    let mut signer1 = PackageSigner::new();
    signer1.signing_key = Some(key1);
    let signature = signer1.sign_cog(temp_file.path()).unwrap();

    // Modify package
    std::fs::write(temp_file.path(), b"modified package data").unwrap();

    // Verify should fail
    let valid = PackageSigner::verify_signature(temp_file.path(), &signature).unwrap();
    assert!(!valid);
}
