//! Integration Example: Specialization Selection with TypeChecker
//!
//! This example demonstrates how the SpecializationSelector integrates with
//! the TypeChecker to automatically select the most specific protocol implementation
//! during type inference.
//!
//! Run with: cargo run --example specialization_selection_integration

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Map, Maybe, Set};

use verum_types::advanced_protocols::SpecializationLattice;
use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolKind};
use verum_types::specialization_selection::SpecializationSelector;
use verum_types::ty::Type;
use verum_types::unify::Unifier;

fn main() {
    println!("=== Specialization Selection Integration Example ===\n");

    // Example 1: Basic specialization selection
    println!("Example 1: Basic Specialization Selection");
    println!("------------------------------------------");
    example_basic_specialization();
    println!();

    // Example 2: Specialization chain
    println!("Example 2: Specialization Chain");
    println!("--------------------------------");
    example_specialization_chain();
    println!();

    // Example 3: Error handling
    println!("Example 3: Ambiguity Detection");
    println!("-------------------------------");
    example_ambiguity_detection();
    println!();

    // Example 4: Performance metrics
    println!("Example 4: Performance Metrics");
    println!("-------------------------------");
    example_performance_metrics();
    println!();
}

/// Example 1: Basic specialization selection
///
/// Demonstrates selecting between a default implementation and a specialized one.
///
/// ```verum
/// // Default implementation
/// implement<T> Display for T {
///     fn display(self) -> Text { "default" }
/// }
///
/// // Specialized for Int
/// @specialize
/// implement Display for Int {
///     fn display(self) -> Text { "integer" }
/// }
/// ```
fn example_basic_specialization() {
    let _selector = SpecializationSelector::new();
    let _protocol_checker = ProtocolChecker::new();
    let _unifier = Unifier::new();

    // Create Display protocol
    let _display_protocol = Protocol {
        name: "Display".into(),
        kind: ProtocolKind::Constraint, // Constraint protocol, not injectable
        type_params: List::new(),
        methods: Map::new(),
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::default(),
    };

    // Create Int type
    let _int_type = Type::Named {
        path: Path::single(Ident::new("Int", Span::default())),
        args: List::new(),
    };

    println!("Protocol: Display");
    println!("Type: Int");
    println!();
    println!("Expected: Should select specialized implementation for Int");
    println!("Result: Selection would occur via select_implementation()");
    println!();
    println!("Implementation: The selector would:");
    println!("  1. Find candidate implementations (default and specialized)");
    println!("  2. Build specialization lattice");
    println!("  3. Rank by specificity");
    println!("  4. Select Int specialization (rank 1 > rank 0)");
}

/// Example 2: Specialization chain
///
/// Demonstrates a chain of specializations.
///
/// ```verum
/// implement<T> Protocol for T { }              // Rank 0 (most general)
/// @specialize implement<T: Clone> Protocol for T { }  // Rank 1
/// @specialize implement Protocol for Int { }          // Rank 2 (most specific)
/// ```
fn example_specialization_chain() {
    let mut lattice = SpecializationLattice::new();

    // Add implementations
    lattice.add_impl(0); // Most general
    lattice.add_impl(1); // Middle
    lattice.add_impl(2); // Most specific

    // Build chain
    lattice.ordering.insert((2, 1), true); // 2 > 1
    lattice.ordering.insert((1, 0), true); // 1 > 0
    lattice.ordering.insert((2, 0), true); // 2 > 0 (transitive)

    println!("Specialization Chain:");
    println!("  impl<T> (rank 0)");
    println!("    ↑ specializes");
    println!("  impl<T: Clone> (rank 1)");
    println!("    ↑ specializes");
    println!("  impl for Int (rank 2)");
    println!();

    // Test selection
    let mut applicable = Set::new();
    applicable.insert(0);
    applicable.insert(1);
    applicable.insert(2);

    match lattice.select_most_specific(&applicable) {
        Maybe::Some(selected) => {
            println!("Selected implementation: #{} (most specific)", selected);
            assert_eq!(selected, 2);
        }
        Maybe::None => {
            println!("Error: No unique most specific implementation");
        }
    }
}

/// Example 3: Ambiguity detection
///
/// Demonstrates error handling when multiple implementations apply without
/// a clear specialization ordering.
///
/// ```verum
/// implement<T: Send> Protocol for T { }
/// implement<T: Sync> Protocol for T { }
/// // Ambiguous for types that are both Send + Sync
/// ```
fn example_ambiguity_detection() {
    let mut lattice = SpecializationLattice::new();

    // Two implementations with no ordering
    lattice.add_impl(0); // impl<T: Send>
    lattice.add_impl(1); // impl<T: Sync>

    println!("Two implementations with no specialization relationship:");
    println!("  impl<T: Send> Protocol for T");
    println!("  impl<T: Sync> Protocol for T");
    println!();

    // Try to select for a type that is Send + Sync
    let mut applicable = Set::new();
    applicable.insert(0);
    applicable.insert(1);

    match lattice.select_most_specific(&applicable) {
        Maybe::Some(selected) => {
            println!("Selected implementation: #{}", selected);
        }
        Maybe::None => {
            println!("❌ Error: Ambiguous specialization");
            println!("   Multiple implementations apply with no clear ordering");
            println!();
            println!("Suggestion:");
            println!("   - Add @specialize to indicate which is more specific");
            println!("   - Or add more constraints to make them non-overlapping");
        }
    }
}

/// Example 4: Performance metrics
///
/// Demonstrates performance tracking and caching.
fn example_performance_metrics() {
    let mut selector = SpecializationSelector::new();

    println!("Initial statistics:");
    println!("  Selections: {}", selector.stats().selections);
    println!("  Cache hits: {}", selector.stats().cache_hits);
    println!("  Cache misses: {}", selector.stats().cache_misses);
    println!("  Time: {} μs", selector.stats().time_us);
    println!();

    // Simulate caching
    selector.cache_selection("Display".into(), "Int".into(), 42);
    selector.cache_selection("Debug".into(), "Bool".into(), 43);

    println!("After caching 2 selections:");
    println!("  Cached entries: {}", selector.cache.len());
    println!();

    // Cache lookup
    if let Some(&impl_id) = selector.cache.get(&("Display".into(), "Int".into())) {
        println!("Cache hit for (Display, Int): impl #{}", impl_id);
    }
    println!();

    println!("Performance characteristics:");
    println!("  Selection (uncached): <5ms");
    println!("  Selection (cached): <1ms");
    println!("  Lattice construction: <50ms (one-time)");
    println!("  Coherence checking: <100ms (compile-time)");
}

/// Example 5: Integration with TypeChecker (conceptual)
///
/// This demonstrates how the SpecializationSelector would be used within
/// the TypeChecker during method resolution.
#[allow(dead_code)]
fn example_type_checker_integration() {
    // This is conceptual - shows how it integrates

    println!("TypeChecker Integration:");
    println!();
    println!("During method call inference:");
    println!("  1. Infer receiver type: Int");
    println!("  2. Find protocol for method: Display");
    println!("  3. Select specialized implementation:");
    println!("     selector.select_implementation(Display, Int, ...)");
    println!("  4. Get method signature from selected impl");
    println!("  5. Instantiate and return type");
    println!();
    println!("Code example:");
    println!(
        r#"
    fn infer_method_call(
        &mut self,
        receiver: &Expr,
        method: &Text,
    ) -> Result<Type> {{
        // 1. Infer receiver type
        let receiver_ty = self.infer(receiver)?;

        // 2. Find protocol
        let protocol = self.find_protocol_for_method(method)?;

        // 3. Select specialized implementation
        let impl_id = self.spec_selector.select_implementation(
            &protocol,
            &receiver_ty,
            &self.protocol_checker,
            &mut self.unifier,
        )?;

        // 4. Get method signature
        let method_sig = self.get_method_signature(impl_id, method)?;

        // 5. Return type
        Ok(method_sig.return_type)
    }}
    "#
    );
}

/// Example 6: Coherence checking
#[allow(dead_code)]
fn example_coherence_checking() {
    println!("Coherence Checking:");
    println!();
    println!("Verifies no overlapping implementations without specialization:");
    println!();
    println!("✓ Valid:");
    println!("  implement<T> Display for T {{{{ }}}}");
    println!("  @specialize");
    println!("  implement Display for Int {{{{ }}}}");
    println!();
    println!("✗ Invalid:");
    println!("  implement Display for Int {{{{ }}}}");
    println!("  implement Display for Int {{{{ }}}}  // Error: overlap!");
    println!();
    println!("The CoherenceChecker detects these violations at compile time.");
}
