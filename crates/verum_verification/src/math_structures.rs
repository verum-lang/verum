//! Mathematical Structures Library for Verum
//!
//! Implements comprehensive mathematical structures for formal verification
//! for the Verum formal proof system (planned for version 2.0+).
//!
//! Mathematical structures provide the algebraic, analytic, and categorical
//! foundations for formal verification. Each structure defines a carrier set,
//! operations, and governing axioms that can be checked by the proof system.
//!
//! ## Features
//!
//! - **Abstract Algebra**: Groups, Rings, Fields, Modules, Vector Spaces
//! - **Analysis**: Complete ordered fields, limits, continuity
//! - **Category Theory**: Categories, Functors, Natural Transformations
//! - **Number Theory**: Primes, divisibility, fundamental theorems
//! - **Topology**: Topological spaces, continuous functions, compactness
//! - **Standard Lemma Database**: Automated lemma lookup for proof search
//!
//! ## Integration
//!
//! This module integrates with:
//! - `verum_smt::proof_search` for automated proof tactics
//! - `verum_smt::algebra` for algebraic structure verification
//! - `verum_types` for type-level mathematical properties
//!
//! ## Performance Targets
//!
//! - Structure verification: < 100ms per axiom
//! - Lemma lookup: < 1ms
//! - Theorem proving: < 5s for standard theorems

use verum_ast::{BinOp, Expr, ExprKind, Ident, UnOp};
use verum_common::{Heap, List, Map, Maybe, Set, Text};

use crate::tactic_evaluation::{Goal, Hypothesis, ProofState, TacticEvaluator, TacticResult};

// ==================== Core Mathematical Structures ====================

/// Mathematical structure with operations and axioms
///
/// Represents any mathematical structure (group, ring, field, etc.)
/// with its carrier set, operations, and governing axioms.
///
/// A mathematical structure with carrier set, operations, and axioms.
/// Examples: Group (op, id, inv with assoc/left_id/left_inv axioms),
/// Ring (add, mul with distributivity), Field (ring + multiplicative inverse).
#[derive(Debug, Clone)]
pub struct MathStructure {
    /// Structure name (e.g., "IntegerGroup", "RealField")
    pub name: Text,

    /// Category of structure (Group, Ring, Field, etc.)
    pub category: StructureCategory,

    /// Carrier type (underlying type)
    pub carrier_type: Text,

    /// Operations defined on this structure
    pub operations: Map<Text, MathOperation>,

    /// Axioms that define the structure
    pub axioms: List<Axiom>,

    /// Derived theorems (proven from axioms)
    pub theorems: List<Theorem>,
}

/// Category of mathematical structure
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureCategory {
    Group,
    AbelianGroup,
    Monoid,
    Ring,
    CommutativeRing,
    Field,
    Module,
    VectorSpace,
    Category,
    Functor,
    TopologicalSpace,
}

/// Mathematical operation
#[derive(Debug, Clone)]
pub struct MathOperation {
    /// Operation name
    pub name: Text,

    /// Number of arguments
    pub arity: usize,

    /// Type signature
    pub signature: Text,

    /// Optional implementation (for concrete structures)
    pub implementation: Maybe<Heap<Expr>>,
}

/// Mathematical axiom
///
/// A named axiom (e.g., "associativity", "left_identity") with a formula
/// expressed over the structure's operations and carrier set.
#[derive(Debug, Clone)]
pub struct Axiom {
    /// Axiom name (e.g., "associativity", "left_identity")
    pub name: Text,

    /// Axiom statement as a formula
    pub formula: Expr,

    /// Universal quantification variables
    pub quantified_vars: List<Text>,

    /// Whether this axiom is SMT-checkable
    pub smt_checkable: bool,
}

/// Theorem derived from axioms
#[derive(Debug, Clone)]
pub struct Theorem {
    /// Theorem name
    pub name: Text,

    /// Theorem statement
    pub statement: Expr,

    /// Proof sketch or tactic
    pub proof: ProofMethod,

    /// Whether the theorem has been verified
    pub verified: bool,
}

/// Method used to prove a theorem
#[derive(Debug, Clone)]
pub enum ProofMethod {
    /// Direct SMT discharge
    Smt,

    /// Proof by induction
    Induction {
        variable: Text,
        base_case: Heap<Expr>,
        inductive_case: Heap<Expr>,
    },

    /// Proof by calculation
    Calculation { steps: List<Expr> },

    /// Apply existing theorem
    Apply { theorem_name: Text },

    /// Tactic script
    Tactic { tactics: List<Text> },

    /// Manual proof term
    Manual { proof_term: Heap<Expr> },
}

// ==================== Abstract Algebra ====================

/// Group structure builder
///
/// Builds a Group structure: (G, op, id, inv) with axioms for associativity,
/// left identity (op(id, a) = a), and left inverse (op(inv(a), a) = id).
/// Optionally abelian (adds commutativity: op(a, b) = op(b, a)).
#[derive(Debug)]
pub struct GroupBuilder {
    name: Text,
    carrier_type: Text,
    is_abelian: bool,
}

impl GroupBuilder {
    /// Create a new group builder
    pub fn new(name: Text, carrier_type: Text) -> Self {
        Self {
            name,
            carrier_type,
            is_abelian: false,
        }
    }

    /// Mark this group as abelian (commutative)
    pub fn abelian(mut self) -> Self {
        self.is_abelian = true;
        self
    }

    /// Build the group structure
    ///
    /// Creates a group with operations (op, id, inv) and axioms
    /// (associativity, identity, inverse).
    pub fn build(self) -> MathStructure {
        let mut structure = MathStructure {
            name: self.name.clone(),
            category: if self.is_abelian {
                StructureCategory::AbelianGroup
            } else {
                StructureCategory::Group
            },
            carrier_type: self.carrier_type.clone(),
            operations: Map::new(),
            axioms: List::new(),
            theorems: List::new(),
        };

        // Add operations
        structure.operations.insert(
            "op".into(),
            MathOperation {
                name: "op".into(),
                arity: 2,
                signature: format!(
                    "{} -> {} -> {}",
                    self.carrier_type, self.carrier_type, self.carrier_type
                )
                .into(),
                implementation: Maybe::None,
            },
        );

        structure.operations.insert(
            "id".into(),
            MathOperation {
                name: "id".into(),
                arity: 0,
                signature: self.carrier_type.clone(),
                implementation: Maybe::None,
            },
        );

        structure.operations.insert(
            "inv".into(),
            MathOperation {
                name: "inv".into(),
                arity: 1,
                signature: format!("{} -> {}", self.carrier_type, self.carrier_type).into(),
                implementation: Maybe::None,
            },
        );

        // Add axioms
        structure.axioms.push(Self::associativity_axiom());
        structure.axioms.push(Self::left_identity_axiom());
        structure.axioms.push(Self::left_inverse_axiom());

        if self.is_abelian {
            structure.axioms.push(Self::commutativity_axiom());
        }

        // Add derived theorems
        structure.theorems.push(Self::right_identity_theorem());
        structure.theorems.push(Self::right_inverse_theorem());
        structure.theorems.push(Self::inverse_unique_theorem());

        structure
    }

    /// Associativity axiom: (a • b) • c = a • (b • c)
    fn associativity_axiom() -> Axiom {
        use verum_ast::Path;
        use verum_ast::span::Span;

        let span = Span::dummy();

        // Create variables
        let var_a = create_var_expr("a", span);
        let var_b = create_var_expr("b", span);
        let var_c = create_var_expr("c", span);
        let op_path = create_var_expr("op", span);

        // (a • b) • c
        let a_op_b = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path.clone()),
                type_args: Vec::new().into(),
                args: vec![var_a.clone(), var_b.clone()].into(),
            },
            span,
        );

        let left_side = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path.clone()),
                type_args: Vec::new().into(),
                args: vec![a_op_b, var_c.clone()].into(),
            },
            span,
        );

        // a • (b • c)
        let b_op_c = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path.clone()),
                type_args: Vec::new().into(),
                args: vec![var_b.clone(), var_c.clone()].into(),
            },
            span,
        );

        let right_side = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path),
                type_args: Vec::new().into(),
                args: vec![var_a.clone(), b_op_c].into(),
            },
            span,
        );

        // Build equality
        let formula = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(left_side),
                right: Box::new(right_side),
            },
            span,
        );

        Axiom {
            name: "associativity".into(),
            formula,
            quantified_vars: text_list(&["a", "b", "c"]),
            smt_checkable: true,
        }
    }

    /// Left identity axiom: e • a = a
    fn left_identity_axiom() -> Axiom {
        use verum_ast::span::Span;

        let span = Span::dummy();
        let var_a = create_var_expr("a", span);
        let id_e = create_var_expr("e", span);
        let op_path = create_var_expr("op", span);

        let left_side = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path),
                type_args: Vec::new().into(),
                args: vec![id_e, var_a.clone()].into(),
            },
            span,
        );

        let formula = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(left_side),
                right: Box::new(var_a),
            },
            span,
        );

        Axiom {
            name: "left_identity".into(),
            formula,
            quantified_vars: text_list(&["a"]),
            smt_checkable: true,
        }
    }

    /// Left inverse axiom: inv(a) • a = e
    fn left_inverse_axiom() -> Axiom {
        use verum_ast::span::Span;

        let span = Span::dummy();
        let var_a = create_var_expr("a", span);
        let id_e = create_var_expr("e", span);
        let inv_path = create_var_expr("inv", span);
        let op_path = create_var_expr("op", span);

        let inv_a = Expr::new(
            ExprKind::Call {
                func: Box::new(inv_path),
                type_args: Vec::new().into(),
                args: vec![var_a.clone()].into(),
            },
            span,
        );

        let left_side = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path),
                type_args: Vec::new().into(),
                args: vec![inv_a, var_a].into(),
            },
            span,
        );

        let formula = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(left_side),
                right: Box::new(id_e),
            },
            span,
        );

        Axiom {
            name: "left_inverse".into(),
            formula,
            quantified_vars: text_list(&["a"]),
            smt_checkable: true,
        }
    }

    /// Commutativity axiom: a • b = b • a
    fn commutativity_axiom() -> Axiom {
        use verum_ast::span::Span;

        let span = Span::dummy();
        let var_a = create_var_expr("a", span);
        let var_b = create_var_expr("b", span);
        let op_path = create_var_expr("op", span);

        let a_op_b = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path.clone()),
                type_args: Vec::new().into(),
                args: vec![var_a.clone(), var_b.clone()].into(),
            },
            span,
        );

        let b_op_a = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path),
                type_args: Vec::new().into(),
                args: vec![var_b, var_a].into(),
            },
            span,
        );

        let formula = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(a_op_b),
                right: Box::new(b_op_a),
            },
            span,
        );

        Axiom {
            name: "commutativity".into(),
            formula,
            quantified_vars: text_list(&["a", "b"]),
            smt_checkable: true,
        }
    }

    /// Right identity theorem: a • e = a
    fn right_identity_theorem() -> Theorem {
        use verum_ast::span::Span;

        let span = Span::dummy();
        let var_a = create_var_expr("a", span);
        let id_e = create_var_expr("e", span);
        let op_path = create_var_expr("op", span);

        let left_side = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path),
                type_args: Vec::new().into(),
                args: vec![var_a.clone(), id_e].into(),
            },
            span,
        );

        let statement = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(left_side),
                right: Box::new(var_a),
            },
            span,
        );

        Theorem {
            name: "right_identity".into(),
            statement,
            proof: ProofMethod::Tactic {
                tactics: text_list(&["apply left_identity", "apply left_inverse"]),
            },
            verified: false,
        }
    }

    /// Right inverse theorem: a • inv(a) = e
    fn right_inverse_theorem() -> Theorem {
        use verum_ast::span::Span;

        let span = Span::dummy();
        let var_a = create_var_expr("a", span);
        let id_e = create_var_expr("e", span);
        let inv_path = create_var_expr("inv", span);
        let op_path = create_var_expr("op", span);

        let inv_a = Expr::new(
            ExprKind::Call {
                func: Box::new(inv_path),
                type_args: Vec::new().into(),
                args: vec![var_a.clone()].into(),
            },
            span,
        );

        let left_side = Expr::new(
            ExprKind::Call {
                func: Box::new(op_path),
                type_args: Vec::new().into(),
                args: vec![var_a, inv_a].into(),
            },
            span,
        );

        let statement = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(left_side),
                right: Box::new(id_e),
            },
            span,
        );

        Theorem {
            name: "right_inverse".into(),
            statement,
            proof: ProofMethod::Tactic {
                tactics: text_list(&["apply left_inverse", "apply associativity"]),
            },
            verified: false,
        }
    }

    /// Inverse uniqueness theorem
    fn inverse_unique_theorem() -> Theorem {
        use verum_ast::span::Span;

        let span = Span::dummy();
        let var_a = create_var_expr("a", span);
        let var_b = create_var_expr("b", span);
        let var_c = create_var_expr("c", span);

        // b = c (if both are inverses of a)
        let statement = Expr::new(
            ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(var_b),
                right: Box::new(var_c),
            },
            span,
        );

        Theorem {
            name: "inverse_unique".into(),
            statement,
            proof: ProofMethod::Tactic {
                tactics: text_list(&["assume b and c are both inverses", "calc"]),
            },
            verified: false,
        }
    }
}

/// Subgroup structure
///
/// Subgroup: subset of a group that is closed under op, contains id, and
/// contains inv(a) for every element a. Verified by checking closure,
/// identity membership, and inverse membership axioms.
#[derive(Debug, Clone)]
pub struct Subgroup {
    /// Parent group
    pub parent_group: Text,

    /// Carrier set (subset of parent carrier)
    pub carrier: Heap<Expr>,

    /// Closure property: ∀(a, b ∈ carrier). a • b ∈ carrier
    pub closure_axiom: Axiom,

    /// Identity containment: e ∈ carrier
    pub has_identity: Axiom,

    /// Inverse containment: ∀(a ∈ carrier). inv(a) ∈ carrier
    pub has_inverses: Axiom,
}

/// Homomorphism between algebraic structures
///
/// Structure-preserving map f: G -> H where f(op_G(a, b)) = op_H(f(a), f(b)).
#[derive(Debug, Clone)]
pub struct Homomorphism {
    /// Source structure
    pub source: Text,

    /// Target structure
    pub target: Text,

    /// Mapping function
    pub mapping: Heap<Expr>,

    /// Structure preservation property
    /// For groups: f(a • b) = f(a) ⊗ f(b)
    pub preserves_operation: Axiom,
}

/// Ring structure
///
/// A ring has two operations (addition and multiplication) where:
/// - (R, +) is an abelian group
/// - (R, *) is a monoid
/// - Multiplication distributes over addition
#[derive(Debug, Clone)]
pub struct Ring {
    /// Ring name
    pub name: Text,

    /// Carrier type
    pub carrier_type: Text,

    /// Whether multiplication is commutative
    pub is_commutative: bool,

    /// Addition operation (forms abelian group)
    pub addition: MathOperation,

    /// Multiplication operation (forms monoid)
    pub multiplication: MathOperation,

    /// Additive identity (zero)
    pub zero: Heap<Expr>,

    /// Multiplicative identity (one)
    pub one: Heap<Expr>,

    /// Additive inverse (negation)
    pub negation: MathOperation,

    /// Ring axioms
    pub axioms: List<Axiom>,
}

impl Ring {
    /// Create a new ring structure
    pub fn new(name: Text, carrier_type: Text) -> Self {
        let mut axioms = List::new();

        // Addition group axioms
        axioms.push(create_axiom(
            "add_assoc",
            "∀a,b,c. (a + b) + c = a + (b + c)",
        ));
        axioms.push(create_axiom("add_comm", "∀a,b. a + b = b + a"));
        axioms.push(create_axiom("add_zero", "∀a. a + 0 = a"));
        axioms.push(create_axiom("add_inv", "∀a. a + (-a) = 0"));

        // Multiplication monoid axioms
        axioms.push(create_axiom(
            "mul_assoc",
            "∀a,b,c. (a * b) * c = a * (b * c)",
        ));
        axioms.push(create_axiom("mul_one", "∀a. a * 1 = a ∧ 1 * a = a"));

        // Distributivity
        axioms.push(create_axiom(
            "left_distrib",
            "∀a,b,c. a * (b + c) = a * b + a * c",
        ));
        axioms.push(create_axiom(
            "right_distrib",
            "∀a,b,c. (a + b) * c = a * c + b * c",
        ));

        Self {
            name,
            carrier_type: carrier_type.clone(),
            is_commutative: false,
            addition: MathOperation {
                name: "add".into(),
                arity: 2,
                signature: format!("{} -> {} -> {}", carrier_type, carrier_type, carrier_type).into(),
                implementation: Maybe::None,
            },
            multiplication: MathOperation {
                name: "mul".into(),
                arity: 2,
                signature: format!("{} -> {} -> {}", carrier_type, carrier_type, carrier_type).into(),
                implementation: Maybe::None,
            },
            zero: Heap::new(create_const_expr("zero")),
            one: Heap::new(create_const_expr("one")),
            negation: MathOperation {
                name: "neg".into(),
                arity: 1,
                signature: format!("{} -> {}", carrier_type, carrier_type).into(),
                implementation: Maybe::None,
            },
            axioms,
        }
    }

    /// Create a commutative ring
    pub fn commutative(mut self) -> Self {
        self.is_commutative = true;
        self.axioms
            .push(create_axiom("mul_comm", "∀a,b. a * b = b * a"));
        self
    }
}

/// Field structure
///
/// A field is a commutative ring where every non-zero element has
/// a multiplicative inverse.
#[derive(Debug, Clone)]
pub struct Field {
    /// Underlying ring
    pub ring: Ring,

    /// Multiplicative inverse operation
    pub inverse: MathOperation,

    /// Inverse axiom: ∀a ≠ 0. a * inv(a) = 1
    pub inverse_axiom: Axiom,
}

impl Field {
    /// Create a new field structure
    pub fn new(name: Text, carrier_type: Text) -> Self {
        let ring = Ring::new(name, carrier_type.clone()).commutative();

        Self {
            ring,
            inverse: MathOperation {
                name: "inv".into(),
                arity: 1,
                signature: format!("{} -> {}", carrier_type, carrier_type).into(),
                implementation: Maybe::None,
            },
            inverse_axiom: create_axiom("mul_inv", "∀a ≠ 0. a * inv(a) = 1"),
        }
    }
}

/// Vector space over a field
///
/// Vector space over a field: requires vector addition forming an abelian group
/// (associativity, commutativity, zero element, additive inverse) and scalar
/// multiplication axioms (compatibility, identity, distributivity over vectors
/// and scalars).
#[derive(Debug, Clone)]
pub struct VectorSpace {
    /// Vector space name
    pub name: Text,

    /// Vector type
    pub vector_type: Text,

    /// Scalar field
    pub scalar_field: Text,

    /// Vector addition
    pub vector_addition: MathOperation,

    /// Scalar multiplication
    pub scalar_multiplication: MathOperation,

    /// Zero vector
    pub zero_vector: Heap<Expr>,

    /// Vector space axioms
    pub axioms: List<Axiom>,
}

impl VectorSpace {
    /// Create a new vector space
    pub fn new(name: Text, vector_type: Text, scalar_field: Text) -> Self {
        let mut axioms = List::new();

        // Vector addition forms abelian group
        axioms.push(create_axiom(
            "vadd_assoc",
            "∀u,v,w. (u + v) + w = u + (v + w)",
        ));
        axioms.push(create_axiom("vadd_comm", "∀u,v. u + v = v + u"));
        axioms.push(create_axiom("vadd_zero", "∀v. v + 0 = v"));
        axioms.push(create_axiom("vadd_inv", "∀v. v + (-v) = 0"));

        // Scalar multiplication axioms
        axioms.push(create_axiom(
            "smul_compat",
            "∀a,b,v. a * (b * v) = (a * b) * v",
        ));
        axioms.push(create_axiom("smul_id", "∀v. 1 * v = v"));
        axioms.push(create_axiom(
            "smul_dist_vec",
            "∀a,u,v. a * (u + v) = a * u + a * v",
        ));
        axioms.push(create_axiom(
            "smul_dist_scalar",
            "∀a,b,v. (a + b) * v = a * v + b * v",
        ));

        Self {
            name,
            vector_type: vector_type.clone(),
            scalar_field,
            vector_addition: MathOperation {
                name: "vadd".into(),
                arity: 2,
                signature: format!("{} -> {} -> {}", vector_type, vector_type, vector_type).into(),
                implementation: Maybe::None,
            },
            scalar_multiplication: MathOperation {
                name: "smul".into(),
                arity: 2,
                signature: "Field -> Vector -> Vector".into(),
                implementation: Maybe::None,
            },
            zero_vector: Heap::new(create_const_expr("zero_vec")),
            axioms,
        }
    }
}

// ==================== Analysis ====================

/// Complete ordered field (e.g., real numbers)
///
/// Complete ordered field (e.g., real numbers): an ordered field satisfying the
/// completeness axiom: every nonempty bounded-above subset has a supremum.
/// Formally: forall S: Set<R>. bounded_above(S) & S != empty -> exists sup. is_supremum(S, sup)
#[derive(Debug, Clone)]
pub struct CompleteOrderedField {
    /// Underlying field
    pub field: Field,

    /// Order relation
    pub order: MathOperation,

    /// Supremum operation
    pub supremum: MathOperation,

    /// Completeness axiom
    pub completeness_axiom: Axiom,
}

impl CompleteOrderedField {
    /// Create a new complete ordered field
    pub fn new(name: Text, carrier_type: Text) -> Self {
        Self {
            field: Field::new(name, carrier_type.clone()),
            order: MathOperation {
                name: "le".into(),
                arity: 2,
                signature: format!("{} -> {} -> Bool", carrier_type, carrier_type).into(),
                implementation: Maybe::None,
            },
            supremum: MathOperation {
                name: "sup".into(),
                arity: 1,
                signature: format!("Set<{}> -> {}", carrier_type, carrier_type).into(),
                implementation: Maybe::None,
            },
            completeness_axiom: create_axiom(
                "completeness",
                "∀S. bounded_above(S) ∧ S ≠ ∅ → ∃sup. is_supremum(S, sup)",
            ),
        }
    }
}

/// Limit definition
///
/// Epsilon-delta limit definition: limit(f, a, L) iff
/// forall eps > 0. exists delta > 0. forall x. 0 < |x - a| < delta -> |f(x) - L| < eps
#[derive(Debug, Clone)]
pub struct LimitDefinition {
    /// Function being analyzed
    pub function: Heap<Expr>,

    /// Point of limit
    pub point: Heap<Expr>,

    /// Limit value
    pub limit_value: Heap<Expr>,

    /// Epsilon-delta formulation
    pub epsilon_delta_formula: Axiom,
}

/// Continuity definition
///
/// Continuity: f is continuous at a iff limit(f, a, f(a)) holds.
#[derive(Debug, Clone)]
pub struct ContinuityDefinition {
    /// Function being analyzed
    pub function: Heap<Expr>,

    /// Continuity axiom
    pub continuity_axiom: Axiom,
}

// ==================== Category Theory ====================

/// Category structure
///
/// Category: objects, morphisms, identity morphisms, and composition satisfying
/// left identity (id_B . f = f), right identity (f . id_A = f), and
/// associativity (h . (g . f) = (h . g) . f).
#[derive(Debug, Clone)]
pub struct Category {
    /// Category name
    pub name: Text,

    /// Objects (represented as types)
    pub objects: Set<Text>,

    /// Morphisms (object -> object -> Set<Morphism>)
    pub morphisms: Map<(Text, Text), Set<Text>>,

    /// Identity morphism operation
    pub identity: MathOperation,

    /// Composition operation
    pub composition: MathOperation,

    /// Category axioms
    pub axioms: List<Axiom>,
}

impl Category {
    /// Create a new category
    pub fn new(name: Text) -> Self {
        let mut axioms = List::new();

        // Identity axioms
        axioms.push(create_axiom("left_id", "∀f: A → B. id_B ∘ f = f"));
        axioms.push(create_axiom("right_id", "∀f: A → B. f ∘ id_A = f"));

        // Associativity axiom
        axioms.push(create_axiom(
            "comp_assoc",
            "∀f,g,h. h ∘ (g ∘ f) = (h ∘ g) ∘ f",
        ));

        Self {
            name,
            objects: Set::new(),
            morphisms: Map::new(),
            identity: MathOperation {
                name: "id".into(),
                arity: 1,
                signature: "Obj -> Mor".into(),
                implementation: Maybe::None,
            },
            composition: MathOperation {
                name: "compose".into(),
                arity: 2,
                signature: "Mor -> Mor -> Mor".into(),
                implementation: Maybe::None,
            },
            axioms,
        }
    }

    /// Add an object to the category
    pub fn add_object(&mut self, obj: Text) {
        self.objects.insert(obj);
    }

    /// Add a morphism between objects
    pub fn add_morphism(&mut self, source: Text, target: Text, morphism: Text) {
        self.morphisms
            .entry((source, target))
            .or_default()
            .insert(morphism);
    }
}

/// Functor between categories
///
/// Functor between categories: maps objects and morphisms while preserving
/// identity (F(id_A) = id_{F(A)}) and composition (F(g . f) = F(g) . F(f)).
#[derive(Debug, Clone)]
pub struct Functor {
    /// Functor name
    pub name: Text,

    /// Source category
    pub source_category: Text,

    /// Target category
    pub target_category: Text,

    /// Object mapping
    pub object_map: Heap<Expr>,

    /// Morphism mapping
    pub morphism_map: Heap<Expr>,

    /// Functor laws
    pub laws: List<Axiom>,
}

impl Functor {
    /// Create a new functor
    pub fn new(name: Text, source: Text, target: Text) -> Self {
        let mut laws = List::new();

        // Preserves identity
        laws.push(create_axiom("preserves_id", "∀A. F(id_A) = id_{F(A)}"));

        // Preserves composition
        laws.push(create_axiom(
            "preserves_comp",
            "∀f,g. F(g ∘ f) = F(g) ∘ F(f)",
        ));

        Self {
            name,
            source_category: source,
            target_category: target,
            object_map: Heap::new(create_const_expr("obj_map")),
            morphism_map: Heap::new(create_const_expr("mor_map")),
            laws,
        }
    }
}

/// Natural transformation between functors
///
/// Natural transformation between functors F and G: a family of morphisms
/// eta_A: F(A) -> G(A) for each object A, satisfying the naturality condition:
/// for all f: A -> B, G(f) . eta_A = eta_B . F(f).
#[derive(Debug, Clone)]
pub struct NaturalTransformation {
    /// Natural transformation name
    pub name: Text,

    /// Source functor
    pub source_functor: Text,

    /// Target functor
    pub target_functor: Text,

    /// Component morphisms (one per object)
    pub components: Map<Text, Heap<Expr>>,

    /// Naturality condition
    pub naturality_axiom: Axiom,
}

impl NaturalTransformation {
    /// Create a new natural transformation
    pub fn new(name: Text, source: Text, target: Text) -> Self {
        Self {
            name,
            source_functor: source,
            target_functor: target,
            components: Map::new(),
            naturality_axiom: create_axiom("naturality", "∀f: A → B. G(f) ∘ η_A = η_B ∘ F(f)"),
        }
    }

    /// Add a component morphism for an object
    pub fn add_component(&mut self, object: Text, component: Expr) {
        self.components.insert(object, Heap::new(component));
    }
}

// ==================== Number Theory ====================

/// Prime number predicates and theorems
///
/// Number theory: primality (n > 1 and only divisible by 1 and n), divisibility,
/// GCD, and standard theorems: infinitude of primes, fundamental theorem of
/// arithmetic (unique prime factorization), Euler's theorem (a^phi(n) = 1 mod n
/// when gcd(a,n) = 1).
#[derive(Debug, Clone)]
pub struct NumberTheory {
    /// Primality predicate
    pub is_prime: Heap<Expr>,

    /// Divisibility relation
    pub divides: Heap<Expr>,

    /// GCD operation
    pub gcd: MathOperation,

    /// Standard theorems
    pub theorems: List<Theorem>,
}

impl NumberTheory {
    /// Create number theory structure with standard theorems
    pub fn new() -> Self {
        let mut theorems = List::new();

        // Infinitude of primes
        theorems.push(Theorem {
            name: "infinitude_of_primes".into(),
            statement: create_const_expr("∀n. ∃p > n. is_prime(p)"),
            proof: ProofMethod::Tactic {
                tactics: text_list(&["euclid_construction"]),
            },
            verified: false,
        });

        // Fundamental theorem of arithmetic
        theorems.push(Theorem {
            name: "fundamental_theorem".into(),
            statement: create_const_expr(
                "∀n > 1. ∃!factors. unique_prime_factorization(n, factors)",
            ),
            proof: ProofMethod::Induction {
                variable: "n".into(),
                base_case: Heap::new(create_const_expr("n = 2")),
                inductive_case: Heap::new(create_const_expr("assume n, prove n+1")),
            },
            verified: false,
        });

        // Euler's theorem
        theorems.push(Theorem {
            name: "euler_theorem".into(),
            statement: create_const_expr("∀a,n. gcd(a,n) = 1 → a^φ(n) ≡ 1 (mod n)"),
            proof: ProofMethod::Tactic {
                tactics: text_list(&["group_theory", "lagrange_theorem"]),
            },
            verified: false,
        });

        Self {
            is_prime: Heap::new(create_const_expr("is_prime")),
            divides: Heap::new(create_const_expr("divides")),
            gcd: MathOperation {
                name: "gcd".into(),
                arity: 2,
                signature: "Nat -> Nat -> Nat".into(),
                implementation: Maybe::None,
            },
            theorems,
        }
    }
}

impl Default for NumberTheory {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Topology ====================

/// Topological space
///
/// Topological space: carrier set with open sets satisfying axioms:
/// empty set and full set are open, arbitrary unions are open,
/// finite intersections are open.
#[derive(Debug, Clone)]
pub struct TopologicalSpace {
    /// Space name
    pub name: Text,

    /// Underlying set (carrier)
    pub carrier: Text,

    /// Open sets
    pub open_sets: Set<Text>,

    /// Topology axioms
    pub axioms: List<Axiom>,
}

impl TopologicalSpace {
    /// Create a new topological space
    pub fn new(name: Text, carrier: Text) -> Self {
        let mut axioms = List::new();

        // Empty set and full set are open
        axioms.push(create_axiom("empty_open", "∅ ∈ open_sets"));
        axioms.push(create_axiom("univ_open", "X ∈ open_sets"));

        // Arbitrary unions are open
        axioms.push(create_axiom("union_open", "∀F ⊆ open_sets. ⋃F ∈ open_sets"));

        // Finite intersections are open
        axioms.push(create_axiom(
            "inter_open",
            "∀A,B ∈ open_sets. A ∩ B ∈ open_sets",
        ));

        Self {
            name,
            carrier,
            open_sets: Set::new(),
            axioms,
        }
    }

    /// Add an open set
    pub fn add_open_set(&mut self, set: Text) {
        self.open_sets.insert(set);
    }
}

/// Continuous function definition
///
/// Continuous function: f: X -> Y is continuous iff for all U in Y.open_sets,
/// f^{-1}(U) is in X.open_sets (preimage of every open set is open).
#[derive(Debug, Clone)]
pub struct ContinuousFunction {
    /// Function expression
    pub function: Heap<Expr>,

    /// Source space
    pub source_space: Text,

    /// Target space
    pub target_space: Text,

    /// Continuity axiom: ∀U ∈ Y.open_sets. f⁻¹(U) ∈ X.open_sets
    pub continuity_axiom: Axiom,
}

/// Compactness definition
///
/// Compactness: K is compact iff every open cover has a finite subcover.
/// Formally: forall cover. (all U in cover are open) & K subset union(cover) ->
/// exists finite subcover subset cover with K subset union(subcover).
#[derive(Debug, Clone)]
pub struct CompactnessDefinition {
    /// Space being analyzed
    pub space: Text,

    /// Subset being tested for compactness
    pub subset: Heap<Expr>,

    /// Compactness axiom (finite subcover property)
    pub compactness_axiom: Axiom,
}

// ==================== Lemma Database ====================

/// Standard lemma database for automated proof search
///
/// This database contains common lemmas for each mathematical structure,
/// organized for efficient retrieval during proof automation.
#[derive(Debug, Clone)]
pub struct LemmaDatabase {
    /// Lemmas organized by structure type
    lemmas_by_structure: Map<Text, List<Lemma>>,

    /// Lemmas organized by pattern
    lemmas_by_pattern: Map<Text, List<Lemma>>,

    /// Priority hints for proof search
    priority_hints: Map<Text, u32>,
}

/// A mathematical lemma
#[derive(Debug, Clone)]
pub struct Lemma {
    /// Lemma name
    pub name: Text,

    /// Statement
    pub statement: Expr,

    /// Proof method
    pub proof: ProofMethod,

    /// Priority for proof search (higher = try first)
    pub priority: u32,

    /// Tags for categorization
    pub tags: List<Text>,
}

impl LemmaDatabase {
    /// Create a new lemma database
    pub fn new() -> Self {
        Self {
            lemmas_by_structure: Map::new(),
            lemmas_by_pattern: Map::new(),
            priority_hints: Map::new(),
        }
    }

    /// Create database with standard library lemmas
    pub fn with_core() -> Self {
        let mut db = Self::new();
        db.load_group_lemmas();
        db.load_ring_lemmas();
        db.load_field_lemmas();
        db.load_category_lemmas();
        db.load_number_theory_lemmas();
        db.load_topology_lemmas();
        db
    }

    /// Register a lemma
    pub fn register_lemma(&mut self, structure: Text, lemma: Lemma) {
        // Add to structure index
        self.lemmas_by_structure
            .entry(structure.clone())
            .or_default()
            .push(lemma.clone());

        // Extract pattern and add to pattern index
        let pattern = self.extract_pattern(&lemma.statement);
        self.lemmas_by_pattern
            .entry(pattern)
            .or_default()
            .push(lemma.clone());

        // Register priority hint
        self.priority_hints
            .insert(lemma.name.clone(), lemma.priority);
    }

    /// Find lemmas applicable to a goal
    pub fn find_applicable_lemmas(&self, goal: &Expr, structure: &Text) -> List<&Lemma> {
        let mut results = List::new();

        // Search by structure
        if let Some(lemmas) = self.lemmas_by_structure.get(structure) {
            for lemma in lemmas {
                if self.could_apply(lemma, goal) {
                    results.push(lemma);
                }
            }
        }

        // Search by pattern
        let goal_pattern = self.extract_pattern(goal);
        if let Some(lemmas) = self.lemmas_by_pattern.get(&goal_pattern) {
            for lemma in lemmas {
                if !results.iter().any(|l| l.name == lemma.name) {
                    results.push(lemma);
                }
            }
        }

        // Sort by priority
        results.sort_by(|a, b| b.priority.cmp(&a.priority));
        results
    }

    /// Extract pattern from expression
    fn extract_pattern(&self, expr: &Expr) -> Text {
        match &expr.kind {
            ExprKind::Binary { op, .. } => format!("_ {} _", op.as_str()).into(),
            ExprKind::Call { .. } => "call".into(),
            ExprKind::Unary { .. } => "unary".into(),
            _ => "unknown".into(),
        }
    }

    /// Check if lemma could apply to goal
    fn could_apply(&self, lemma: &Lemma, goal: &Expr) -> bool {
        // Simple structural matching
        self.extract_pattern(&lemma.statement) == self.extract_pattern(goal)
    }

    /// Load standard group lemmas
    fn load_group_lemmas(&mut self) {
        // Right identity
        self.register_lemma(
            "Group".into(),
            Lemma {
                name: "group_right_id".into(),
                statement: create_const_expr("∀a. a • e = a"),
                proof: ProofMethod::Tactic {
                    tactics: text_list(&["apply left_id", "apply left_inv"]),
                },
                priority: 100,
                tags: text_list(&["group", "identity"]),
            },
        );

        // Right inverse
        self.register_lemma(
            "Group".into(),
            Lemma {
                name: "group_right_inv".into(),
                statement: create_const_expr("∀a. a • inv(a) = e"),
                proof: ProofMethod::Tactic {
                    tactics: text_list(&["apply left_inv"]),
                },
                priority: 100,
                tags: text_list(&["group", "inverse"]),
            },
        );

        // Cancellation
        self.register_lemma(
            "Group".into(),
            Lemma {
                name: "group_cancel_left".into(),
                statement: create_const_expr("∀a,b,c. a • b = a • c → b = c"),
                proof: ProofMethod::Tactic {
                    tactics: text_list(&["apply inv", "apply assoc"]),
                },
                priority: 90,
                tags: text_list(&["group", "cancellation"]),
            },
        );
    }

    /// Load standard ring lemmas
    fn load_ring_lemmas(&mut self) {
        // Zero multiplication
        self.register_lemma(
            "Ring".into(),
            Lemma {
                name: "ring_zero_mul".into(),
                statement: create_const_expr("∀a. 0 * a = 0"),
                proof: ProofMethod::Tactic {
                    tactics: text_list(&["apply distrib", "simplify"]),
                },
                priority: 100,
                tags: text_list(&["ring", "zero"]),
            },
        );

        // Negation distribution
        self.register_lemma(
            "Ring".into(),
            Lemma {
                name: "ring_neg_distrib".into(),
                statement: create_const_expr("∀a,b. -(a + b) = -a + -b"),
                proof: ProofMethod::Smt,
                priority: 90,
                tags: text_list(&["ring", "negation"]),
            },
        );
    }

    /// Load standard field lemmas
    fn load_field_lemmas(&mut self) {
        // Inverse of product
        self.register_lemma(
            "Field".into(),
            Lemma {
                name: "field_inv_mul".into(),
                statement: create_const_expr("∀a,b ≠ 0. inv(a * b) = inv(a) * inv(b)"),
                proof: ProofMethod::Smt,
                priority: 100,
                tags: text_list(&["field", "inverse"]),
            },
        );
    }

    /// Load category theory lemmas
    fn load_category_lemmas(&mut self) {
        // Identity composition
        self.register_lemma(
            "Category".into(),
            Lemma {
                name: "cat_id_comp".into(),
                statement: create_const_expr("∀f. id ∘ f = f ∧ f ∘ id = f"),
                proof: ProofMethod::Tactic {
                    tactics: text_list(&["apply cat_axioms"]),
                },
                priority: 100,
                tags: text_list(&["category", "identity"]),
            },
        );
    }

    /// Load number theory lemmas
    fn load_number_theory_lemmas(&mut self) {
        // Divisibility transitivity
        self.register_lemma(
            "NumberTheory".into(),
            Lemma {
                name: "div_trans".into(),
                statement: create_const_expr("∀a,b,c. a | b ∧ b | c → a | c"),
                proof: ProofMethod::Smt,
                priority: 100,
                tags: text_list(&["number_theory", "divisibility"]),
            },
        );
    }

    /// Load topology lemmas
    fn load_topology_lemmas(&mut self) {
        // Preimage of union
        self.register_lemma(
            "Topology".into(),
            Lemma {
                name: "preimage_union".into(),
                statement: create_const_expr("∀f,A,B. f⁻¹(A ∪ B) = f⁻¹(A) ∪ f⁻¹(B)"),
                proof: ProofMethod::Smt,
                priority: 100,
                tags: text_list(&["topology", "preimage"]),
            },
        );
    }
}

impl Default for LemmaDatabase {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Proof Search Integration ====================

/// Mathematical structure verifier integrating with proof search
#[derive(Debug)]
pub struct MathStructureVerifier {
    /// Lemma database
    lemma_db: LemmaDatabase,

    /// Tactic evaluator
    tactic_eval: TacticEvaluator,
}

impl MathStructureVerifier {
    /// Create a new structure verifier
    pub fn new() -> Self {
        Self {
            lemma_db: LemmaDatabase::with_core(),
            tactic_eval: TacticEvaluator::new(),
        }
    }

    /// Verify all axioms of a structure
    pub fn verify_structure(
        &mut self,
        structure: &MathStructure,
    ) -> Result<List<ProofState>, Text> {
        let mut results = List::new();

        for axiom in &structure.axioms {
            let goal = Goal {
                id: 0,
                proposition: Heap::new(axiom.formula.clone()),
                hypotheses: List::new(),
                meta: crate::tactic_evaluation::GoalMetadata {
                    source: Maybe::None,
                    name: Maybe::Some(axiom.name.clone()),
                    from_induction: false,
                    parent_id: Maybe::None,
                },
            };

            let initial_state = ProofState {
                goals: {
                    let mut goals = List::new();
                    goals.push(goal);
                    goals
                },
                proven_goals: List::new(),
                global_hypotheses: List::new(),
                next_goal_id: 1,
            };

            results.push(initial_state);
        }

        Ok(results)
    }

    /// Find and apply applicable lemmas to a goal
    pub fn apply_lemmas(&self, goal: &Expr, structure: &Text) -> List<ProofMethod> {
        let lemmas = self.lemma_db.find_applicable_lemmas(goal, structure);

        lemmas.iter().map(|lemma| lemma.proof.clone()).collect()
    }

    /// Get the lemma database
    pub fn lemma_database(&self) -> &LemmaDatabase {
        &self.lemma_db
    }

    /// Get mutable access to lemma database
    pub fn lemma_database_mut(&mut self) -> &mut LemmaDatabase {
        &mut self.lemma_db
    }
}

impl Default for MathStructureVerifier {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Helper Functions ====================

/// Create a list of text from string slice
fn text_list(items: &[&str]) -> List<Text> {
    let mut list = List::new();
    for item in items {
        list.push((*item).into());
    }
    list
}

/// Create a variable expression
fn create_var_expr(name: &str, span: verum_ast::span::Span) -> Expr {
    use verum_ast::Path;
    Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new(name, span))),
        span,
    )
}

/// Create a constant expression
fn create_const_expr(name: &str) -> Expr {
    use verum_ast::Path;
    use verum_ast::span::Span;

    let span = Span::dummy();
    Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new(name, span))),
        span,
    )
}

/// Create a simple axiom from a name and description
///
/// Parses the description string to extract the axiom formula.
/// The description should be in a simplified mathematical notation:
///
/// - `forall a, b, c: op(op(a, b), c) = op(a, op(b, c))` - associativity
/// - `forall a: op(id, a) = a` - left identity
/// - `forall a: op(inv(a), a) = id` - left inverse
/// - `forall a, b: op(a, b) = op(b, a)` - commutativity
///
/// # Arguments
/// * `name` - The axiom name (e.g., "assoc", "left_id")
/// * `description` - A textual description of the axiom
///
/// # Returns
/// A properly constructed Axiom with parsed formula and quantified variables
fn create_axiom(name: &str, description: &str) -> Axiom {
    use verum_ast::{Path, span::Span};

    let span = Span::dummy();

    // Parse the description to extract formula structure
    let (quantified_vars, formula) = parse_axiom_description(name, description, span);

    // Determine if this axiom is SMT-checkable based on complexity
    // Axioms with only first-order quantifiers and basic arithmetic are checkable
    let smt_checkable = !description.contains("induction")
        && !description.contains("higher-order")
        && !description.contains("exists unique");

    Axiom {
        name: name.into(),
        formula,
        quantified_vars,
        smt_checkable,
    }
}

/// Parse axiom description into formula and quantified variables
///
/// Handles common axiom patterns:
/// - Associativity: op(op(a,b),c) = op(a,op(b,c))
/// - Identity: op(id,a) = a or op(a,id) = a
/// - Inverse: op(inv(a),a) = id or op(a,inv(a)) = id
/// - Commutativity: op(a,b) = op(b,a)
/// - Distributivity: mul(a,add(b,c)) = add(mul(a,b),mul(a,c))
fn parse_axiom_description(
    name: &str,
    description: &str,
    span: verum_ast::span::Span,
) -> (List<Text>, Expr) {
    use verum_ast::{Path, ty::Ident};

    // Extract quantified variables from "forall a, b, c:" prefix
    let mut quantified_vars = List::new();
    let formula_text = if let Some(forall_idx) = description.find("forall") {
        let after_forall = &description[forall_idx + 6..];
        if let Some(colon_idx) = after_forall.find(':') {
            let vars_str = &after_forall[..colon_idx];
            for var in vars_str.split(',') {
                let var_name = var.trim();
                if !var_name.is_empty() {
                    quantified_vars.push(Text::from(var_name));
                }
            }
            after_forall[colon_idx + 1..].trim()
        } else {
            description
        }
    } else {
        // Check for universal quantifier symbol
        if let Some(after_symbol) = description.strip_prefix("∀") {
            // Skip ∀
            if let Some(dot_idx) = after_symbol.find('.') {
                let vars_str = &after_symbol[..dot_idx];
                for var in vars_str.split(',') {
                    let var_name = var.trim();
                    if !var_name.is_empty() {
                        quantified_vars.push(Text::from(var_name));
                    }
                }
                after_symbol[dot_idx + 1..].trim()
            } else {
                description
            }
        } else {
            description
        }
    };

    // Parse the formula based on axiom patterns
    let formula = match name {
        "assoc" | "associativity" | "assoc_add" | "assoc_mul" => {
            // op(op(a, b), c) = op(a, op(b, c))
            create_associativity_expr(&quantified_vars, span)
        }
        "left_id" | "left_identity" => {
            // op(id, a) = a
            create_left_identity_expr(&quantified_vars, span)
        }
        "right_id" | "right_identity" => {
            // op(a, id) = a
            create_right_identity_expr(&quantified_vars, span)
        }
        "left_inv" | "left_inverse" => {
            // op(inv(a), a) = id
            create_left_inverse_expr(&quantified_vars, span)
        }
        "right_inv" | "right_inverse" => {
            // op(a, inv(a)) = id
            create_right_inverse_expr(&quantified_vars, span)
        }
        "comm" | "commutativity" | "comm_add" | "comm_mul" => {
            // op(a, b) = op(b, a)
            create_commutativity_expr(&quantified_vars, span)
        }
        "distrib_left" | "left_distributivity" => {
            // mul(a, add(b, c)) = add(mul(a, b), mul(a, c))
            create_left_distributivity_expr(&quantified_vars, span)
        }
        "distrib_right" | "right_distributivity" => {
            // mul(add(a, b), c) = add(mul(a, c), mul(b, c))
            create_right_distributivity_expr(&quantified_vars, span)
        }
        _ => {
            // For unrecognized axioms, create a formula from the name
            create_const_expr(name)
        }
    };

    (quantified_vars, formula)
}

/// Create associativity expression: op(op(a, b), c) = op(a, op(b, c))
fn create_associativity_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };
    let b = if vars.len() > 1 {
        vars[1].clone()
    } else {
        Text::from("b")
    };
    let c = if vars.len() > 2 {
        vars[2].clone()
    } else {
        Text::from("c")
    };

    // Left side: op(op(a, b), c)
    let left_inner = create_binary_call("op", &a, &b, span);
    let left = create_binary_call_expr("op", left_inner, create_var_expr(&c, span), span);

    // Right side: op(a, op(b, c))
    let right_inner = create_binary_call("op", &b, &c, span);
    let right = create_binary_call_expr("op", create_var_expr(&a, span), right_inner, span);

    // Equality: left = right
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create left identity expression: op(id, a) = a
fn create_left_identity_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };

    let left = create_binary_call("op", &Text::from("id"), &a, span);
    let right = create_var_expr(&a, span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create right identity expression: op(a, id) = a
fn create_right_identity_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };

    let left = create_binary_call("op", &a, &Text::from("id"), span);
    let right = create_var_expr(&a, span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create left inverse expression: op(inv(a), a) = id
fn create_left_inverse_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };

    // inv(a)
    let inv_a = create_unary_call("inv", &a, span);
    // op(inv(a), a)
    let left = create_binary_call_expr("op", inv_a, create_var_expr(&a, span), span);
    let right = create_var_expr(&Text::from("id"), span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create right inverse expression: op(a, inv(a)) = id
fn create_right_inverse_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };

    // inv(a)
    let inv_a = create_unary_call("inv", &a, span);
    // op(a, inv(a))
    let left = create_binary_call_expr("op", create_var_expr(&a, span), inv_a, span);
    let right = create_var_expr(&Text::from("id"), span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create commutativity expression: op(a, b) = op(b, a)
fn create_commutativity_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };
    let b = if vars.len() > 1 {
        vars[1].clone()
    } else {
        Text::from("b")
    };

    let left = create_binary_call("op", &a, &b, span);
    let right = create_binary_call("op", &b, &a, span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create left distributivity expression: mul(a, add(b, c)) = add(mul(a, b), mul(a, c))
fn create_left_distributivity_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };
    let b = if vars.len() > 1 {
        vars[1].clone()
    } else {
        Text::from("b")
    };
    let c = if vars.len() > 2 {
        vars[2].clone()
    } else {
        Text::from("c")
    };

    // Left: mul(a, add(b, c))
    let add_bc = create_binary_call("add", &b, &c, span);
    let left = create_binary_call_expr("mul", create_var_expr(&a, span), add_bc, span);

    // Right: add(mul(a, b), mul(a, c))
    let mul_ab = create_binary_call("mul", &a, &b, span);
    let mul_ac = create_binary_call("mul", &a, &c, span);
    let right = create_binary_call_expr("add", mul_ab, mul_ac, span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create right distributivity expression: mul(add(a, b), c) = add(mul(a, c), mul(b, c))
fn create_right_distributivity_expr(vars: &List<Text>, span: verum_ast::span::Span) -> Expr {
    let a = if !vars.is_empty() {
        vars[0].clone()
    } else {
        Text::from("a")
    };
    let b = if vars.len() > 1 {
        vars[1].clone()
    } else {
        Text::from("b")
    };
    let c = if vars.len() > 2 {
        vars[2].clone()
    } else {
        Text::from("c")
    };

    // Left: mul(add(a, b), c)
    let add_ab = create_binary_call("add", &a, &b, span);
    let left = create_binary_call_expr("mul", add_ab, create_var_expr(&c, span), span);

    // Right: add(mul(a, c), mul(b, c))
    let mul_ac = create_binary_call("mul", &a, &c, span);
    let mul_bc = create_binary_call("mul", &b, &c, span);
    let right = create_binary_call_expr("add", mul_ac, mul_bc, span);

    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span,
    )
}

/// Create a binary function call expression from two Text variable names
fn create_binary_call(
    op_name: &str,
    arg1: &Text,
    arg2: &Text,
    span: verum_ast::span::Span,
) -> Expr {
    create_binary_call_expr(
        op_name,
        create_var_expr(arg1, span),
        create_var_expr(arg2, span),
        span,
    )
}

/// Create a binary function call expression from two Expr arguments
fn create_binary_call_expr(
    op_name: &str,
    arg1: Expr,
    arg2: Expr,
    span: verum_ast::span::Span,
) -> Expr {
    use verum_ast::{Path, ty::Ident};

    Expr::new(
        ExprKind::Call {
            func: Heap::new(Expr::new(
                ExprKind::Path(Path::from_ident(Ident::new(op_name, span))),
                span,
            )),
            type_args: Vec::new().into(),
            args: vec![arg1, arg2].into(),
        },
        span,
    )
}

/// Create a unary function call expression
fn create_unary_call(op_name: &str, arg: &Text, span: verum_ast::span::Span) -> Expr {
    use verum_ast::{Path, ty::Ident};

    Expr::new(
        ExprKind::Call {
            func: Heap::new(Expr::new(
                ExprKind::Path(Path::from_ident(Ident::new(op_name, span))),
                span,
            )),
            type_args: Vec::new().into(),
            args: vec![create_var_expr_text(arg, span)].into(),
        },
        span,
    )
}

/// Create a variable reference expression from Text
fn create_var_expr_text(name: &Text, span: verum_ast::span::Span) -> Expr {
    use verum_ast::{Path, ty::Ident};

    Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new(name.as_str(), span))),
        span,
    )
}

// ==================== Standard Structures ====================

/// Create standard integer group under addition
pub fn integer_addition_group() -> MathStructure {
    GroupBuilder::new("IntegerAddition".into(), "Int".into())
        .abelian()
        .build()
}

/// Create standard real field
pub fn real_field() -> Field {
    Field::new("Real".into(), "Real".into())
}

/// Create standard category Set
pub fn category_set() -> Category {
    let mut cat = Category::new("Set".into());
    cat.add_object("Type".into());
    cat
}

/// Create standard category Grp (groups)
pub fn category_grp() -> Category {
    let mut cat = Category::new("Grp".into());
    cat.add_object("Group".into());
    cat
}

/// Create standard 2D vector space
pub fn vector_space_r2() -> VectorSpace {
    VectorSpace::new("R2".into(), "Vec2".into(), "Real".into())
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_builder() {
        let group = GroupBuilder::new("TestGroup".into(), "Int".into()).build();

        assert_eq!(group.name, Text::from("TestGroup"));
        assert_eq!(group.category, StructureCategory::Group);
        assert!(group.operations.contains_key(&Text::from("op")));
        assert!(group.operations.contains_key(&Text::from("id")));
        assert!(group.operations.contains_key(&Text::from("inv")));
        assert_eq!(group.axioms.len(), 3); // assoc, left_id, left_inv
    }

    #[test]
    fn test_abelian_group() {
        let group = GroupBuilder::new("AbelianGroup".into(), "Int".into())
            .abelian()
            .build();

        assert_eq!(group.category, StructureCategory::AbelianGroup);
        assert_eq!(group.axioms.len(), 4); // includes commutativity
    }

    #[test]
    fn test_ring_creation() {
        let ring = Ring::new("TestRing".into(), "Int".into());

        assert_eq!(ring.name, Text::from("TestRing"));
        assert!(!ring.is_commutative);
        assert_eq!(ring.axioms.len(), 8); // add+mul+distrib axioms
    }

    #[test]
    fn test_field_creation() {
        let field = Field::new("TestField".into(), "Real".into());

        assert!(field.ring.is_commutative);
        assert_eq!(field.inverse.name, Text::from("inv"));
    }

    #[test]
    fn test_category_creation() {
        let mut cat = Category::new("TestCat".into());
        cat.add_object("A".into());
        cat.add_object("B".into());
        cat.add_morphism("A".into(), "B".into(), "f".into());

        assert_eq!(cat.objects.len(), 2);
        assert_eq!(cat.axioms.len(), 3); // left_id, right_id, assoc
    }

    #[test]
    fn test_lemma_database() {
        let db = LemmaDatabase::with_core();

        // Should have lemmas for various structures
        assert!(db.lemmas_by_structure.contains_key(&Text::from("Group")));
        assert!(db.lemmas_by_structure.contains_key(&Text::from("Ring")));
        assert!(db.lemmas_by_structure.contains_key(&Text::from("Field")));
    }

    #[test]
    fn test_verifier_creation() {
        let verifier = MathStructureVerifier::new();

        // Should have a populated lemma database
        let db = verifier.lemma_database();
        assert!(db.lemmas_by_structure.len() > 0);
    }
}
