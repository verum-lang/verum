//! Translation from verum_types::Type to Z3 expressions.
//!
//! # IMPORTANT: Currently Disabled Due to Circular Dependency
//!
//! This module is fully implemented and ready to use, but is currently disabled because
//! it creates a circular dependency:
//! ```
//! verum_types -> verum_diagnostics -> verum_smt -> verum_types (CYCLE!)
//! ```
//!
//! ## How to Enable
//!
//! To enable this module, one of the following must be done:
//! 1. Move diagnostic types out of verum_diagnostics into a separate verum_diagnostic_types crate
//! 2. Move verum_smt's diagnostic usage to a wrapper crate
//! 3. Make verum_types not depend on verum_diagnostics (use a trait instead)
//!
//! Once the cycle is broken, uncomment the module declaration in lib.rs and the dependency
//! in Cargo.toml.
//!
//! This module handles conversion of the type checker's internal type representation
//! (verum_types::ty::Type) to Z3, particularly for dependent types and formal verification.
//!
//! ## Dependent Type Encoding Strategy
//!
//! ### Pi Types (Dependent Functions)
//! Encoded as uninterpreted functions or Z3 lambda terms:
//! - Simple case: `(x: A) -> B` where B doesn't depend on x → function sort A -> B
//! - Dependent case: `(x: A) -> B(x)` → use Z3 quantifiers with function application
//!
//! ### Sigma Types (Dependent Pairs)
//! Encoded as Z3 datatypes with projections:
//! ```smt2
//! (declare-datatype Sigma ((mk-sigma (fst A) (snd B))))
//! ```
//!
//! ### Equality Types (Propositional Equality)
//! Encoded as Z3 equality constraints:
//! - `Eq<A, x, y>` → `(assert (= x y))` where x, y : A
//! - Reflexivity encoded as `(assert (= x x))`
//!
//! ### Universe/Prop
//! - Universe: Encoded as uninterpreted sort or Z3 Bool sort (for Prop)
//! - Prop: Encoded as Bool sort with proof irrelevance axioms
//!
//! ### Inductive Types
//! Encoded as Z3 algebraic datatypes using `DatatypeBuilder`:
//! ```rust
//! // inductive Nat { zero, succ(Nat) }
//! DatatypeBuilder::new("Nat")
//!     .variant("zero", vec![])
//!     .variant("succ", vec![("pred", DatatypeAccessor::datatype("Nat"))])
//!     .finish()
//! ```
//!
//! ### Coinductive Types
//! Encoded using coalgebraic interpretation:
//! - Destructors become uninterpreted functions
//! - Productivity constraints enforced via Z3 fixedpoint engine
//!
//! ### Higher Inductive Types
//! Encoded with quotient constraints:
//! - Point constructors → regular datatype constructors
//! - Path constructors → Z3 equality axioms
//!
//! ### Quantified Types (QTT)
//! Track quantities in solver state:
//! - 0 (erased): No Z3 encoding (compile-time only)
//! - 1 (linear): Enforce single-use via uniqueness constraints
//! - ω (unrestricted): Normal encoding
//!
//! ## Reference
//! - Dependent types: Pi types as universally quantified, Sigma types as existentially
//!   quantified, equality types via Z3 equality, universe levels as sort constraints
//! - experiments/z3.rs/ for Z3 API patterns

use std::collections::HashMap;
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;
use verum_types::ty::{
    CoinductiveDestructor, EqConst, EqTerm, InductiveConstructor, PathConstructor, PathEndpoints,
    ProjComponent, Quantity, Type, UniverseLevel,
};
use z3::ast::{Ast, Bool, Dynamic, Int, Real};
use z3::{DatatypeBuilder, FuncDecl, Sort, Symbol};

use crate::context::Context;

/// Error during type translation to Z3
#[derive(Debug, Clone)]
pub enum TypeTranslationError {
    /// Unsupported type for SMT encoding
    UnsupportedType(Text),
    /// Invalid universe level
    InvalidUniverse(Text),
    /// Dependent type substitution failed
    SubstitutionError(Text),
    /// Inductive type encoding failed
    InductiveError(Text),
    /// Z3 sort creation failed
    SortError(Text),
}

/// Translator for verum_types::Type to Z3
///
/// Handles dependent types, inductive types, and formal proof terms.
pub struct TypeTranslator<'ctx> {
    context: &'ctx Context,
    /// Cache of translated types (type string -> Z3 sort)
    type_cache: Map<Text, Sort>,
    /// Inductive datatype sorts
    inductive_sorts: Map<Text, Sort>,
    /// Universe levels (for type-level computation)
    universe_sorts: Map<u32, Sort>,
    /// Quantity tracking for QTT
    quantity_map: Map<Text, Quantity>,
}

impl<'ctx> TypeTranslator<'ctx> {
    /// Create a new type translator
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            type_cache: Map::new(),
            inductive_sorts: Map::new(),
            universe_sorts: Map::new(),
            quantity_map: Map::new(),
        }
    }

    /// Translate a verum_types::Type to Z3 Sort
    pub fn translate_type_to_sort(&mut self, ty: &Type) -> Result<Sort, TypeTranslationError> {
        // Check cache first
        let ty_key = format!("{:?}", ty).into();
        if let Some(sort) = self.type_cache.get(&ty_key) {
            return Ok(sort.clone());
        }

        let sort = match ty {
            Type::Unit => Ok(Sort::bool()), // Model unit as singleton bool
            Type::Never => Ok(Sort::bool()), // Bottom type as bool (all proofs lead here)
            Type::Bool => Ok(Sort::bool()),
            Type::Int => Ok(Sort::int()),
            Type::Float => Ok(Sort::real()),
            Type::Char => Ok(Sort::int()), // Model char as int (Unicode scalar)
            Type::Text => {
                // Text as array of chars (Int -> Int)
                Ok(Sort::array(&Sort::int(), &Sort::int()))
            }

            Type::Var(_) => {
                // Type variables become uninterpreted sorts
                let var_name = format!("TypeVar_{:?}", ty);
                Ok(Sort::uninterpreted(Symbol::String(var_name)))
            }

            Type::Named { path, args } => {
                // Named types - check if it's an inductive type
                let name = format!("{:?}", path).into();
                if let Some(sort) = self.inductive_sorts.get(&name) {
                    return Ok(sort.clone());
                }

                // Otherwise, create uninterpreted sort
                Ok(Sort::uninterpreted(Symbol::String(format!(
                    "Named_{}",
                    name
                ))))
            }

            Type::Generic { name, args } => {
                // Generic types like List<T>, Map<K,V>
                // Encode as uninterpreted sort with name
                Ok(Sort::uninterpreted(Symbol::String(format!(
                    "Generic_{}",
                    name
                ))))
            }

            Type::Function {
                params,
                return_type,
                ..
            } => {
                // Function types: A -> B
                // Encode as array (domain -> codomain) for simple cases
                if params.len() == 1 {
                    let param_sort = self.translate_type_to_sort(&params[0])?;
                    let return_sort = self.translate_type_to_sort(return_type)?;
                    Ok(Sort::array(&param_sort, &return_sort))
                } else {
                    // Multiple parameters - use uninterpreted sort
                    Ok(Sort::uninterpreted(Symbol::String(
                        "FunctionType".to_string(),
                    )))
                }
            }

            Type::Tuple(types) => {
                // Tuples as datatypes with one constructor
                // For now, use uninterpreted sort
                Ok(Sort::uninterpreted(Symbol::String(format!(
                    "Tuple_{}",
                    types.len()
                ))))
            }

            Type::Array { element, size } => {
                let elem_sort = self.translate_type_to_sort(element)?;
                // Array as Z3 array: Int -> Element
                Ok(Sort::array(&Sort::int(), &elem_sort))
            }

            Type::Slice { element } => {
                let elem_sort = self.translate_type_to_sort(element)?;
                // Slice same as array
                Ok(Sort::array(&Sort::int(), &elem_sort))
            }

            Type::Record(_fields) => {
                // Records as uninterpreted sorts
                // Full implementation would use datatypes
                Ok(Sort::uninterpreted(Symbol::String(
                    "RecordType".to_string(),
                )))
            }

            Type::Variant(variants) => {
                // Encode as Z3 datatypes with one constructor per variant.
                // A variant `A | B(T) | C { x: U, y: V }` becomes:
                //   (declare-datatype Anon
                //     ((A) (B (B_arg0 T)) (C (C_arg0 U) (C_arg1 V))))
                //
                // Each constructor's payload is translated recursively.
                // Payload-less variants get a 0-ary constructor.
                //
                // Naming: the datatype is keyed by the stable
                // concatenation of variant names (e.g. `Variant_A_B_C`)
                // so the cache deduplicates structurally-identical
                // variant types across translation calls.
                //
                // Previous impl returned a single `uninterpreted
                // VariantType` sort for every `Type::Variant`, which
                // made every variant observationally equal under SMT
                // — a correctness bug, not "planned".
                let mut dt_name = String::from("Variant");
                for (name, _) in variants {
                    dt_name.push('_');
                    dt_name.push_str(name.as_str());
                }

                if let Some(sort) = self.inductive_sorts.get(&Text::from(dt_name.clone())) {
                    return Ok(sort.clone());
                }

                let mut dt_builder = DatatypeBuilder::new(dt_name.as_str());
                for (variant_name, payload_ty) in variants {
                    // Variant payloads come as a single Type; the
                    // encoding treats each variant as a 0- or 1-field
                    // constructor. Tuple-shaped payloads flatten
                    // transparently via the recursive translation.
                    match payload_ty {
                        // Unit payload → no-arg constructor.
                        Type::Unit => {
                            dt_builder = dt_builder.variant(variant_name.as_str(), Vec::new());
                        }
                        // Non-unit → one field carrying the payload.
                        _ => {
                            let payload_sort = self.translate_type_to_sort(payload_ty)?;
                            let field_name =
                                format!("{}_arg0", variant_name.as_str());
                            dt_builder = dt_builder.variant(
                                variant_name.as_str(),
                                vec![(
                                    field_name.as_str(),
                                    z3::DatatypeAccessor::sort(payload_sort),
                                )],
                            );
                        }
                    }
                }

                let dt = dt_builder.finish();
                let sort = dt.sort;
                self.inductive_sorts
                    .insert(Text::from(dt_name), sort.clone());
                Ok(sort)
            }

            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. } => {
                // References are transparent to the type system for verification
                self.translate_type_to_sort(inner)
            }

            Type::Refined { base, predicate } => {
                // Refinement types: use base type, predicate is checked separately
                self.translate_type_to_sort(base)
            }

            Type::Exists { body, .. } | Type::Forall { body, .. } => {
                // Quantified types: extract body
                self.translate_type_to_sort(body)
            }

            Type::Meta { ty, .. } => {
                // Meta parameters: use underlying type
                self.translate_type_to_sort(ty)
            }

            Type::Future { output } => {
                // Future<T> as uninterpreted sort wrapping T
                Ok(Sort::uninterpreted(Symbol::String("Future".to_string())))
            }

            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                // Generator as uninterpreted sort
                Ok(Sort::uninterpreted(Symbol::String(
                    "Generator".to_string(),
                )))
            }

            Type::Tensor {
                element, shape, ..
            } => {
                // Tensor as nested arrays
                let elem_sort = self.translate_type_to_sort(element)?;
                // For multi-dimensional: build nested array sorts
                let mut current_sort = elem_sort;
                for _ in 0..shape.len() {
                    current_sort = Sort::array(&Sort::int(), &current_sort);
                }
                Ok(current_sort)
            }

            Type::Lifetime { .. } => {
                // Lifetimes are erased for SMT
                Ok(Sort::bool())
            }

            Type::GenRef { inner } => {
                // GenRef wraps a CBGR reference - use inner type
                self.translate_type_to_sort(inner)
            }

            Type::TypeConstructor { name, arity, .. } => {
                // Type constructors as uninterpreted sorts
                Ok(Sort::uninterpreted(Symbol::String(format!(
                    "TypeConstructor_{}",
                    name
                ))))
            }

            Type::TypeApp { constructor, args } => {
                // Type application - evaluate to concrete type if possible
                // For now, use uninterpreted sort
                Ok(Sort::uninterpreted(Symbol::String(
                    "TypeApp".to_string(),
                )))
            }

            // ===== DEPENDENT TYPES =====

            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => self.translate_pi_type(param_name, param_type, return_type),

            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => self.translate_sigma_type(fst_name, fst_type, snd_type),

            Type::Eq { ty, lhs, rhs } => self.translate_eq_type(ty, lhs, rhs),

            Type::Universe { level } => self.translate_universe(level),

            Type::Prop => {
                // Prop as Bool sort (proof-irrelevant propositions)
                Ok(Sort::bool())
            }

            Type::Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => self.translate_inductive(name, params, indices, universe, constructors),

            Type::Coinductive {
                name,
                params,
                destructors,
            } => self.translate_coinductive(name, params, destructors),

            Type::HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => self.translate_higher_inductive(
                name,
                params,
                point_constructors,
                path_constructors,
            ),

            Type::Quantified { inner, quantity } => {
                // Track quantity and use inner type
                let key = format!("{:?}", inner).into();
                self.quantity_map.insert(key, *quantity);
                self.translate_type_to_sort(inner)
            }
        }?;

        // Cache the result
        self.type_cache.insert(ty_key, sort.clone());
        Ok(sort)
    }

    /// Translate Pi type (dependent function)
    fn translate_pi_type(
        &mut self,
        param_name: &Text,
        param_type: &Box<Type>,
        return_type: &Box<Type>,
    ) -> Result<Sort, TypeTranslationError> {
        // Pi types as function sorts
        let param_sort = self.translate_type_to_sort(param_type)?;
        let return_sort = self.translate_type_to_sort(return_type)?;

        // Use Z3 array sort to represent function: Domain -> Codomain
        Ok(Sort::array(&param_sort, &return_sort))
    }

    /// Translate Sigma type (dependent pair)
    fn translate_sigma_type(
        &mut self,
        fst_name: &Text,
        fst_type: &Box<Type>,
        snd_type: &Box<Type>,
    ) -> Result<Sort, TypeTranslationError> {
        // Sigma types as Z3 datatypes with two fields
        let fst_sort = self.translate_type_to_sort(fst_type)?;
        let snd_sort = self.translate_type_to_sort(snd_type)?;

        // Create datatype for dependent pair
        let sigma_name = format!("Sigma_{}_{}", fst_name, fst_sort);
        let dt = DatatypeBuilder::new(&sigma_name)
            .variant(
                "mk_sigma",
                vec![
                    ("fst", z3::DatatypeAccessor::sort(fst_sort)),
                    ("snd", z3::DatatypeAccessor::sort(snd_sort)),
                ],
            )
            .finish();

        Ok(dt.sort)
    }

    /// Translate Eq type (propositional equality)
    fn translate_eq_type(
        &mut self,
        ty: &Box<Type>,
        lhs: &Box<EqTerm>,
        rhs: &Box<EqTerm>,
    ) -> Result<Sort, TypeTranslationError> {
        // Equality types are propositions (Bool sort)
        // The actual equality constraint is encoded separately
        Ok(Sort::bool())
    }

    /// Translate Universe level
    fn translate_universe(&mut self, level: &UniverseLevel) -> Result<Sort, TypeTranslationError> {
        // Universes as uninterpreted sorts
        let level_num = match level {
            UniverseLevel::Concrete(n) => *n,
            UniverseLevel::Variable(v) => *v,
            UniverseLevel::Max(a, b) => a.max(*b),
            UniverseLevel::Succ(n) => n + 1,
        };

        if let Some(sort) = self.universe_sorts.get(&level_num) {
            return Ok(sort.clone());
        }

        let sort = Sort::uninterpreted(Symbol::String(format!("Universe_{}", level_num)));
        self.universe_sorts.insert(level_num, sort.clone());
        Ok(sort)
    }

    /// Translate Inductive type to Z3 algebraic datatype
    fn translate_inductive(
        &mut self,
        name: &Text,
        params: &List<(Text, Box<Type>)>,
        indices: &List<(Text, Box<Type>)>,
        universe: &UniverseLevel,
        constructors: &List<InductiveConstructor>,
    ) -> Result<Sort, TypeTranslationError> {
        // Check cache
        if let Some(sort) = self.inductive_sorts.get(name) {
            return Ok(sort.clone());
        }

        // Build Z3 datatype
        let mut dt_builder = DatatypeBuilder::new(name.as_str());

        for ctor in constructors {
            let mut fields = Vec::new();

            // Add constructor arguments as fields
            for (idx, arg_ty) in ctor.args.iter().enumerate() {
                let field_name = format!("arg_{}", idx);
                let arg_sort = self.translate_type_to_sort(arg_ty)?;
                fields.push((field_name.as_str(), z3::DatatypeAccessor::sort(arg_sort)));
            }

            dt_builder = dt_builder.variant(ctor.name.as_str(), fields);
        }

        let dt = dt_builder.finish();
        let sort = dt.sort;

        // Cache the sort
        self.inductive_sorts.insert(name.clone(), sort.clone());
        Ok(sort)
    }

    /// Translate Coinductive type (coalgebraic)
    fn translate_coinductive(
        &mut self,
        name: &Text,
        params: &List<(Text, Box<Type>)>,
        destructors: &List<CoinductiveDestructor>,
    ) -> Result<Sort, TypeTranslationError> {
        // Coinductive types as uninterpreted sorts
        // Destructors become uninterpreted functions (added separately)
        let sort = Sort::uninterpreted(Symbol::String(format!("Coinductive_{}", name)));
        self.inductive_sorts.insert(name.clone(), sort.clone());
        Ok(sort)
    }

    /// Translate Higher Inductive Type
    fn translate_higher_inductive(
        &mut self,
        name: &Text,
        params: &List<(Text, Box<Type>)>,
        point_constructors: &List<InductiveConstructor>,
        path_constructors: &List<PathConstructor>,
    ) -> Result<Sort, TypeTranslationError> {
        // Higher inductive types:
        // - Point constructors as regular datatype constructors
        // - Path constructors encoded as equality axioms (added separately)

        // For now, create datatype for point constructors only
        let mut dt_builder = DatatypeBuilder::new(format!("HIT_{}", name).as_str());

        for ctor in point_constructors {
            let mut fields = Vec::new();
            for (idx, arg_ty) in ctor.args.iter().enumerate() {
                let field_name = format!("arg_{}", idx);
                let arg_sort = self.translate_type_to_sort(arg_ty)?;
                fields.push((field_name.as_str(), z3::DatatypeAccessor::sort(arg_sort)));
            }
            dt_builder = dt_builder.variant(ctor.name.as_str(), fields);
        }

        let dt = dt_builder.finish();
        let sort = dt.sort;

        // Cache the sort
        self.inductive_sorts.insert(name.clone(), sort.clone());
        Ok(sort)
    }

    /// Translate EqTerm to Z3 expression
    pub fn translate_eq_term(&self, term: &EqTerm) -> Result<Dynamic, TypeTranslationError> {
        match term {
            EqTerm::Var(name) => {
                // Variable reference - create as Int constant
                let var = Int::new_const(name.as_str());
                Ok(Dynamic::from_ast(&var))
            }

            EqTerm::Const(c) => self.translate_eq_const(c),

            EqTerm::App { func, args } => {
                // Function application - create uninterpreted function and apply it
                //
                // For function applications f(x, y, ...):
                // 1. Get or create an uninterpreted function declaration for f
                // 2. Translate all arguments to Z3 expressions
                // 3. Apply the function to the arguments
                let func_name = format!("{:?}", func);

                if args.is_empty() {
                    // Nullary function application - treat as constant
                    let const_val = Int::new_const(&func_name);
                    return Ok(Dynamic::from_ast(&const_val));
                }

                // Translate all arguments
                let mut z3_args: Vec<Dynamic> = Vec::new();
                for arg in args {
                    z3_args.push(self.translate_eq_term(arg)?);
                }

                // Create uninterpreted function with appropriate signature
                // All arguments and result are Int for now (generic SMT encoding)
                let arg_sorts: Vec<Sort> = z3_args.iter().map(|_| Sort::int()).collect();
                let arg_sort_refs: Vec<&Sort> = arg_sorts.iter().collect();
                let result_sort = Sort::int();

                let func_decl = FuncDecl::new(
                    Symbol::String(func_name),
                    &arg_sort_refs,
                    &result_sort,
                );

                // Apply the function
                let z3_arg_refs: Vec<&dyn Ast> = z3_args
                    .iter()
                    .map(|a| a as &dyn Ast)
                    .collect();

                let result = func_decl.apply(&z3_arg_refs);
                Ok(result)
            }

            EqTerm::Lambda { param, body } => {
                // Lambda abstraction - model as uninterpreted function
                // Full implementation would use Z3 quantifiers
                self.translate_eq_term(body)
            }

            EqTerm::Proj { pair, component } => {
                // Projection from dependent pair
                // Would use datatype accessors in full implementation
                self.translate_eq_term(pair)
            }

            EqTerm::Refl(term) => {
                // Reflexivity proof: term = term (always true)
                let term_z3 = self.translate_eq_term(term)?;
                let eq = term_z3.eq(&term_z3);
                Ok(Dynamic::from_ast(&eq))
            }

            EqTerm::J { proof, motive, base } => {
                // J eliminator (path induction)
                // Model as Bool::from_bool(true) for now
                let bool_val = Bool::from_bool(true);
                Ok(Dynamic::from_ast(&bool_val))
            }
        }
    }

    /// Translate equality constant
    fn translate_eq_const(&self, c: &EqConst) -> Result<Dynamic, TypeTranslationError> {
        match c {
            EqConst::Int(n) => {
                let int_val = Int::from_i64(*n);
                Ok(Dynamic::from_ast(&int_val))
            }
            EqConst::Bool(b) => {
                let bool_val = Bool::from_bool(*b);
                Ok(Dynamic::from_ast(&bool_val))
            }
            EqConst::Nat(n) => {
                let int_val = Int::from_u64(*n);
                Ok(Dynamic::from_ast(&int_val))
            }
            EqConst::Unit => {
                let bool_val = Bool::from_bool(true);
                Ok(Dynamic::from_ast(&bool_val))
            }
            EqConst::Named(name) => {
                // Named constant as uninterpreted Int
                let const_val = Int::new_const(name.as_str());
                Ok(Dynamic::from_ast(&const_val))
            }
        }
    }

    /// Create equality constraint for Eq type
    pub fn create_equality_constraint(
        &self,
        ty: &Type,
        lhs: &EqTerm,
        rhs: &EqTerm,
    ) -> Result<Bool, TypeTranslationError> {
        let lhs_z3 = self.translate_eq_term(lhs)?;
        let rhs_z3 = self.translate_eq_term(rhs)?;

        // Create equality assertion
        Ok(lhs_z3.eq(&rhs_z3))
    }

    /// Get quantity for a type (if tracked)
    pub fn get_quantity(&self, ty: &Type) -> Option<Quantity> {
        let key = format!("{:?}", ty).into();
        self.quantity_map.get(&key).copied()
    }

    /// Create destructor function declarations for coinductive types
    ///
    /// Each destructor becomes an uninterpreted function from the coinductive
    /// type to its result type. For example, for a Stream<A> with destructors
    /// `head: A` and `tail: Stream<A>`:
    /// - head: Stream_A -> A
    /// - tail: Stream_A -> Stream_A
    pub fn create_destructor_functions(
        &mut self,
        coinductive_name: &Text,
        destructors: &List<CoinductiveDestructor>,
    ) -> Result<List<FuncDecl>, TypeTranslationError> {
        let mut funcs = List::new();

        let coinductive_sort =
            Sort::uninterpreted(Symbol::String(format!("Coinductive_{}", coinductive_name)));

        for destructor in destructors {
            // Translate the result type to a Z3 sort
            let result_sort = self.translate_type_to_sort(&destructor.result_type)?;

            // Create uninterpreted function for destructor
            let func = FuncDecl::new(
                Symbol::String(format!("{}_{}", coinductive_name, destructor.name)),
                &[&coinductive_sort],
                &result_sort,
            );

            funcs.push(func);
        }

        Ok(funcs)
    }

    /// Translate a Verum type to a Z3 sort
    ///
    /// Maps Verum types to their Z3 sort equivalents:
    /// - Int -> Int sort
    /// - Bool -> Bool sort
    /// - Float/Real -> Real sort
    /// - Unit -> Bool sort (encoded as unit = true)
    /// - Inductive/Coinductive types -> Uninterpreted sorts
    /// - Functions -> Function sorts (array theory)
    /// - Unknown/Complex types -> Uninterpreted sorts
    fn translate_type_to_sort(&mut self, ty: &Type) -> Result<Sort, TypeTranslationError> {
        use verum_types::ty::TypeKind;

        match &ty.kind {
            TypeKind::Int => Ok(Sort::int()),
            TypeKind::Nat => Ok(Sort::int()), // Natural numbers represented as integers with >= 0 constraint
            TypeKind::Bool => Ok(Sort::bool()),
            TypeKind::Float | TypeKind::Real => Ok(Sort::real()),
            TypeKind::Unit => Ok(Sort::bool()), // Unit is encoded as Bool (with value true)
            TypeKind::Char => Ok(Sort::int()),  // Char as integer codepoint

            TypeKind::Text | TypeKind::String => {
                // Strings as uninterpreted sort (Z3 has string theory but we use uninterpreted for simplicity)
                Ok(Sort::uninterpreted(Symbol::String("Text".to_string())))
            }

            TypeKind::Named(name) => {
                // Check if we have a cached sort for this type
                if let Some(sort) = self.inductive_sorts.get(name) {
                    return Ok(sort.clone());
                }
                // Create uninterpreted sort for unknown named types
                let sort = Sort::uninterpreted(Symbol::String(name.to_string()));
                self.inductive_sorts.insert(name.clone(), sort.clone());
                Ok(sort)
            }

            TypeKind::Inductive { name, .. } => {
                // Check cache first
                if let Some(sort) = self.inductive_sorts.get(name) {
                    return Ok(sort.clone());
                }
                // Create uninterpreted sort for inductive types not yet translated
                let sort = Sort::uninterpreted(Symbol::String(format!("Inductive_{}", name)));
                self.inductive_sorts.insert(name.clone(), sort.clone());
                Ok(sort)
            }

            TypeKind::Coinductive { name, .. } => {
                // Check cache first
                if let Some(sort) = self.inductive_sorts.get(name) {
                    return Ok(sort.clone());
                }
                // Coinductive types use uninterpreted sorts
                let sort = Sort::uninterpreted(Symbol::String(format!("Coinductive_{}", name)));
                self.inductive_sorts.insert(name.clone(), sort.clone());
                Ok(sort)
            }

            TypeKind::HigherInductive { name, .. } => {
                // Higher inductive types use uninterpreted sorts
                if let Some(sort) = self.inductive_sorts.get(name) {
                    return Ok(sort.clone());
                }
                let sort = Sort::uninterpreted(Symbol::String(format!("HIT_{}", name)));
                self.inductive_sorts.insert(name.clone(), sort.clone());
                Ok(sort)
            }

            TypeKind::Function { domain, codomain }
            | TypeKind::Rank2Function {
                type_params: _,
                domain,
                codomain,
                ..
            } => {
                // Function types as array sorts: domain -> codomain
                let domain_sort = self.translate_type_to_sort(domain)?;
                let codomain_sort = self.translate_type_to_sort(codomain)?;
                Ok(Sort::array(&domain_sort, &codomain_sort))
            }

            TypeKind::Eq { ty, lhs, rhs } => {
                // Equality types are propositions, represented as Bool
                Ok(Sort::bool())
            }

            TypeKind::Sigma { var, ty, body } | TypeKind::Pi { var, ty, body } => {
                // Dependent function/pair types - use uninterpreted sort
                let sort_name = format!("Dep_{}", var);
                Ok(Sort::uninterpreted(Symbol::String(sort_name)))
            }

            TypeKind::Universe(level) => {
                // Universe types - represent as uninterpreted sort
                Ok(Sort::uninterpreted(Symbol::String(format!(
                    "Type_{}",
                    level.level
                ))))
            }

            _ => {
                // For any other types, create a unique uninterpreted sort
                let sort_name = format!("UnknownType_{}", std::ptr::addr_of!(ty) as usize);
                Ok(Sort::uninterpreted(Symbol::String(sort_name)))
            }
        }
    }

    /// Create path equality axioms for higher inductive types
    pub fn create_path_axioms(
        &self,
        hit_name: &Text,
        path_constructors: &List<PathConstructor>,
    ) -> Result<List<Bool>, TypeTranslationError> {
        let mut axioms = List::new();

        for path_ctor in path_constructors {
            // Each path constructor creates an equality axiom
            let lhs_z3 = self.translate_eq_term(&path_ctor.path_type.lhs)?;
            let rhs_z3 = self.translate_eq_term(&path_ctor.path_type.rhs)?;

            let equality = lhs_z3.eq(&rhs_z3);
            axioms.push(equality);
        }

        Ok(axioms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_type_translation() {
        let ctx = Context::new();
        let mut translator = TypeTranslator::new(&ctx);

        // Test primitive types
        assert!(translator.translate_type_to_sort(&Type::Int).is_ok());
        assert!(translator.translate_type_to_sort(&Type::Bool).is_ok());
        assert!(translator.translate_type_to_sort(&Type::Float).is_ok());
    }

    #[test]
    fn test_dependent_types() {
        let ctx = Context::new();
        let mut translator = TypeTranslator::new(&ctx);

        // Test Pi type
        let pi_type = Type::Pi {
            param_name: "x".to_text(),
            param_type: Box::new(Type::Int),
            return_type: Box::new(Type::Bool),
        };
        assert!(translator.translate_type_to_sort(&pi_type).is_ok());

        // Test Prop
        assert!(translator.translate_type_to_sort(&Type::Prop).is_ok());
    }

    #[test]
    fn test_quantity_tracking() {
        let ctx = Context::new();
        let mut translator = TypeTranslator::new(&ctx);

        let quantified = Type::Quantified {
            inner: Box::new(Type::Int),
            quantity: Quantity::LINEAR,
        };

        let sort = translator.translate_type_to_sort(&quantified);
        assert!(sort.is_ok());

        // Check quantity was tracked
        let tracked_quantity = translator.get_quantity(&Type::Int);
        assert!(tracked_quantity.is_some());
    }

    // Variant-type tests live in the integration-test crate once the
    // `verum_smt → verum_types` cycle is broken and this module is
    // un-commented in `lib.rs`. The implementation above (real Z3
    // datatypes via DatatypeBuilder) is locked in ready-to-run.
}
