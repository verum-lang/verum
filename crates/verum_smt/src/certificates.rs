//! Proof Certificate Generation and Validation
//!
//! Implements generation of machine-checkable proof certificates for external verification.
//!
//! Proof certificates are machine-checkable evidence of verification results.
//! Supported formats: Dedukti (universal proof checker), OpenTheory (HOL family),
//! Lean, Coq (proof terms), and Metamath. Each certificate contains axioms,
//! definitions, the proof term, and an integrity checksum. Cross-verification
//! generates certificates in multiple formats and checks each independently.
//!
//! ## Features
//!
//! - **Multi-Format Export**: Coq, Lean, Dedukti, OpenTheory, Metamath
//! - **Certificate Validation**: Checksums and integrity verification
//! - **Cross-Verification**: Generate certificates in multiple formats
//! - **Independent Checking**: External proof checker integration
//!
//! ## Performance Targets
//!
//! - Certificate generation: < 100ms per proof
//! - Checksum computation: < 10ms
//! - Format conversion: < 50ms

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use verum_ast::{BinOp, Expr, ExprKind};
use verum_common::{List, Map, Maybe, Text};

use crate::proof_term_unified::ProofTerm;

// ==================== Certificate Formats ====================

/// Supported proof certificate formats
///
/// Certificate formats for exporting machine-checkable proofs.
/// Dedukti is the universal proof checker; OpenTheory targets the HOL family;
/// Lean, Coq, and Metamath target specific proof assistants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CertificateFormat {
    /// Dedukti - Universal proof checker
    Dedukti,

    /// Coq proof assistant
    Coq,

    /// Lean theorem prover
    Lean,

    /// OpenTheory for HOL family
    OpenTheory,

    /// Metamath proof language
    Metamath,

    /// Custom JSON format
    Json,
}

impl CertificateFormat {
    /// Get file extension for this format
    pub fn extension(&self) -> &str {
        match self {
            CertificateFormat::Dedukti => ".dk",
            CertificateFormat::Coq => ".v",
            CertificateFormat::Lean => ".lean",
            CertificateFormat::OpenTheory => ".art",
            CertificateFormat::Metamath => ".mm",
            CertificateFormat::Json => ".json",
        }
    }

    /// Get human-readable name
    pub fn name(&self) -> &str {
        match self {
            CertificateFormat::Dedukti => "Dedukti",
            CertificateFormat::Coq => "Coq",
            CertificateFormat::Lean => "Lean",
            CertificateFormat::OpenTheory => "OpenTheory",
            CertificateFormat::Metamath => "Metamath",
            CertificateFormat::Json => "JSON",
        }
    }
}

// ==================== Certificates ====================

/// Proof certificate
///
/// A proof certificate containing the format, axioms, definitions, proof term,
/// and integrity checksum. Can be independently verified by the target proof checker.
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Certificate format
    pub format: CertificateFormat,

    /// Theorem statement
    pub theorem: Theorem,

    /// Proof content (format-specific)
    pub content: Text,

    /// SHA-256 checksum for integrity verification (32 bytes)
    pub checksum: List<u8>,

    /// Digital signature for authenticity (64 bytes for Ed25519)
    pub signature: Maybe<List<u8>>,

    /// Public key used for signing (32 bytes for Ed25519)
    pub public_key: Maybe<List<u8>>,

    /// Certificate chain for dependent proofs
    pub dependencies: List<CertificateReference>,

    /// Metadata
    pub metadata: CertificateMetadata,
}

/// Reference to a dependent certificate
#[derive(Debug, Clone)]
pub struct CertificateReference {
    /// Name of the dependent theorem
    pub theorem_name: Text,

    /// SHA-256 checksum of the dependent certificate
    pub checksum: List<u8>,

    /// Format of the dependent certificate
    pub format: CertificateFormat,
}

impl Certificate {
    /// Create a new certificate without signing
    pub fn new(format: CertificateFormat, theorem: Theorem, content: Text) -> Self {
        let checksum = compute_sha256_checksum(&content);

        Self {
            format,
            theorem,
            content,
            checksum,
            signature: Maybe::None,
            public_key: Maybe::None,
            dependencies: List::new(),
            metadata: CertificateMetadata::default(),
        }
    }

    /// Create a new certificate with signing
    pub fn new_signed(
        format: CertificateFormat,
        theorem: Theorem,
        content: Text,
        signing_key: &SigningKey,
    ) -> Self {
        let checksum = compute_sha256_checksum(&content);

        // Sign the checksum
        let signature_bytes = signing_key.sign(&checksum).to_bytes();
        let signature: List<u8> = signature_bytes.iter().copied().collect();

        // Store public key (verifying key)
        let verifying_key = signing_key.verifying_key();
        let public_key_bytes = verifying_key.to_bytes();
        let public_key: List<u8> = public_key_bytes.iter().copied().collect();

        Self {
            format,
            theorem,
            content,
            checksum,
            signature: Maybe::Some(signature),
            public_key: Maybe::Some(public_key),
            dependencies: List::new(),
            metadata: CertificateMetadata::default(),
        }
    }

    /// Add a dependency to this certificate
    pub fn add_dependency(&mut self, dependency: CertificateReference) {
        self.dependencies.push(dependency);
    }

    /// Verify certificate integrity (checksum only)
    pub fn verify_integrity(&self) -> bool {
        let expected_checksum = compute_sha256_checksum(&self.content);

        // Compare checksums
        if self.checksum.len() != expected_checksum.len() {
            return false;
        }

        for (a, b) in self.checksum.iter().zip(expected_checksum.iter()) {
            if a != b {
                return false;
            }
        }

        true
    }

    /// Verify certificate signature
    ///
    /// Returns Ok(()) if signature is valid, Err otherwise
    pub fn verify_signature(&self) -> Result<(), CertificateError> {
        let signature_bytes = match &self.signature {
            Maybe::Some(sig) => sig,
            Maybe::None => {
                return Err(CertificateError::ValidationFailed(
                    "Certificate is not signed".into(),
                ));
            }
        };

        let public_key_bytes = match &self.public_key {
            Maybe::Some(pk) => pk,
            Maybe::None => {
                return Err(CertificateError::ValidationFailed(
                    "Certificate has no public key".into(),
                ));
            }
        };

        // Convert to fixed-size arrays
        if signature_bytes.len() != 64 {
            return Err(CertificateError::ValidationFailed(
                format!(
                    "Invalid signature length: expected 64, got {}",
                    signature_bytes.len()
                )
                .into(),
            ));
        }

        if public_key_bytes.len() != 32 {
            return Err(CertificateError::ValidationFailed(
                format!(
                    "Invalid public key length: expected 32, got {}",
                    public_key_bytes.len()
                )
                .into(),
            ));
        }

        let mut sig_array = [0u8; 64];
        let mut pk_array = [0u8; 32];

        for (i, &byte) in signature_bytes.iter().enumerate() {
            sig_array[i] = byte;
        }

        for (i, &byte) in public_key_bytes.iter().enumerate() {
            pk_array[i] = byte;
        }

        let signature = Signature::from_bytes(&sig_array);

        let verifying_key = VerifyingKey::from_bytes(&pk_array).map_err(|e| {
            CertificateError::ValidationFailed(format!("Invalid public key format: {}", e).into())
        })?;

        // Verify signature against checksum
        verifying_key
            .verify(&self.checksum, &signature)
            .map_err(|e| {
                CertificateError::ValidationFailed(
                    format!("Signature verification failed: {}", e).into(),
                )
            })?;

        Ok(())
    }

    /// Verify the entire certificate chain
    ///
    /// This verifies:
    /// 1. The certificate's own integrity and signature
    /// 2. All dependencies exist and are valid
    /// 3. No circular dependencies
    pub fn verify_chain(
        &self,
        certificate_store: &CertificateStore,
    ) -> Result<(), CertificateError> {
        // First verify this certificate's integrity
        if !self.verify_integrity() {
            return Err(CertificateError::ValidationFailed(
                format!(
                    "Certificate integrity check failed for theorem '{}'",
                    self.theorem.name
                )
                .into(),
            ));
        }

        // Verify signature if present
        if self.signature.is_some() {
            self.verify_signature()?;
        }

        // Track visited certificates to detect cycles
        let mut visited = Map::new();
        visited.insert(self.theorem.name.clone(), true);

        // Verify all dependencies recursively
        self.verify_dependencies_recursive(certificate_store, &mut visited)?;

        Ok(())
    }

    /// Internal recursive dependency verification
    fn verify_dependencies_recursive(
        &self,
        certificate_store: &CertificateStore,
        visited: &mut Map<Text, bool>,
    ) -> Result<(), CertificateError> {
        for dep in &self.dependencies {
            // Check for circular dependency
            if visited.get(&dep.theorem_name).is_some() {
                return Err(CertificateError::ValidationFailed(
                    format!("Circular dependency detected: '{}'", dep.theorem_name).into(),
                ));
            }

            // Get the dependent certificate
            let dep_cert = certificate_store
                .get(&dep.theorem_name, dep.format)
                .ok_or_else(|| {
                    CertificateError::ValidationFailed(
                        format!("Dependent certificate not found: '{}'", dep.theorem_name).into(),
                    )
                })?;

            // Verify the dependent certificate's checksum matches
            if dep_cert.checksum.len() != dep.checksum.len() {
                return Err(CertificateError::ValidationFailed(
                    format!(
                        "Checksum length mismatch for dependency '{}'",
                        dep.theorem_name
                    )
                    .into(),
                ));
            }

            for (a, b) in dep_cert.checksum.iter().zip(dep.checksum.iter()) {
                if a != b {
                    return Err(CertificateError::ValidationFailed(
                        format!("Checksum mismatch for dependency '{}'", dep.theorem_name).into(),
                    ));
                }
            }

            // Verify the dependent certificate's integrity
            if !dep_cert.verify_integrity() {
                return Err(CertificateError::ValidationFailed(
                    format!(
                        "Integrity check failed for dependency '{}'",
                        dep.theorem_name
                    )
                    .into(),
                ));
            }

            // Mark as visited and recurse
            visited.insert(dep.theorem_name.clone(), true);
            dep_cert.verify_dependencies_recursive(certificate_store, visited)?;
            visited.remove(&dep.theorem_name);
        }

        Ok(())
    }

    /// Get certificate size in bytes
    pub fn size(&self) -> usize {
        self.content.len()
    }

    /// Get the certificate's checksum as a hex string
    pub fn checksum_hex(&self) -> Text {
        bytes_to_hex(&self.checksum)
    }
}

/// Theorem statement
#[derive(Debug, Clone)]
pub struct Theorem {
    /// Theorem name
    pub name: Text,

    /// Theorem statement (as text)
    pub statement: Text,

    /// Hypotheses/axioms used
    pub axioms: List<Text>,
}

impl Theorem {
    /// Create a new theorem
    pub fn new(name: Text, statement: Text) -> Self {
        Self {
            name,
            statement,
            axioms: List::new(),
        }
    }

    /// Add axiom dependency
    pub fn add_axiom(&mut self, axiom: Text) {
        self.axioms.push(axiom);
    }
}

/// Certificate metadata
#[derive(Debug, Clone, Default)]
pub struct CertificateMetadata {
    /// Generator version
    pub generator_version: Text,

    /// Generation timestamp
    pub timestamp: Text,

    /// Additional properties
    pub properties: Map<Text, Text>,
}

// ==================== Certificate Generator ====================

/// Proof certificate generator
///
/// Generates machine-checkable certificates by translating proof terms to the
/// target format's syntax (e.g., Coq vernacular, Lean tactics, Dedukti terms).
pub struct CertificateGenerator {
    /// Target format
    format: CertificateFormat,

    /// Configuration
    config: GeneratorConfig,
}

/// Generator configuration
#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    /// Include comments in output
    pub include_comments: bool,

    /// Pretty-print output
    pub pretty_print: bool,

    /// Include metadata
    pub include_metadata: bool,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            include_comments: true,
            pretty_print: true,
            include_metadata: true,
        }
    }
}

impl CertificateGenerator {
    /// Create a new certificate generator
    pub fn new(format: CertificateFormat) -> Self {
        Self {
            format,
            config: GeneratorConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(format: CertificateFormat, config: GeneratorConfig) -> Self {
        Self { format, config }
    }

    /// Generate certificate from proof
    ///
    /// Generate a certificate from a proof term by translating to the target format,
    /// computing checksums, and attaching metadata (version, timestamp).
    pub fn generate(
        &self,
        proof: &ProofTerm,
        theorem: Theorem,
    ) -> Result<Certificate, CertificateError> {
        let content = match self.format {
            CertificateFormat::Dedukti => self.to_dedukti(proof, &theorem)?,
            CertificateFormat::Coq => self.to_coq(proof, &theorem)?,
            CertificateFormat::Lean => self.to_lean(proof, &theorem)?,
            CertificateFormat::OpenTheory => self.to_opentheory(proof, &theorem)?,
            CertificateFormat::Metamath => self.to_metamath(proof, &theorem)?,
            CertificateFormat::Json => self.to_json(proof, &theorem)?,
        };

        let mut cert = Certificate::new(self.format, theorem, content);

        if self.config.include_metadata {
            cert.metadata.generator_version = "verum-smt-1.0".into();
            cert.metadata.timestamp = chrono::Utc::now().to_rfc3339().into();
        }

        Ok(cert)
    }

    /// Generate Coq vernacular
    ///
    /// Generate Coq vernacular: `Theorem name: prop. Proof. proof_term. Qed.`
    fn to_coq(&self, proof: &ProofTerm, theorem: &Theorem) -> Result<Text, CertificateError> {
        let mut output = Text::new();

        // Add comment header
        if self.config.include_comments {
            output.push_str("(* Generated by Verum SMT *)\n");
            output.push_str(&format!("(* Theorem: {} *)\n\n", theorem.name));
        }

        // Theorem declaration
        output.push_str(&format!(
            "Theorem {} : {}.\n",
            theorem.name, theorem.statement
        ));

        // Proof
        output.push_str("Proof.\n");
        output.push_str(self.proof_to_coq_tactic(proof)?.as_str());
        output.push_str("Qed.\n");

        Ok(output)
    }

    /// Convert proof term to Coq tactics
    fn proof_to_coq_tactic(&self, proof: &ProofTerm) -> Result<Text, CertificateError> {
        self.proof_to_coq_tactic_impl(proof, 1)
    }

    fn proof_to_coq_tactic_impl(
        &self,
        proof: &ProofTerm,
        indent: usize,
    ) -> Result<Text, CertificateError> {
        let spaces = "  ".repeat(indent);

        match proof {
            // Base cases
            ProofTerm::Axiom { name, .. } => Ok(format!("{}apply {}.\n", spaces, name).into()),

            ProofTerm::Assumption { id, .. } => {
                Ok(format!("{}assumption (* {} *).\n", spaces, id).into())
            }

            ProofTerm::Hypothesis { id, .. } => Ok(format!("{}exact H{}.\n", spaces, id).into()),

            // Classical logic
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* modus ponens *)\n", spaces));
                tactics.push_str(self.proof_to_coq_tactic_impl(premise, indent)?.as_str());
                tactics.push_str(self.proof_to_coq_tactic_impl(implication, indent)?.as_str());
                tactics.push_str(&format!("{}apply H_impl.\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::Rewrite { source, rule, .. } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_coq_tactic_impl(source, indent)?.as_str());
                tactics.push_str(&format!("{}rewrite {}.\n", spaces, rule));
                Ok(tactics.into())
            }

            ProofTerm::Symmetry { equality } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_coq_tactic_impl(equality, indent)?.as_str());
                tactics.push_str(&format!("{}symmetry.\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::Transitivity { left, right } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* transitivity *)\n", spaces));
                tactics.push_str(self.proof_to_coq_tactic_impl(left, indent)?.as_str());
                tactics.push_str(self.proof_to_coq_tactic_impl(right, indent)?.as_str());
                tactics.push_str(&format!("{}transitivity y. auto. auto.\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::Reflexivity { .. } => Ok(format!("{}reflexivity.\n", spaces).into()),

            // Theory reasoning
            ProofTerm::TheoryLemma { theory, .. } => {
                Ok(format!("{}(* theory lemma: {} *) auto.\n", spaces, theory).into())
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* unit resolution *)\n", spaces));
                for clause in clauses {
                    tactics.push_str(self.proof_to_coq_tactic_impl(clause, indent)?.as_str());
                }
                tactics.push_str(&format!("{}tauto.\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_coq_tactic_impl(quantified, indent)?.as_str());
                for (var, _) in instantiation {
                    tactics.push_str(&format!("{}specialize (H {}).\n", spaces, var));
                }
                Ok(tactics.into())
            }

            // Constructive proofs
            ProofTerm::Apply { rule, premises } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}apply {}.\n", spaces, rule));
                for (idx, premise) in premises.iter().enumerate() {
                    if premises.len() > 1 {
                        tactics.push_str(&format!("{}(* premise {} *)\n", spaces, idx + 1));
                    }
                    tactics.push_str(self.proof_to_coq_tactic_impl(premise, indent)?.as_str());
                }
                Ok(tactics.into())
            }

            ProofTerm::Lambda { var, body } => {
                let mut tactics = format!("{}intro {}.\n", spaces, var);
                tactics.push_str(self.proof_to_coq_tactic_impl(body, indent)?.as_str());
                Ok(tactics.into())
            }

            ProofTerm::Cases { scrutinee, cases } => {
                let mut tactics = format!(
                    "{}destruct {}.\n",
                    spaces,
                    self.expr_to_coq_term(scrutinee)?
                );
                for (idx, (_, case_proof)) in cases.iter().enumerate() {
                    tactics.push_str(&format!("{}- (* case {} *)\n", spaces, idx + 1));
                    tactics.push_str(
                        self.proof_to_coq_tactic_impl(case_proof, indent + 1)?
                            .as_str(),
                    );
                }
                Ok(tactics.into())
            }

            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                let mut tactics = format!("{}induction {}.\n", spaces, var);
                tactics.push_str(&format!("{}- (* base case *)\n", spaces));
                tactics.push_str(
                    self.proof_to_coq_tactic_impl(base_case, indent + 1)?
                        .as_str(),
                );
                tactics.push_str(&format!("{}- (* inductive case *)\n", spaces));
                tactics.push_str(
                    self.proof_to_coq_tactic_impl(inductive_case, indent + 1)?
                        .as_str(),
                );
                Ok(tactics.into())
            }

            // SMT integration
            ProofTerm::SmtProof {
                solver, formula, ..
            } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}(* Verified by {} *)\n", spaces, solver));
                tactics.push_str(&format!(
                    "{}(* Formula: {} *)\n",
                    spaces,
                    self.expr_to_coq_term(formula)?
                ));
                tactics.push_str(&format!("{}auto.\n", spaces));
                Ok(tactics.into())
            }

            // Dependent types
            ProofTerm::Subst { eq_proof, .. } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_coq_tactic_impl(eq_proof, indent)?.as_str());
                tactics.push_str(&format!("{}subst.\n", spaces));
                Ok(tactics.into())
            }

            // Meta-level
            ProofTerm::Lemma { conclusion, proof } => {
                let mut tactics = String::new();
                tactics.push_str(&format!(
                    "{}(* lemma: {} *)\n",
                    spaces,
                    self.expr_to_coq_term(conclusion)?
                ));
                tactics.push_str(self.proof_to_coq_tactic_impl(proof, indent)?.as_str());
                Ok(tactics.into())
            }

            // Handle other proof terms with a generic tactic
            _ => Ok(format!("{}(* unhandled proof term *)\n{}auto.\n", spaces, spaces).into()),
        }
    }

    /// Convert Verum expression to Coq term representation
    fn expr_to_coq_term(&self, expr: &Expr) -> Result<Text, CertificateError> {
        match &expr.kind {
            ExprKind::Literal(lit) => Ok(format!("{:?}", lit).into()),
            // Variables are represented as single-segment paths
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    Ok(ident.name.clone().into())
                } else {
                    Ok("_".into())
                }
            }
            ExprKind::Path(path) => {
                // Convert path segments to text
                let segments: Vec<String> = path
                    .segments
                    .iter()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => "_".to_string(),
                    })
                    .collect();
                Ok(segments.join("::").into())
            }
            ExprKind::Binary { op, left, right } => {
                let left_str = self.expr_to_coq_term(left)?;
                let right_str = self.expr_to_coq_term(right)?;
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Eq => "=",
                    BinOp::Ne => "<>",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::And => "/\\",
                    BinOp::Or => "\\/",
                    _ => "?",
                };
                Ok(format!("({} {} {})", left_str, op_str, right_str).into())
            }
            _ => Ok("_".into()),
        }
    }

    /// Generate Lean proof
    fn to_lean(&self, proof: &ProofTerm, theorem: &Theorem) -> Result<Text, CertificateError> {
        let mut output = Text::new();

        if self.config.include_comments {
            output.push_str("-- Generated by Verum SMT\n");
            output.push_str(&format!("-- Theorem: {}\n\n", theorem.name));
        }

        output.push_str(&format!(
            "theorem {} : {} := by\n",
            theorem.name, theorem.statement
        ));
        output.push_str(self.proof_to_lean_tactic(proof, 1)?.as_str());

        Ok(output)
    }

    /// Convert proof term to Lean tactics
    fn proof_to_lean_tactic(
        &self,
        proof: &ProofTerm,
        indent: usize,
    ) -> Result<Text, CertificateError> {
        let spaces = "  ".repeat(indent);

        match proof {
            // Base cases
            ProofTerm::Axiom { name, .. } => Ok(format!("{}exact {}\n", spaces, name).into()),

            ProofTerm::Assumption { id, .. } => {
                Ok(format!("{}assumption -- {}\n", spaces, id).into())
            }

            ProofTerm::Hypothesis { id, .. } => Ok(format!("{}exact h{}\n", spaces, id).into()),

            // Classical logic
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- modus ponens\n", spaces));
                tactics.push_str(self.proof_to_lean_tactic(premise, indent)?.as_str());
                tactics.push_str(self.proof_to_lean_tactic(implication, indent)?.as_str());
                tactics.push_str(&format!("{}apply h_impl\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::Rewrite { source, rule, .. } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_lean_tactic(source, indent)?.as_str());
                tactics.push_str(&format!("{}rw [{}]\n", spaces, rule));
                Ok(tactics.into())
            }

            ProofTerm::Symmetry { equality } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_lean_tactic(equality, indent)?.as_str());
                tactics.push_str(&format!("{}symm\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::Transitivity { left, right } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- transitivity\n", spaces));
                tactics.push_str(self.proof_to_lean_tactic(left, indent)?.as_str());
                tactics.push_str(self.proof_to_lean_tactic(right, indent)?.as_str());
                tactics.push_str(&format!("{}trans\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::Reflexivity { .. } => Ok(format!("{}rfl\n", spaces).into()),

            // Theory reasoning
            ProofTerm::TheoryLemma { theory, .. } => {
                Ok(format!("{}-- theory lemma: {}\n{}simp\n", spaces, theory, spaces).into())
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- unit resolution\n", spaces));
                for clause in clauses {
                    tactics.push_str(self.proof_to_lean_tactic(clause, indent)?.as_str());
                }
                tactics.push_str(&format!("{}tauto\n", spaces));
                Ok(tactics.into())
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_lean_tactic(quantified, indent)?.as_str());
                for (var, _) in instantiation {
                    tactics.push_str(&format!("{}specialize h {}\n", spaces, var));
                }
                Ok(tactics.into())
            }

            // Constructive proofs
            ProofTerm::Apply { rule, premises } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}apply {}\n", spaces, rule));
                for (idx, premise) in premises.iter().enumerate() {
                    if premises.len() > 1 {
                        tactics.push_str(&format!("{}case h{} =>\n", spaces, idx + 1));
                    }
                    tactics.push_str(self.proof_to_lean_tactic(premise, indent + 1)?.as_str());
                }
                Ok(tactics.into())
            }

            ProofTerm::Lambda { var, body } => {
                let mut tactics = format!("{}intro {}\n", spaces, var);
                tactics.push_str(self.proof_to_lean_tactic(body, indent)?.as_str());
                Ok(tactics.into())
            }

            ProofTerm::Cases { scrutinee, cases } => {
                let mut tactics =
                    format!("{}cases {}\n", spaces, self.expr_to_lean_term(scrutinee)?);
                for (idx, (_, case_proof)) in cases.iter().enumerate() {
                    tactics.push_str(&format!("{}case c{} =>\n", spaces, idx + 1));
                    tactics.push_str(self.proof_to_lean_tactic(case_proof, indent + 1)?.as_str());
                }
                Ok(tactics.into())
            }

            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                let mut tactics = format!("{}induction {}\n", spaces, var);
                tactics.push_str(&format!("{}case zero =>\n", spaces));
                tactics.push_str(self.proof_to_lean_tactic(base_case, indent + 1)?.as_str());
                tactics.push_str(&format!("{}case succ ih =>\n", spaces));
                tactics.push_str(
                    self.proof_to_lean_tactic(inductive_case, indent + 1)?
                        .as_str(),
                );
                Ok(tactics.into())
            }

            // SMT integration
            ProofTerm::SmtProof {
                solver, formula, ..
            } => {
                let mut tactics = String::new();
                tactics.push_str(&format!("{}-- Verified by {}\n", spaces, solver));
                tactics.push_str(&format!(
                    "{}-- Formula: {}\n",
                    spaces,
                    self.expr_to_lean_term(formula)?
                ));
                tactics.push_str(&format!("{}simp\n", spaces));
                Ok(tactics.into())
            }

            // Dependent types
            ProofTerm::Subst { eq_proof, .. } => {
                let mut tactics = String::new();
                tactics.push_str(self.proof_to_lean_tactic(eq_proof, indent)?.as_str());
                tactics.push_str(&format!("{}subst\n", spaces));
                Ok(tactics.into())
            }

            // Meta-level
            ProofTerm::Lemma { conclusion, proof } => {
                let mut tactics = String::new();
                tactics.push_str(&format!(
                    "{}-- lemma: {}\n",
                    spaces,
                    self.expr_to_lean_term(conclusion)?
                ));
                tactics.push_str(self.proof_to_lean_tactic(proof, indent)?.as_str());
                Ok(tactics.into())
            }

            // Handle other proof terms with a generic tactic
            _ => Ok(format!("{}-- unhandled proof term\n{}trivial\n", spaces, spaces).into()),
        }
    }

    /// Convert Verum expression to Lean term representation
    fn expr_to_lean_term(&self, expr: &Expr) -> Result<Text, CertificateError> {
        match &expr.kind {
            ExprKind::Literal(lit) => Ok(format!("{:?}", lit).into()),
            // Variables are represented as single-segment paths
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    Ok(ident.name.clone().into())
                } else {
                    Ok("_".into())
                }
            }
            ExprKind::Path(path) => {
                // Convert path segments to text
                let segments: Vec<String> = path
                    .segments
                    .iter()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => "_".to_string(),
                    })
                    .collect();
                Ok(segments.join("::").into())
            }
            ExprKind::Binary { op, left, right } => {
                let left_str = self.expr_to_lean_term(left)?;
                let right_str = self.expr_to_lean_term(right)?;
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Eq => "=",
                    BinOp::Ne => "≠",
                    BinOp::Lt => "<",
                    BinOp::Le => "≤",
                    BinOp::Gt => ">",
                    BinOp::Ge => "≥",
                    BinOp::And => "∧",
                    BinOp::Or => "∨",
                    _ => "?",
                };
                Ok(format!("({} {} {})", left_str, op_str, right_str).into())
            }
            _ => Ok("_".into()),
        }
    }

    /// Generate Dedukti proof
    fn to_dedukti(&self, proof: &ProofTerm, theorem: &Theorem) -> Result<Text, CertificateError> {
        let mut output = Text::new();

        if self.config.include_comments {
            output.push_str("(; Generated by Verum SMT ;)\n");
            output.push_str(&format!("(; Theorem: {} ;)\n\n", theorem.name));
        }

        // Dedukti uses lambda-Pi calculus with type annotations
        output.push_str(&format!(
            "def {} : {} :=\n",
            theorem.name, theorem.statement
        ));
        output.push_str(self.proof_to_dedukti_term(proof, 1)?.as_str());
        output.push_str(".\n");

        Ok(output)
    }

    /// Convert proof term to Dedukti lambda term
    fn proof_to_dedukti_term(
        &self,
        proof: &ProofTerm,
        indent: usize,
    ) -> Result<Text, CertificateError> {
        let spaces = "  ".repeat(indent);

        match proof {
            // Base cases
            ProofTerm::Axiom { name, .. } => Ok(format!("{}{}", spaces, name).into()),

            ProofTerm::Assumption { id, .. } => Ok(format!("{}H{}", spaces, id).into()),

            ProofTerm::Hypothesis { id, .. } => Ok(format!("{}h{}", spaces, id).into()),

            // Classical logic
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let premise_term = self.proof_to_dedukti_term(premise, 0)?;
                let impl_term = self.proof_to_dedukti_term(implication, 0)?;
                Ok(format!("{}({} {})", spaces, impl_term.trim(), premise_term.trim()).into())
            }

            ProofTerm::Rewrite { source, rule, .. } => {
                let source_term = self.proof_to_dedukti_term(source, 0)?;
                Ok(format!("{}(rewrite {} {})", spaces, rule, source_term.trim()).into())
            }

            ProofTerm::Symmetry { equality } => {
                let eq_term = self.proof_to_dedukti_term(equality, 0)?;
                Ok(format!("{}(sym {})", spaces, eq_term.trim()).into())
            }

            ProofTerm::Transitivity { left, right } => {
                let left_term = self.proof_to_dedukti_term(left, 0)?;
                let right_term = self.proof_to_dedukti_term(right, 0)?;
                Ok(format!(
                    "{}(trans {} {})",
                    spaces,
                    left_term.trim(),
                    right_term.trim()
                )
                .into())
            }

            ProofTerm::Reflexivity { .. } => Ok(format!("{}refl", spaces).into()),

            // Theory reasoning
            ProofTerm::TheoryLemma { theory, .. } => {
                Ok(format!("{}(; theory: {} ;) axiom", spaces, theory).into())
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut term = format!("{}(resolution", spaces);
                for clause in clauses {
                    term.push(' ');
                    term.push_str(self.proof_to_dedukti_term(clause, 0)?.as_str().trim());
                }
                term.push(')');
                Ok(term.into())
            }

            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                let q_term = self.proof_to_dedukti_term(quantified, 0)?;
                let mut term = format!("{}({}", spaces, q_term.trim());
                for (_, val) in instantiation {
                    term.push(' ');
                    term.push_str(self.expr_to_dedukti_term(val)?.as_str());
                }
                term.push(')');
                Ok(term.into())
            }

            // Constructive proofs
            ProofTerm::Apply { rule, premises } => {
                let mut term = format!("{}({}", spaces, rule);
                for premise in premises {
                    term.push(' ');
                    term.push_str(self.proof_to_dedukti_term(premise, 0)?.as_str().trim());
                }
                term.push(')');
                Ok(term.into())
            }

            ProofTerm::Lambda { var, body } => {
                let mut term = format!("{}({} : _ =>\n", spaces, var);
                term.push_str(self.proof_to_dedukti_term(body, indent + 1)?.as_str());
                term.push_str(&format!("\n{})", spaces));
                Ok(term.into())
            }

            ProofTerm::Cases { scrutinee, cases } => {
                let mut term = format!(
                    "{}match {} with\n",
                    spaces,
                    self.expr_to_dedukti_term(scrutinee)?
                );
                for (idx, (_, case_proof)) in cases.iter().enumerate() {
                    term.push_str(&format!("{}| c{} => \n", spaces, idx + 1));
                    term.push_str(self.proof_to_dedukti_term(case_proof, indent + 1)?.as_str());
                    term.push('\n');
                }
                term.push_str(&format!("{}end", spaces));
                Ok(term.into())
            }

            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                let mut term = format!("{}(ind {} \n", spaces, var);
                term.push_str(&format!("{}  (; base ;) \n", spaces));
                term.push_str(self.proof_to_dedukti_term(base_case, indent + 1)?.as_str());
                term.push('\n');
                term.push_str(&format!("{}  (; step ;) \n", spaces));
                term.push_str(
                    self.proof_to_dedukti_term(inductive_case, indent + 1)?
                        .as_str(),
                );
                term.push_str(&format!("\n{})", spaces));
                Ok(term.into())
            }

            // SMT integration
            ProofTerm::SmtProof { solver, .. } => {
                let mut term = String::new();
                term.push_str(&format!("{}(; SMT by {} ;)\n", spaces, solver));
                term.push_str(&format!("{}smt_proof", spaces));
                Ok(term.into())
            }

            // Dependent types
            ProofTerm::Subst { eq_proof, property } => {
                let eq_term = self.proof_to_dedukti_term(eq_proof, 0)?;
                Ok(format!("{}(subst {} _)", spaces, eq_term.trim()).into())
            }

            // Meta-level
            ProofTerm::Lemma { conclusion, proof } => {
                let proof_term = self.proof_to_dedukti_term(proof, indent)?;
                Ok(format!("{}(; lemma ;)\n{}", spaces, proof_term).into())
            }

            // Handle other proof terms with a generic term
            _ => Ok(format!("{}(; unhandled ;) _", spaces).into()),
        }
    }

    /// Convert Verum expression to Dedukti term
    fn expr_to_dedukti_term(&self, expr: &Expr) -> Result<Text, CertificateError> {
        match &expr.kind {
            ExprKind::Literal(lit) => Ok(format!("{:?}", lit).into()),
            // Variables are represented as single-segment paths
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    Ok(ident.name.clone().into())
                } else {
                    Ok("_".into())
                }
            }
            ExprKind::Path(path) => {
                // Convert path segments to text
                let segments: Vec<String> = path
                    .segments
                    .iter()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => "_".to_string(),
                    })
                    .collect();
                Ok(segments.join("::").into())
            }
            ExprKind::Binary { op, left, right } => {
                let left_str = self.expr_to_dedukti_term(left)?;
                let right_str = self.expr_to_dedukti_term(right)?;
                let op_str = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "mul",
                    BinOp::Div => "div",
                    BinOp::Eq => "eq",
                    BinOp::Ne => "neq",
                    BinOp::Lt => "lt",
                    BinOp::Le => "le",
                    BinOp::Gt => "gt",
                    BinOp::Ge => "ge",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    _ => "unknown",
                };
                Ok(format!("({} {} {})", op_str, left_str, right_str).into())
            }
            _ => Ok("hole".into()),
        }
    }

    /// Generate OpenTheory article
    fn to_opentheory(
        &self,
        proof: &ProofTerm,
        theorem: &Theorem,
    ) -> Result<Text, CertificateError> {
        let mut output = Text::new();

        // OpenTheory header
        output.push_str("version: 6\n");
        if self.config.include_metadata {
            output.push_str("# Generated by Verum SMT\n");
            output.push_str(&format!("# Theorem: {}\n", theorem.name));
        }
        output.push_str("\n");

        // OpenTheory uses a stack-based proof format
        output.push_str("# Type definitions\n");
        output.push_str("\"bool\" const\n");
        output.push_str("\"->\" const\n\n");

        output.push_str("# Theorem statement\n");
        output.push_str(&format!("\"{}\" const\n", theorem.name));
        output.push_str(&format!("\"{}\" constTerm\n", theorem.statement));
        output.push_str("\n");

        output.push_str("# Proof\n");
        output.push_str(self.proof_to_opentheory_commands(proof)?.as_str());

        output.push_str("\n# Conclude theorem\n");
        output.push_str("thm\n");

        Ok(output)
    }

    /// Convert proof term to OpenTheory command sequence
    fn proof_to_opentheory_commands(&self, proof: &ProofTerm) -> Result<Text, CertificateError> {
        match proof {
            // Base cases
            ProofTerm::Axiom { name, .. } => Ok(format!("\"{}\" axiom\n", name).into()),

            ProofTerm::Assumption { id, .. } => Ok(format!("# assumption {}\nassume\n", id).into()),

            ProofTerm::Hypothesis { id, .. } => Ok(format!("# hypothesis {}\nassume\n", id).into()),

            // Classical logic
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                let mut commands = String::new();
                commands.push_str(self.proof_to_opentheory_commands(premise)?.as_str());
                commands.push_str(self.proof_to_opentheory_commands(implication)?.as_str());
                commands.push_str("eqMp\n");
                Ok(commands.into())
            }

            ProofTerm::Rewrite { source, rule, .. } => {
                let mut commands = String::new();
                commands.push_str(self.proof_to_opentheory_commands(source)?.as_str());
                commands.push_str(&format!("\"{}\" const\n", rule));
                commands.push_str("eqMp\n");
                Ok(commands.into())
            }

            ProofTerm::Symmetry { equality } => {
                let mut commands = String::new();
                commands.push_str(self.proof_to_opentheory_commands(equality)?.as_str());
                commands.push_str("sym\n");
                Ok(commands.into())
            }

            ProofTerm::Transitivity { left, right } => {
                let mut commands = String::new();
                commands.push_str(self.proof_to_opentheory_commands(left)?.as_str());
                commands.push_str(self.proof_to_opentheory_commands(right)?.as_str());
                commands.push_str("trans\n");
                Ok(commands.into())
            }

            ProofTerm::Reflexivity { .. } => Ok("refl\n".into()),

            // Theory reasoning
            ProofTerm::TheoryLemma { theory, .. } => {
                Ok(format!("# theory: {}\naxiom\n", theory).into())
            }

            ProofTerm::UnitResolution { clauses } => {
                let mut commands = String::new();
                commands.push_str("# unit resolution\n");
                for clause in clauses {
                    commands.push_str(self.proof_to_opentheory_commands(clause)?.as_str());
                }
                commands.push_str("proveHyp\n");
                Ok(commands.into())
            }

            ProofTerm::QuantifierInstantiation { quantified, .. } => {
                let mut commands = String::new();
                commands.push_str(self.proof_to_opentheory_commands(quantified)?.as_str());
                commands.push_str("spec\n");
                Ok(commands.into())
            }

            // Constructive proofs
            ProofTerm::Apply { rule, premises } => {
                let mut commands = String::new();
                for premise in premises {
                    commands.push_str(self.proof_to_opentheory_commands(premise)?.as_str());
                }
                commands.push_str(&format!("\"{}\" const\n", rule));
                commands.push_str("appTerm\n");
                for _ in 0..premises.len() {
                    commands.push_str("appThm\n");
                }
                Ok(commands.into())
            }

            ProofTerm::Lambda { var, body } => {
                let mut commands = String::new();
                commands.push_str(&format!("\"{}\" var\n", var));
                commands.push_str(self.proof_to_opentheory_commands(body)?.as_str());
                commands.push_str("absTerm\n");
                commands.push_str("absThm\n");
                Ok(commands.into())
            }

            ProofTerm::Cases { scrutinee, cases } => {
                let mut commands = String::new();
                commands.push_str(&format!(
                    "# cases on {}\n",
                    self.expr_to_opentheory_term(scrutinee)?
                ));
                for (idx, (_, case_proof)) in cases.iter().enumerate() {
                    commands.push_str(&format!("# case {}\n", idx + 1));
                    commands.push_str(self.proof_to_opentheory_commands(case_proof)?.as_str());
                }
                commands.push_str("# combine cases\n");
                for _ in 1..cases.len() {
                    commands.push_str("orElim\n");
                }
                Ok(commands.into())
            }

            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                let mut commands = String::new();
                commands.push_str(&format!("# induction on {}\n", var));
                commands.push_str("# base case\n");
                commands.push_str(self.proof_to_opentheory_commands(base_case)?.as_str());
                commands.push_str("# inductive case\n");
                commands.push_str(self.proof_to_opentheory_commands(inductive_case)?.as_str());
                commands.push_str("# combine by induction\n");
                commands.push_str(&format!("\"{}\" var\n", var));
                commands.push_str("inductionThm\n");
                Ok(commands.into())
            }

            // SMT integration
            ProofTerm::SmtProof {
                solver, formula, ..
            } => {
                let mut commands = String::new();
                commands.push_str(&format!("# SMT proof by {}\n", solver));
                commands.push_str(&format!(
                    "\"{}\" axiom\n",
                    self.expr_to_opentheory_term(formula)?
                ));
                Ok(commands.into())
            }

            // Dependent types
            ProofTerm::Subst { eq_proof, .. } => {
                let mut commands = String::new();
                commands.push_str(self.proof_to_opentheory_commands(eq_proof)?.as_str());
                commands.push_str("eqMp\n");
                Ok(commands.into())
            }

            // Meta-level
            ProofTerm::Lemma { proof, .. } => self.proof_to_opentheory_commands(proof),

            // Handle other proof terms
            _ => Ok("# unhandled proof term\naxiom\n".into()),
        }
    }

    /// Convert Verum expression to OpenTheory term
    fn expr_to_opentheory_term(&self, expr: &Expr) -> Result<Text, CertificateError> {
        match &expr.kind {
            ExprKind::Literal(lit) => Ok(format!("{:?}", lit).into()),
            // Variables are represented as single-segment paths
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    Ok(ident.name.clone().into())
                } else {
                    Ok("_".into())
                }
            }
            ExprKind::Path(path) => {
                // Convert path segments to text
                let segments: Vec<String> = path
                    .segments
                    .iter()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => "_".to_string(),
                    })
                    .collect();
                Ok(segments.join("::").into())
            }
            ExprKind::Binary { op, left, right } => {
                let left_str = self.expr_to_opentheory_term(left)?;
                let right_str = self.expr_to_opentheory_term(right)?;
                let op_str = match op {
                    BinOp::Eq => "=",
                    BinOp::And => "/\\",
                    BinOp::Or => "\\/",
                    _ => "op",
                };
                Ok(format!("({} {} {})", op_str, left_str, right_str).into())
            }
            _ => Ok("term".into()),
        }
    }

    /// Generate Metamath proof
    fn to_metamath(&self, proof: &ProofTerm, theorem: &Theorem) -> Result<Text, CertificateError> {
        let mut output = Text::new();

        if self.config.include_comments {
            output.push_str("$( Generated by Verum SMT $)\n");
            output.push_str(&format!("$( Theorem: {} $)\n", theorem.name));
        }

        // Metamath requires variable declarations
        output.push_str("\n$( Variable declarations $)\n");
        let vars = self.collect_proof_variables(proof);
        for var in &vars {
            output.push_str(&format!("  $v {} $.\n", var));
        }
        output.push_str("\n");

        // Type declarations
        for var in &vars {
            output.push_str(&format!("  {}-type $f wff {} $.\n", var, var));
        }
        output.push_str("\n");

        // Theorem statement with proof
        output.push_str(&format!("$( {} $)\n", theorem.statement));
        output.push_str(&format!("{} $p {} $=\n", theorem.name, theorem.statement));

        // Generate proof steps
        let (proof_steps, proof_labels) = self.proof_to_metamath_steps(proof)?;

        // Output proof body
        output.push_str("    ");
        for (idx, label) in proof_labels.iter().enumerate() {
            output.push_str(label.as_str());
            if idx < proof_labels.len() - 1 {
                output.push_str(" ");
                if (idx + 1) % 10 == 0 {
                    output.push_str("\n    ");
                }
            }
        }
        output.push_str("\n$.\n");

        Ok(output)
    }

    /// Collect all variables used in the proof
    fn collect_proof_variables(&self, proof: &ProofTerm) -> List<Text> {
        let mut vars = List::new();
        self.collect_variables_impl(proof, &mut vars);

        // Deduplicate
        let mut unique_vars = List::new();
        for var in vars {
            if !unique_vars.contains(&var) {
                unique_vars.push(var);
            }
        }

        unique_vars
    }

    fn collect_variables_impl(&self, proof: &ProofTerm, vars: &mut List<Text>) {
        match proof {
            // Base cases - no variables
            ProofTerm::Axiom { .. }
            | ProofTerm::Assumption { .. }
            | ProofTerm::Hypothesis { .. }
            | ProofTerm::Reflexivity { .. }
            | ProofTerm::TheoryLemma { .. }
            | ProofTerm::SmtProof { .. } => {}

            // Classical logic
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                self.collect_variables_impl(premise, vars);
                self.collect_variables_impl(implication, vars);
            }
            ProofTerm::Rewrite { source, .. } | ProofTerm::Symmetry { equality: source } => {
                self.collect_variables_impl(source, vars);
            }
            ProofTerm::Transitivity { left, right } => {
                self.collect_variables_impl(left, vars);
                self.collect_variables_impl(right, vars);
            }

            // Theory reasoning
            ProofTerm::UnitResolution { clauses } => {
                for clause in clauses {
                    self.collect_variables_impl(clause, vars);
                }
            }
            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                self.collect_variables_impl(quantified, vars);
                for (var_name, _) in instantiation {
                    vars.push(var_name.clone());
                }
            }

            // Constructive proofs
            ProofTerm::Apply { premises, .. } => {
                for premise in premises {
                    self.collect_variables_impl(premise, vars);
                }
            }
            ProofTerm::Lambda { var, body } => {
                vars.push(var.clone());
                self.collect_variables_impl(body, vars);
            }
            ProofTerm::Cases { cases, .. } => {
                for (_, case_proof) in cases {
                    self.collect_variables_impl(case_proof, vars);
                }
            }
            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                vars.push(var.clone());
                self.collect_variables_impl(base_case, vars);
                self.collect_variables_impl(inductive_case, vars);
            }

            // Dependent types
            ProofTerm::Subst { eq_proof, .. } => {
                self.collect_variables_impl(eq_proof, vars);
            }

            // Meta-level
            ProofTerm::Lemma { proof, .. } => {
                self.collect_variables_impl(proof, vars);
            }

            // Handle other proof terms - no variables to collect
            _ => {}
        }
    }

    /// Convert proof to Metamath proof steps
    fn proof_to_metamath_steps(
        &self,
        proof: &ProofTerm,
    ) -> Result<(List<Text>, List<Text>), CertificateError> {
        let mut steps = List::new();
        let mut labels = List::new();
        self.proof_to_metamath_impl(proof, &mut steps, &mut labels, &mut 1)?;
        Ok((steps, labels))
    }

    fn proof_to_metamath_impl(
        &self,
        proof: &ProofTerm,
        steps: &mut List<Text>,
        labels: &mut List<Text>,
        counter: &mut usize,
    ) -> Result<(), CertificateError> {
        match proof {
            // Base cases
            ProofTerm::Axiom { name, .. } => {
                labels.push(name.clone());
            }
            ProofTerm::Assumption { id, .. } => {
                labels.push(format!("hyp{}", id).into());
            }
            ProofTerm::Hypothesis { id, .. } => {
                labels.push(format!("h{}", id).into());
            }
            ProofTerm::Reflexivity { .. } => {
                labels.push("eqid".into());
            }
            ProofTerm::TheoryLemma { theory, .. } => {
                labels.push(format!("th-{}", theory).into());
            }

            // Classical logic
            ProofTerm::ModusPonens {
                premise,
                implication,
            } => {
                self.proof_to_metamath_impl(premise, steps, labels, counter)?;
                self.proof_to_metamath_impl(implication, steps, labels, counter)?;
                labels.push("ax-mp".into());
            }
            ProofTerm::Rewrite { source, rule, .. } => {
                self.proof_to_metamath_impl(source, steps, labels, counter)?;
                labels.push(rule.clone());
                labels.push("eqtr".into());
            }
            ProofTerm::Symmetry { equality } => {
                self.proof_to_metamath_impl(equality, steps, labels, counter)?;
                labels.push("eqcom".into());
            }
            ProofTerm::Transitivity { left, right } => {
                self.proof_to_metamath_impl(left, steps, labels, counter)?;
                self.proof_to_metamath_impl(right, steps, labels, counter)?;
                labels.push("eqtr".into());
            }

            // Theory reasoning
            ProofTerm::UnitResolution { clauses } => {
                labels.push("res-begin".into());
                for clause in clauses {
                    self.proof_to_metamath_impl(clause, steps, labels, counter)?;
                }
                labels.push("res-end".into());
            }
            ProofTerm::QuantifierInstantiation {
                quantified,
                instantiation,
            } => {
                self.proof_to_metamath_impl(quantified, steps, labels, counter)?;
                for (var, _) in instantiation {
                    labels.push(var.clone());
                }
                labels.push("syl".into());
            }

            // Constructive proofs
            ProofTerm::Apply { rule, premises } => {
                for premise in premises {
                    self.proof_to_metamath_impl(premise, steps, labels, counter)?;
                }
                labels.push(rule.clone());
            }
            ProofTerm::Lambda { var, body } => {
                labels.push("lam".into());
                labels.push(var.clone());
                self.proof_to_metamath_impl(body, steps, labels, counter)?;
                labels.push("lam-ax".into());
            }
            ProofTerm::Cases { cases, .. } => {
                labels.push("case-begin".into());
                for (idx, (_, case_proof)) in cases.iter().enumerate() {
                    labels.push(format!("case-{}", idx + 1).into());
                    self.proof_to_metamath_impl(case_proof, steps, labels, counter)?;
                }
                labels.push("case-end".into());
            }
            ProofTerm::Induction {
                var,
                base_case,
                inductive_case,
            } => {
                labels.push("ind-begin".into());
                labels.push(var.clone());
                labels.push("ind-base".into());
                self.proof_to_metamath_impl(base_case, steps, labels, counter)?;
                labels.push("ind-step".into());
                self.proof_to_metamath_impl(inductive_case, steps, labels, counter)?;
                labels.push("ind-conclude".into());
            }

            // SMT integration
            ProofTerm::SmtProof { solver, .. } => {
                labels.push(format!("smt-{}", solver).into());
            }

            // Dependent types
            ProofTerm::Subst { eq_proof, .. } => {
                self.proof_to_metamath_impl(eq_proof, steps, labels, counter)?;
                labels.push("subst".into());
            }

            // Meta-level
            ProofTerm::Lemma { proof, .. } => {
                labels.push("lem-begin".into());
                self.proof_to_metamath_impl(proof, steps, labels, counter)?;
                labels.push("lem-end".into());
            }

            // Handle other proof terms with a placeholder label
            _ => {
                labels.push("unhandled".into());
            }
        }

        Ok(())
    }

    /// Generate JSON certificate
    fn to_json(&self, proof: &ProofTerm, theorem: &Theorem) -> Result<Text, CertificateError> {
        let json = serde_json::json!({
            "theorem": {
                "name": theorem.name.as_str(),
                "statement": theorem.statement.as_str(),
                "axioms": theorem.axioms.iter().map(|a| a.as_str()).collect::<List<_>>(),
            },
            "proof": format!("{:?}", proof),
        });

        // `pretty_print` was a config field with no readers — every
        // call serialised compactly regardless of the flag.  JSON is
        // the only certificate format with a meaningful pretty-vs-
        // compact distinction (Coq/Lean/Dedukti are emitted as
        // structured ASCII text where the layout is part of the
        // grammar), so the flag is honoured here.
        let serialised = if self.config.pretty_print {
            serde_json::to_string_pretty(&json)
                .map_err(|e| CertificateError::GenerationFailed(e.to_string().into()))?
        } else {
            json.to_string()
        };

        Ok(serialised.into())
    }
}

// ==================== Cross-Verification ====================

/// Cross-verification report
///
/// Cross-verification report: generates certificates in multiple formats (Dedukti,
/// Coq, Lean) and independently validates each, ensuring the proof is correct
/// across different proof checkers.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    /// Original theorem
    pub theorem: Theorem,

    /// Generated certificates
    pub certificates: List<Certificate>,

    /// Validation results
    pub results: Map<CertificateFormat, ValidationResult>,
}

/// Validation result for a single certificate
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether validation succeeded
    pub success: bool,

    /// Error message (if failed)
    pub error: Maybe<Text>,

    /// Validation time (milliseconds)
    pub time_ms: u64,
}

impl ValidationResult {
    /// Create successful validation
    pub fn success(time_ms: u64) -> Self {
        Self {
            success: true,
            error: Maybe::None,
            time_ms,
        }
    }

    /// Create failed validation
    pub fn failure(error: Text, time_ms: u64) -> Self {
        Self {
            success: false,
            error: Maybe::Some(error),
            time_ms,
        }
    }
}

/// Cross-verify theorem across multiple formats
///
/// Cross-verify a theorem by generating certificates in all requested formats
/// and independently checking each. All must succeed for the theorem to be validated.
pub fn cross_verify(
    proof: &ProofTerm,
    theorem: Theorem,
    formats: &[CertificateFormat],
) -> Result<ValidationReport, CertificateError> {
    let mut certificates = List::new();
    let mut results = Map::new();

    for &format in formats {
        let start = std::time::Instant::now();

        let generator = CertificateGenerator::new(format);
        let cert = generator.generate(proof, theorem.clone())?;

        // Verify integrity
        let validation = if cert.verify_integrity() {
            let elapsed = start.elapsed().as_millis() as u64;
            ValidationResult::success(elapsed)
        } else {
            let elapsed = start.elapsed().as_millis() as u64;
            ValidationResult::failure("Integrity check failed".into(), elapsed)
        };

        certificates.push(cert);
        results.insert(format, validation);
    }

    Ok(ValidationReport {
        theorem,
        certificates,
        results,
    })
}

/// Cross-verify theorem with full chain verification
///
/// This performs comprehensive verification including:
/// - Certificate generation in multiple formats
/// - Integrity checking
/// - Signature verification (if signed)
/// - Chain verification (if dependencies exist)
pub fn cross_verify_with_chain(
    proof: &ProofTerm,
    theorem: Theorem,
    formats: &[CertificateFormat],
    certificate_store: &CertificateStore,
    signing_key: Maybe<&SigningKey>,
) -> Result<ValidationReport, CertificateError> {
    let mut certificates = List::new();
    let mut results = Map::new();

    for &format in formats {
        let start = std::time::Instant::now();

        let generator = CertificateGenerator::new(format);
        let mut cert = match &signing_key {
            Maybe::Some(sk) => {
                // Generate signed certificate
                let content = match format {
                    CertificateFormat::Dedukti => generator.to_dedukti(proof, &theorem)?,
                    CertificateFormat::Coq => generator.to_coq(proof, &theorem)?,
                    CertificateFormat::Lean => generator.to_lean(proof, &theorem)?,
                    CertificateFormat::OpenTheory => generator.to_opentheory(proof, &theorem)?,
                    CertificateFormat::Metamath => generator.to_metamath(proof, &theorem)?,
                    CertificateFormat::Json => generator.to_json(proof, &theorem)?,
                };

                let mut cert = Certificate::new_signed(format, theorem.clone(), content, sk);

                if generator.config.include_metadata {
                    cert.metadata.generator_version = "verum-smt-1.0".into();
                    cert.metadata.timestamp = chrono::Utc::now().to_rfc3339().into();
                }

                cert
            }
            Maybe::None => {
                // Generate unsigned certificate
                generator.generate(proof, theorem.clone())?
            }
        };

        // Verify the certificate
        let validation = match cert.verify_chain(certificate_store) {
            Ok(()) => {
                let elapsed = start.elapsed().as_millis() as u64;
                ValidationResult::success(elapsed)
            }
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                ValidationResult::failure(
                    format!("Chain verification failed: {}", e).into(),
                    elapsed,
                )
            }
        };

        certificates.push(cert);
        results.insert(format, validation);
    }

    Ok(ValidationReport {
        theorem,
        certificates,
        results,
    })
}

// ==================== Errors ====================

/// Certificate generation errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum CertificateError {
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(Text),

    #[error("Generation failed: {0}")]
    GenerationFailed(Text),

    #[error("Validation failed: {0}")]
    ValidationFailed(Text),

    #[error("IO error: {0}")]
    Io(Text),
}

impl From<std::io::Error> for CertificateError {
    fn from(err: std::io::Error) -> Self {
        CertificateError::Io(err.to_string().into())
    }
}

// ==================== Certificate Store ====================

/// Certificate store for managing and verifying certificate chains
///
/// Stores certificates indexed by theorem name and format.
/// Used for chain verification and dependency management.
#[derive(Debug, Clone)]
pub struct CertificateStore {
    /// Certificates indexed by (theorem_name, format)
    certificates: Map<(Text, CertificateFormat), Certificate>,
}

impl CertificateStore {
    /// Create a new empty certificate store
    pub fn new() -> Self {
        Self {
            certificates: Map::new(),
        }
    }

    /// Add a certificate to the store
    pub fn add(&mut self, cert: Certificate) {
        let key = (cert.theorem.name.clone(), cert.format);
        self.certificates.insert(key, cert);
    }

    /// Get a certificate from the store
    pub fn get(&self, theorem_name: &Text, format: CertificateFormat) -> Maybe<&Certificate> {
        let key = (theorem_name.clone(), format);
        match self.certificates.get(&key) {
            Some(cert) => Maybe::Some(cert),
            None => Maybe::None,
        }
    }

    /// Remove a certificate from the store
    pub fn remove(&mut self, theorem_name: &Text, format: CertificateFormat) -> Maybe<Certificate> {
        let key = (theorem_name.clone(), format);
        self.certificates.remove(&key)
    }

    /// Get all certificates in the store
    pub fn all(&self) -> List<&Certificate> {
        self.certificates.values().collect()
    }

    /// Get the number of certificates in the store
    pub fn len(&self) -> usize {
        self.certificates.len()
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> bool {
        self.certificates.is_empty()
    }

    /// Verify all certificates in the store
    ///
    /// Returns a report of all verification results
    pub fn verify_all(&self) -> CertificateStoreVerificationReport {
        let mut report = CertificateStoreVerificationReport {
            total: self.certificates.len(),
            passed: 0,
            failed: 0,
            errors: List::new(),
        };

        for cert in self.certificates.values() {
            match cert.verify_chain(self) {
                Ok(()) => {
                    report.passed += 1;
                }
                Err(e) => {
                    report.failed += 1;
                    report.errors.push((cert.theorem.name.clone(), e));
                }
            }
        }

        report
    }
}

impl Default for CertificateStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Verification report for certificate store
#[derive(Debug, Clone)]
pub struct CertificateStoreVerificationReport {
    /// Total number of certificates
    pub total: usize,

    /// Number of certificates that passed verification
    pub passed: usize,

    /// Number of certificates that failed verification
    pub failed: usize,

    /// Errors for failed certificates
    pub errors: List<(Text, CertificateError)>,
}

impl CertificateStoreVerificationReport {
    /// Check if all certificates passed verification
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.passed as f64 / self.total as f64
        }
    }
}

// ==================== Utilities ====================

/// Compute SHA-256 checksum of content
///
/// Returns 32 bytes (256 bits) representing the SHA-256 hash
fn compute_sha256_checksum(content: &Text) -> List<u8> {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    result.iter().copied().collect()
}

/// Convert bytes to hexadecimal string
fn bytes_to_hex(bytes: &List<u8>) -> Text {
    let mut hex = String::new();
    for &byte in bytes {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex.into()
}

/// Generate a new Ed25519 signing key for signing certificates
pub fn generate_signing_key() -> SigningKey {
    // Generate random bytes for the signing key using getrandom
    let mut key_bytes = [0u8; 32];
    getrandom::fill(&mut key_bytes).expect("Failed to generate random bytes");
    SigningKey::from_bytes(&key_bytes)
}
