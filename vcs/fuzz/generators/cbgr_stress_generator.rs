//! CBGR stress test generator for Verum
//!
//! This module generates memory-intensive programs designed to stress-test
//! Verum's Checked Borrow with Generational References (CBGR) system.
//!
//! # CBGR Three-Tier Model
//!
//! - **Tier 0 (`&T`)**: Full CBGR protection with ~15ns overhead
//! - **Tier 1 (`&checked T`)**: Compiler-proven safe, zero overhead
//! - **Tier 2 (`&unsafe T`)**: Manual safety proof, zero overhead
//!
//! # Memory Layout
//!
//! - `ThinRef<T>`: 16 bytes (ptr + generation + epoch_caps)
//! - `FatRef<T>`: 24 bytes (ptr + generation + epoch_caps + len)
//!
//! # Test Patterns
//!
//! This generator creates programs that stress:
//! - Reference lifecycle management
//! - Generation counter overflow
//! - Epoch boundary transitions
//! - Mixed-tier interactions
//! - Deep reference chains
//! - Aliasing patterns
//! - Reference invalidation scenarios

use rand::Rng;
use rand::seq::IndexedRandom;
use std::collections::HashMap;

/// Configuration for CBGR stress tests
#[derive(Debug, Clone)]
pub struct CbgrStressConfig {
    /// Maximum depth of reference chains
    pub max_ref_depth: usize,
    /// Maximum number of live references
    pub max_live_refs: usize,
    /// Maximum heap allocation size
    pub max_heap_size: usize,
    /// Probability of using Tier 0 (managed) references
    pub tier0_probability: f64,
    /// Probability of using Tier 1 (checked) references
    pub tier1_probability: f64,
    /// Probability of using Tier 2 (unsafe) references
    pub tier2_probability: f64,
    /// Whether to generate aliasing patterns
    pub enable_aliasing: bool,
    /// Whether to generate cyclic reference tests
    pub enable_cycles: bool,
    /// Number of stress iterations in loops
    pub stress_iterations: usize,
    /// Maximum number of function calls (for stack stress)
    pub max_call_depth: usize,
}

impl Default for CbgrStressConfig {
    fn default() -> Self {
        Self {
            max_ref_depth: 10,
            max_live_refs: 50,
            max_heap_size: 1024 * 1024, // 1MB
            tier0_probability: 0.6,
            tier1_probability: 0.3,
            tier2_probability: 0.1,
            enable_aliasing: true,
            enable_cycles: true,
            stress_iterations: 1000,
            max_call_depth: 20,
        }
    }
}

/// Reference tier for code generation
#[derive(Debug, Clone, Copy, PartialEq)]
enum RefTier {
    Managed, // &T
    Checked, // &checked T
    Unsafe,  // &unsafe T
}

impl RefTier {
    fn syntax(&self, inner: &str) -> String {
        match self {
            RefTier::Managed => format!("&{}", inner),
            RefTier::Checked => format!("&checked {}", inner),
            RefTier::Unsafe => format!("&unsafe {}", inner),
        }
    }

    fn ref_name(&self) -> &'static str {
        match self {
            RefTier::Managed => "managed",
            RefTier::Checked => "checked",
            RefTier::Unsafe => "unsafe",
        }
    }
}

/// Tracks allocated references for stress testing
struct RefTracker {
    /// Map of variable name to reference info
    refs: HashMap<String, RefInfo>,
    /// Counter for unique names
    counter: usize,
    /// Current scope depth
    depth: usize,
}

/// Information about a tracked reference
#[derive(Debug, Clone)]
struct RefInfo {
    tier: RefTier,
    inner_type: String,
    is_mutable: bool,
    scope_depth: usize,
}

impl RefTracker {
    fn new() -> Self {
        Self {
            refs: HashMap::new(),
            counter: 0,
            depth: 0,
        }
    }

    fn fresh_ref(&mut self, tier: RefTier, inner_type: &str, is_mutable: bool) -> String {
        self.counter += 1;
        let name = format!("ref_{}_{}", tier.ref_name(), self.counter);
        self.refs.insert(
            name.clone(),
            RefInfo {
                tier,
                inner_type: inner_type.to_string(),
                is_mutable,
                scope_depth: self.depth,
            },
        );
        name
    }

    fn get_refs_of_tier(&self, tier: RefTier) -> Vec<String> {
        self.refs
            .iter()
            .filter(|(_, info)| info.tier == tier)
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn get_mutable_refs(&self) -> Vec<String> {
        self.refs
            .iter()
            .filter(|(_, info)| info.is_mutable)
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn push_scope(&mut self) {
        self.depth += 1;
    }

    fn pop_scope(&mut self) {
        let depth = self.depth;
        self.refs.retain(|_, info| info.scope_depth < depth);
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Generator for CBGR stress tests
pub struct CbgrStressGenerator {
    config: CbgrStressConfig,
}

impl CbgrStressGenerator {
    /// Create a new CBGR stress generator
    pub fn new(config: CbgrStressConfig) -> Self {
        Self { config }
    }

    /// Generate a complete CBGR stress test program
    pub fn generate_program<R: Rng>(&self, rng: &mut R) -> String {
        let mut tracker = RefTracker::new();
        let mut program = String::new();

        program.push_str("// CBGR stress test program\n");
        program.push_str("// Tests all three reference tiers under memory pressure\n\n");

        // Imports
        program.push_str("use verum_std::core::{List, Text, Map, Maybe}\n");
        program.push_str("use verum_std::memory::{Heap, Arena, Pool}\n");
        program.push_str("use verum_std::cbgr::{Managed, Checked, Unsafe}\n\n");

        // Generate data structures
        program.push_str(&self.generate_data_structures(rng));
        program.push('\n');

        // Generate helper functions
        program.push_str(&self.generate_helper_functions(rng, &mut tracker));
        program.push('\n');

        // Generate stress test functions
        program.push_str(&self.generate_stress_functions(rng, &mut tracker));
        program.push('\n');

        // Generate main
        program.push_str(&self.generate_main(rng, &mut tracker));

        program
    }

    /// Generate data structures for stress testing
    fn generate_data_structures<R: Rng>(&self, rng: &mut R) -> String {
        let mut result = String::new();

        // Node for linked structures
        result.push_str("/// Node for reference chain testing\n");
        result.push_str("struct Node<T> {\n");
        result.push_str("    value: T,\n");
        result.push_str("    next: Maybe<&Node<T>>,\n");
        result.push_str("    prev: Maybe<&Node<T>>,\n");
        result.push_str("}\n\n");

        // Tree node
        result.push_str("/// Tree node for depth testing\n");
        result.push_str("struct TreeNode<T> {\n");
        result.push_str("    value: T,\n");
        result.push_str("    children: List<&TreeNode<T>>,\n");
        result.push_str("    parent: Maybe<&TreeNode<T>>,\n");
        result.push_str("}\n\n");

        // Large struct for memory pressure
        let field_count = rng.random_range(10..30);
        result.push_str("/// Large struct for memory pressure testing\n");
        result.push_str("struct LargeStruct {\n");
        for i in 0..field_count {
            let field_type = ["Int", "Float", "Text", "Bool"].choose(rng).unwrap();
            result.push_str(&format!("    field_{}: {},\n", i, field_type));
        }
        result.push_str("}\n\n");

        // Mixed-tier container
        result.push_str("/// Container with mixed reference tiers\n");
        result.push_str("struct MixedContainer {\n");
        result.push_str("    managed_ref: &Int,\n");
        result.push_str("    checked_ref: &checked Int,\n");
        result.push_str("    data: List<Int>,\n");
        result.push_str("}\n\n");

        // Aliasing test struct
        if self.config.enable_aliasing {
            result.push_str("/// Struct for aliasing tests\n");
            result.push_str("struct AliasingTest {\n");
            result.push_str("    a: &mut Int,\n");
            result.push_str("    b: &Int,\n");
            result.push_str("    shared: &Int,\n");
            result.push_str("}\n\n");
        }

        result
    }

    /// Generate helper functions for stress testing
    fn generate_helper_functions<R: Rng>(&self, rng: &mut R, tracker: &mut RefTracker) -> String {
        let mut result = String::new();

        // Reference chain creator
        result.push_str("/// Create a chain of references\n");
        result.push_str("fn create_chain(len: Int) -> List<Node<Int>> {\n");
        result.push_str("    let mut nodes: List<Node<Int>> = [];\n");
        result.push_str("    for i in 0..len {\n");
        result.push_str("        let node = Node {\n");
        result.push_str("            value: i,\n");
        result.push_str("            next: None,\n");
        result.push_str("            prev: if i > 0 { Some(&nodes[i - 1]) } else { None },\n");
        result.push_str("        };\n");
        result.push_str("        if i > 0 {\n");
        result.push_str("            nodes[i - 1].next = Some(&node);\n");
        result.push_str("        }\n");
        result.push_str("        nodes.push(node);\n");
        result.push_str("    }\n");
        result.push_str("    nodes\n");
        result.push_str("}\n\n");

        // Deep tree creator
        result.push_str("/// Create a deep tree structure\n");
        result.push_str(&format!(
            "fn create_tree(depth: Int, branching: Int) -> TreeNode<Int> where depth <= {} {{\n",
            self.config.max_ref_depth
        ));
        result.push_str("    if depth == 0 {\n");
        result.push_str("        return TreeNode { value: 0, children: [], parent: None };\n");
        result.push_str("    }\n\n");
        result.push_str("    let mut node = TreeNode {\n");
        result.push_str("        value: depth,\n");
        result.push_str("        children: [],\n");
        result.push_str("        parent: None,\n");
        result.push_str("    };\n\n");
        result.push_str("    for i in 0..branching {\n");
        result.push_str("        let mut child = create_tree(depth - 1, branching);\n");
        result.push_str("        child.parent = Some(&node);\n");
        result.push_str("        node.children.push(&child);\n");
        result.push_str("    }\n\n");
        result.push_str("    node\n");
        result.push_str("}\n\n");

        // Reference passing functions for each tier
        result.push_str("/// Accept and return Tier 0 (managed) reference\n");
        result.push_str("fn pass_managed(r: &Int) -> &Int {\n");
        result.push_str("    r // CBGR validates generation\n");
        result.push_str("}\n\n");

        result.push_str("/// Accept and return Tier 1 (checked) reference\n");
        result.push_str("fn pass_checked(r: &checked Int) -> &checked Int {\n");
        result.push_str("    r // Compiler-verified safe\n");
        result.push_str("}\n\n");

        result.push_str("/// Accept Tier 2 (unsafe) reference\n");
        result.push_str("fn pass_unsafe(r: &unsafe Int) -> &unsafe Int {\n");
        result.push_str("    // SAFETY: Caller guarantees validity\n");
        result.push_str("    r\n");
        result.push_str("}\n\n");

        // Tier conversion functions
        result.push_str("/// Upgrade from Tier 0 to Tier 1 (when provably safe)\n");
        result.push_str("fn upgrade_to_checked(r: &Int) -> &checked Int {\n");
        result.push_str("    // Compiler analyzes and proves safety\n");
        result.push_str("    r.to_checked()\n");
        result.push_str("}\n\n");

        result.push_str("/// Downgrade from Tier 1 to Tier 0\n");
        result.push_str("fn downgrade_to_managed(r: &checked Int) -> &Int {\n");
        result.push_str("    r.to_managed()\n");
        result.push_str("}\n\n");

        // Mutable reference functions
        result.push_str("/// Mutate through Tier 0 reference\n");
        result.push_str("fn mutate_managed(r: &mut Int, value: Int) {\n");
        result.push_str("    *r = value;\n");
        result.push_str("}\n\n");

        result
    }

    /// Generate stress test functions
    fn generate_stress_functions<R: Rng>(&self, rng: &mut R, tracker: &mut RefTracker) -> String {
        let mut result = String::new();

        // Allocation stress
        result.push_str(&self.generate_allocation_stress(rng));
        result.push('\n');

        // Reference chain stress
        result.push_str(&self.generate_chain_stress(rng));
        result.push('\n');

        // Generation overflow test
        result.push_str(&self.generate_generation_overflow_test(rng));
        result.push('\n');

        // Mixed tier stress
        result.push_str(&self.generate_mixed_tier_stress(rng));
        result.push('\n');

        // Aliasing stress
        if self.config.enable_aliasing {
            result.push_str(&self.generate_aliasing_stress(rng));
            result.push('\n');
        }

        // Stack stress
        result.push_str(&self.generate_stack_stress(rng));
        result.push('\n');

        // Epoch boundary test
        result.push_str(&self.generate_epoch_test(rng));
        result.push('\n');

        result
    }

    /// Generate allocation stress test
    fn generate_allocation_stress<R: Rng>(&self, rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: rapid allocation and deallocation\n");
        result.push_str(&format!(
            "fn stress_allocation(iterations: Int) where iterations <= {} {{\n",
            self.config.stress_iterations
        ));

        result.push_str("    let mut live_refs: List<&Int> = [];\n\n");

        result.push_str("    for i in 0..iterations {\n");
        result.push_str("        // Allocate new reference\n");
        result.push_str("        let value = Heap::alloc(i);\n");
        result.push_str("        live_refs.push(&value);\n\n");

        result.push_str(&format!(
            "        // Periodically free old references (keep max {})\n",
            self.config.max_live_refs
        ));
        result.push_str(&format!(
            "        if len(live_refs) > {} {{\n",
            self.config.max_live_refs
        ));
        result.push_str("            let old = live_refs.remove(0);\n");
        result.push_str("            // Reference becomes invalid after this point\n");
        result.push_str("        }\n\n");

        result.push_str("        // Access random live reference to trigger CBGR check\n");
        result.push_str("        if len(live_refs) > 0 {\n");
        result.push_str("            let idx = i % len(live_refs);\n");
        result.push_str("            let _ = *live_refs[idx]; // CBGR validates\n");
        result.push_str("        }\n");
        result.push_str("    }\n");
        result.push_str("}\n");

        result
    }

    /// Generate reference chain stress test
    fn generate_chain_stress<R: Rng>(&self, rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: long reference chains\n");
        result.push_str(&format!(
            "fn stress_chain(length: Int) where length <= {} {{\n",
            self.config.max_ref_depth
        ));

        result.push_str("    let chain = create_chain(length);\n\n");

        result.push_str("    // Traverse forward\n");
        result.push_str("    let mut current = Some(&chain[0]);\n");
        result.push_str("    let mut count = 0;\n");
        result.push_str("    while let Some(node) = current {\n");
        result.push_str("        count = count + 1;\n");
        result.push_str("        assert node.value >= 0;\n");
        result.push_str("        current = node.next;\n");
        result.push_str("    }\n");
        result.push_str("    assert count == length;\n\n");

        result.push_str("    // Traverse backward\n");
        result.push_str("    current = Some(&chain[length - 1]);\n");
        result.push_str("    count = 0;\n");
        result.push_str("    while let Some(node) = current {\n");
        result.push_str("        count = count + 1;\n");
        result.push_str("        current = node.prev;\n");
        result.push_str("    }\n");
        result.push_str("    assert count == length;\n");
        result.push_str("}\n");

        result
    }

    /// Generate generation counter overflow test
    fn generate_generation_overflow_test<R: Rng>(&self, _rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: generation counter near overflow\n");
        result.push_str("fn stress_generation_overflow() {\n");
        result.push_str("    // Force many allocations to stress generation counter\n");
        result.push_str("    // Generation is typically 32-bit, so we need ~4B allocations\n");
        result.push_str("    // This test runs a realistic subset\n\n");

        result.push_str("    let arena = Arena::new(1024 * 1024); // 1MB arena\n\n");

        result.push_str("    for batch in 0..1000 {\n");
        result.push_str("        // Allocate batch\n");
        result.push_str("        for i in 0..1000 {\n");
        result.push_str("            let ptr = arena.alloc::<Int>();\n");
        result.push_str("            *ptr = batch * 1000 + i;\n");
        result.push_str("        }\n");
        result.push_str("        // Reset arena - all generations invalidated\n");
        result.push_str("        arena.reset();\n");
        result.push_str("    }\n\n");

        result.push_str("    // Verify generation tracking still works\n");
        result.push_str("    let fresh = arena.alloc::<Int>();\n");
        result.push_str("    *fresh = 42;\n");
        result.push_str("    assert *fresh == 42;\n");
        result.push_str("}\n");

        result
    }

    /// Generate mixed tier stress test
    fn generate_mixed_tier_stress<R: Rng>(&self, _rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: mixing all three reference tiers\n");
        result.push_str("fn stress_mixed_tiers() {\n");
        result.push_str("    let mut value: Int = 0;\n\n");

        result.push_str("    // Create references of each tier\n");
        result.push_str("    let managed: &Int = &value;           // Tier 0\n");
        result.push_str("    let checked: &checked Int = &value;   // Tier 1 (proven safe)\n\n");

        result.push_str("    // Pass through tier-specific functions\n");
        result.push_str("    let m1 = pass_managed(managed);\n");
        result.push_str("    let c1 = pass_checked(checked);\n\n");

        result.push_str("    // Verify values match\n");
        result.push_str("    assert *m1 == *c1;\n\n");

        result.push_str("    // Tier conversions\n");
        result.push_str("    let upgraded = upgrade_to_checked(managed);\n");
        result.push_str("    let downgraded = downgrade_to_managed(checked);\n\n");

        result.push_str("    // Create container with mixed tiers\n");
        result.push_str("    let container = MixedContainer {\n");
        result.push_str("        managed_ref: managed,\n");
        result.push_str("        checked_ref: checked,\n");
        result.push_str("        data: [1, 2, 3],\n");
        result.push_str("    };\n\n");

        result.push_str("    // Access through container\n");
        result.push_str("    assert *container.managed_ref == *container.checked_ref;\n");
        result.push_str("}\n");

        result
    }

    /// Generate aliasing stress test
    fn generate_aliasing_stress<R: Rng>(&self, _rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: aliasing patterns\n");
        result.push_str("fn stress_aliasing() {\n");
        result.push_str("    let mut data: List<Int> = [0, 1, 2, 3, 4];\n\n");

        result.push_str("    // Multiple immutable aliases (allowed)\n");
        result.push_str("    let r1 = &data[0];\n");
        result.push_str("    let r2 = &data[0];\n");
        result.push_str("    let r3 = &data[0];\n");
        result.push_str("    assert *r1 == *r2 && *r2 == *r3;\n\n");

        result.push_str("    // Different elements can have separate mutable refs\n");
        result.push_str("    let mr1 = &mut data[1];\n");
        result.push_str("    let mr2 = &mut data[2];\n");
        result.push_str("    *mr1 = 10;\n");
        result.push_str("    *mr2 = 20;\n\n");

        result.push_str("    // Scoped aliasing\n");
        result.push_str("    {\n");
        result.push_str("        let scoped = &data[0];\n");
        result.push_str("        assert *scoped == 0;\n");
        result.push_str("    } // scoped ref ends here\n\n");

        result.push_str("    // Now safe to mutate\n");
        result.push_str("    let mr0 = &mut data[0];\n");
        result.push_str("    *mr0 = 100;\n\n");

        result.push_str("    assert data == [100, 10, 20, 3, 4];\n");
        result.push_str("}\n");

        result
    }

    /// Generate stack stress test (deep recursion)
    fn generate_stack_stress<R: Rng>(&self, _rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: deep call stack with references\n");
        result.push_str(&format!(
            "fn stress_stack(depth: Int, r: &Int) -> Int where depth <= {} {{\n",
            self.config.max_call_depth
        ));
        result.push_str("    if depth == 0 {\n");
        result.push_str("        return *r;\n");
        result.push_str("    }\n\n");

        result.push_str("    // Create local reference that shadows\n");
        result.push_str("    let local = *r + depth;\n");
        result.push_str("    let local_ref = &local;\n\n");

        result.push_str("    // Recurse with both references\n");
        result.push_str("    let result = stress_stack(depth - 1, local_ref);\n\n");

        result.push_str("    // Verify original still valid\n");
        result.push_str("    assert *r >= 0;\n\n");

        result.push_str("    result\n");
        result.push_str("}\n");

        result
    }

    /// Generate epoch boundary test
    fn generate_epoch_test<R: Rng>(&self, _rng: &mut R) -> String {
        let mut result = String::new();

        result.push_str("/// Stress test: epoch boundaries\n");
        result.push_str("fn stress_epochs() {\n");
        result.push_str("    // Create references across multiple epochs\n");
        result.push_str("    let mut refs: List<&Int> = [];\n\n");

        result.push_str("    for epoch in 0..10 {\n");
        result.push_str("        // Start new epoch\n");
        result.push_str("        Cbgr::begin_epoch();\n\n");

        result.push_str("        // Allocate in this epoch\n");
        result.push_str("        for i in 0..100 {\n");
        result.push_str("            let value = Heap::alloc(epoch * 100 + i);\n");
        result.push_str("            refs.push(&value);\n");
        result.push_str("        }\n\n");

        result.push_str("        // Validate all refs (including cross-epoch)\n");
        result.push_str("        for r in &refs {\n");
        result.push_str("            let _ = **r; // CBGR checks epoch validity\n");
        result.push_str("        }\n\n");

        result.push_str("        // End epoch - some refs may be invalidated\n");
        result.push_str("        Cbgr::end_epoch();\n");
        result.push_str("    }\n");
        result.push_str("}\n");

        result
    }

    /// Generate main function
    fn generate_main<R: Rng>(&self, rng: &mut R, tracker: &mut RefTracker) -> String {
        let mut result = String::new();

        result.push_str("/// Run all CBGR stress tests\n");
        result.push_str("fn main() {\n");
        result.push_str("    println!(\"Starting CBGR stress tests...\");\n\n");

        result.push_str("    println!(\"Test 1: Allocation stress\");\n");
        result.push_str(&format!(
            "    stress_allocation({});\n",
            self.config.stress_iterations
        ));
        result.push_str("    println!(\"  PASSED\");\n\n");

        result.push_str("    println!(\"Test 2: Reference chain stress\");\n");
        result.push_str(&format!(
            "    stress_chain({});\n",
            self.config.max_ref_depth
        ));
        result.push_str("    println!(\"  PASSED\");\n\n");

        result.push_str("    println!(\"Test 3: Generation overflow stress\");\n");
        result.push_str("    stress_generation_overflow();\n");
        result.push_str("    println!(\"  PASSED\");\n\n");

        result.push_str("    println!(\"Test 4: Mixed tier stress\");\n");
        result.push_str("    stress_mixed_tiers();\n");
        result.push_str("    println!(\"  PASSED\");\n\n");

        if self.config.enable_aliasing {
            result.push_str("    println!(\"Test 5: Aliasing stress\");\n");
            result.push_str("    stress_aliasing();\n");
            result.push_str("    println!(\"  PASSED\");\n\n");
        }

        result.push_str("    println!(\"Test 6: Stack stress\");\n");
        result.push_str("    let base = 42;\n");
        result.push_str(&format!(
            "    let stack_result = stress_stack({}, &base);\n",
            self.config.max_call_depth
        ));
        result.push_str("    println!(\"  Stack result: {}\", stack_result);\n");
        result.push_str("    println!(\"  PASSED\");\n\n");

        result.push_str("    println!(\"Test 7: Epoch boundary stress\");\n");
        result.push_str("    stress_epochs();\n");
        result.push_str("    println!(\"  PASSED\");\n\n");

        result.push_str("    println!(\"All CBGR stress tests passed!\");\n");
        result.push_str("}\n");

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_cbgr_stress_generator() {
        let config = CbgrStressConfig::default();
        let generator = CbgrStressGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate_program(&mut rng);
        assert!(!program.is_empty());

        // Should contain CBGR-related constructs
        assert!(program.contains("&"));
        assert!(program.contains("stress"));
        assert!(program.contains("fn main()"));
    }

    #[test]
    fn test_ref_tier_syntax() {
        assert_eq!(RefTier::Managed.syntax("Int"), "&Int");
        assert_eq!(RefTier::Checked.syntax("Int"), "&checked Int");
        assert_eq!(RefTier::Unsafe.syntax("Int"), "&unsafe Int");
    }

    #[test]
    fn test_config_options() {
        let config = CbgrStressConfig {
            enable_aliasing: false,
            enable_cycles: false,
            ..Default::default()
        };
        let generator = CbgrStressGenerator::new(config);

        let mut rng = ChaCha8Rng::seed_from_u64(123);
        let program = generator.generate_program(&mut rng);

        // Should not contain aliasing tests when disabled
        assert!(!program.contains("stress_aliasing"));
    }
}
