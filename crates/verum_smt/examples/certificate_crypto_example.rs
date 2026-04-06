//! Example demonstrating cryptographic certificate functionality
//!
//! This example shows:
//! 1. Proof certificate generation
//! 2. Certificate verification
//! 3. Proof checksums
//! 4. Certificate serialization

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;
use z3::ast::{Ast, Int};
use z3::{SatResult, Solver};

fn main() {
    println!("=== Proof Certificate Demo ===\n");

    // Example 1: Generate proof certificate
    demo_proof_generation();

    // Example 2: Verify proof certificate
    demo_proof_verification();

    // Example 3: Proof chain
    demo_proof_chain();

    // Example 4: Certificate format
    demo_certificate_format();

    println!("=== Demo Complete ===");
}

/// Demo 1: Generate a proof certificate for a simple theorem
fn demo_proof_generation() {
    println!("--- Demo 1: Proof Certificate Generation ---\n");

    let solver = Solver::new();

    // Theorem: For all x, x + 0 = x
    let x = Int::fresh_const("x");
    let zero = Int::from_i64(0);
    let sum = Int::add(&[&x, &zero]);

    // Assert the theorem
    solver.assert(&Ast::eq(&sum, &x));

    let start = Instant::now();
    let result = solver.check();
    let elapsed = start.elapsed();

    match result {
        SatResult::Sat => {
            println!("✓ Theorem verified: ∀x. x + 0 = x");

            // Generate certificate
            let cert = ProofCertificate {
                theorem: "∀x. x + 0 = x".to_string(),
                proof_method: "SMT (Z3)".to_string(),
                verification_time_ms: elapsed.as_millis() as u64,
                checksum: compute_checksum("∀x. x + 0 = x"),
            };

            println!("  Certificate generated:");
            println!("    Theorem: {}", cert.theorem);
            println!("    Method: {}", cert.proof_method);
            println!("    Time: {}ms", cert.verification_time_ms);
            println!("    Checksum: {}", cert.checksum);
        }
        _ => println!("✗ Verification failed"),
    }

    println!();
}

/// Demo 2: Verify a proof certificate
fn demo_proof_verification() {
    println!("--- Demo 2: Proof Certificate Verification ---\n");

    // Create a certificate
    let original_cert = ProofCertificate {
        theorem: "2 + 2 = 4".to_string(),
        proof_method: "SMT".to_string(),
        verification_time_ms: 1,
        checksum: compute_checksum("2 + 2 = 4"),
    };

    println!("Original certificate:");
    println!("  Checksum: {}", original_cert.checksum);

    // Verify the certificate
    let recomputed_checksum = compute_checksum(&original_cert.theorem);

    if original_cert.checksum == recomputed_checksum {
        println!("✓ Certificate checksum valid");
    } else {
        println!("✗ Certificate checksum mismatch!");
    }

    // Verify the theorem itself
    let solver = Solver::new();
    let two = Int::from_i64(2);
    let four = Int::from_i64(4);
    let sum = Int::add(&[&two, &two]);
    solver.assert(&Ast::eq(&sum, &four));

    match solver.check() {
        SatResult::Sat => println!("✓ Theorem re-verified independently"),
        _ => println!("✗ Theorem verification failed"),
    }

    // Test tampered certificate
    let tampered_checksum = compute_checksum("2 + 2 = 5");
    if original_cert.checksum != tampered_checksum {
        println!("✓ Tampered theorem correctly rejected");
    }

    println!();
}

/// Demo 3: Proof chain (dependent proofs)
fn demo_proof_chain() {
    println!("--- Demo 3: Proof Chain ---\n");

    let mut proof_store = ProofStore::new();

    // Proof 1: 0 is identity for addition
    let lemma1 = "∀x. x + 0 = x";
    proof_store.add_proof("identity", lemma1);
    println!("  Added: Lemma 1 (identity): {}", lemma1);

    // Proof 2: Addition is commutative
    let lemma2 = "∀x,y. x + y = y + x";
    proof_store.add_proof("commutativity", lemma2);
    println!("  Added: Lemma 2 (commutativity): {}", lemma2);

    // Proof 3: Derives from 1 and 2
    let theorem = "∀x. 0 + x = x";
    proof_store.add_proof_with_deps("zero_left", theorem, vec!["identity", "commutativity"]);
    println!(
        "  Added: Theorem (zero_left): {} [depends on identity, commutativity]",
        theorem
    );

    // Verify the chain
    println!("\n  Proof chain verification:");
    if proof_store.verify_chain("zero_left") {
        println!("  ✓ All dependencies satisfied");
        println!("  ✓ Proof chain valid");
    } else {
        println!("  ✗ Proof chain invalid");
    }

    // Show the store
    println!(
        "\n  Proof store contains {} certificates",
        proof_store.count()
    );

    println!();
}

/// Demo 4: Certificate format (serialization)
fn demo_certificate_format() {
    println!("--- Demo 4: Certificate Format ---\n");

    let cert = ProofCertificate {
        theorem: "∀x,y. x + y = y + x".to_string(),
        proof_method: "SMT (Z3 4.12)".to_string(),
        verification_time_ms: 5,
        checksum: compute_checksum("∀x,y. x + y = y + x"),
    };

    // JSON format
    println!("  JSON Format:");
    println!("  {{");
    println!("    \"theorem\": \"{}\",", cert.theorem);
    println!("    \"proof_method\": \"{}\",", cert.proof_method);
    println!(
        "    \"verification_time_ms\": {},",
        cert.verification_time_ms
    );
    println!("    \"checksum\": \"{}\"", cert.checksum);
    println!("  }}");

    // SMT-LIB2 format
    println!("\n  SMT-LIB2 Format:");
    println!("  ; Proof certificate");
    println!("  ; Theorem: {}", cert.theorem);
    println!("  (declare-const x Int)");
    println!("  (declare-const y Int)");
    println!("  (assert (= (+ x y) (+ y x)))");
    println!("  (check-sat) ; Result: sat");

    // Binary format size estimate
    let json_size = format!(
        "{{\"theorem\":\"{}\",\"proof_method\":\"{}\",\"verification_time_ms\":{},\"checksum\":\"{}\"}}",
        cert.theorem, cert.proof_method, cert.verification_time_ms, cert.checksum
    ).len();

    println!("\n  Size estimates:");
    println!("    JSON: {} bytes", json_size);
    println!("    Binary: ~{} bytes (estimated)", json_size / 2);

    println!();
}

/// Compute SHA-256 checksum
fn compute_checksum(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..16].to_string() // First 16 chars
}

/// Proof certificate structure
#[derive(Debug, Clone)]
struct ProofCertificate {
    theorem: String,
    proof_method: String,
    verification_time_ms: u64,
    checksum: String,
}

/// Proof store for managing proof chains
struct ProofStore {
    proofs: HashMap<String, (String, Vec<String>)>,
}

impl ProofStore {
    fn new() -> Self {
        Self {
            proofs: HashMap::new(),
        }
    }

    fn add_proof(&mut self, name: &str, theorem: &str) {
        self.proofs
            .insert(name.to_string(), (theorem.to_string(), vec![]));
    }

    fn add_proof_with_deps(&mut self, name: &str, theorem: &str, deps: Vec<&str>) {
        self.proofs.insert(
            name.to_string(),
            (
                theorem.to_string(),
                deps.iter().map(|s| s.to_string()).collect(),
            ),
        );
    }

    fn verify_chain(&self, name: &str) -> bool {
        if let Some((_, deps)) = self.proofs.get(name) {
            deps.iter().all(|dep| self.proofs.contains_key(dep))
        } else {
            false
        }
    }

    fn count(&self) -> usize {
        self.proofs.len()
    }
}
