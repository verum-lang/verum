//! SSA (Static Single Assignment) Representation for Escape Analysis
//!
//! Converts function representation to SSA form for precise data flow analysis.
//! This is Phase 1 of the 4-phase escape analysis pipeline defined by the CBGR
//! formal escape analysis specification. A reference escapes if it is: (1) returned
//! from a function, (2) stored in a heap-allocated structure, (3) captured by a
//! closure, or (4) passed to a function that may retain it. SSA form enables
//! precise tracking of these escape paths through use-def chains.
//!
//! SSA form ensures each variable is assigned exactly once, enabling precise
//! use-def chain analysis for escape detection. This is fundamental for the
//! 4-phase escape analysis algorithm:
//!
//! - Phase 1: Build SSA representation (this module)
//! - Phase 2: Track reference flow
//! - Phase 3: Dominance analysis
//! - Phase 4: Promotion decision
//!
//! # Algorithm Overview
//!
//! The SSA construction uses the classic algorithm from Cytron et al. (1991):
//!
//! 1. Compute dominance frontiers for each basic block
//! 2. Place phi nodes at dominance frontiers for each variable
//! 3. Rename variables using a stack-based approach
//! 4. Build use-def chains from the renamed program
//!
//! # Performance
//!
//! - Dominance frontier computation: O(|V| + |E|) where V = blocks, E = edges
//! - Phi placement: O(|V| * |defs|) in worst case
//! - Variable renaming: O(|instructions|)
//! - Total: O(|V|^2) worst case, typically linear in practice

use crate::analysis::{BlockId, ControlFlowGraph, DefSite, RefId, UseeSite};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use verum_common::{List, Text};
use verum_common::{Map, Set};

/// SSA Value representing a single definition
///
/// Each SSA value represents exactly one assignment point in the program.
/// For escape analysis, we track whether the value is a reference type
/// (subject to escape analysis) and all its use sites.
#[derive(Debug, Clone)]
pub struct SsaValue {
    /// Unique identifier for this value
    pub id: u32,
    /// Whether this value is a reference (subject to escape analysis)
    pub is_reference: bool,
    /// Original variable name (for debugging)
    pub name: Option<Text>,
    /// Where this value is defined
    pub definition: DefSite,
    /// All use sites of this value
    pub uses: List<UseeSite>,
    /// Kind of definition (regular, phi, parameter)
    pub def_kind: DefKind,
}

/// Kind of SSA value definition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    /// Regular definition from an instruction
    Regular,
    /// Phi node (merge point of multiple definitions)
    Phi,
    /// Function parameter
    Parameter,
    /// Return value placeholder
    Return,
    /// Heap store target
    HeapStore,
    /// Closure definition (anonymous function capturing environment)
    Closure,
}

/// SSA representation of a function
///
/// Contains all SSA values, use-def chains, def-use chains, and phi nodes.
/// This representation enables precise data flow analysis for escape detection.
#[derive(Debug)]
pub struct SsaFunction {
    /// All SSA values in the function
    pub values: Map<u32, SsaValue>,
    /// Use-def chains: for each use site, which value is used
    pub use_def: HashMap<UseSiteKey, u32>,
    /// Def-use chains: for each value, all its use sites
    pub def_use: Map<u32, Set<UseSiteKey>>,
    /// Phi nodes at block entries (`block_id` -> list of (`var_name`, `phi_value_id`))
    pub phi_nodes: Map<BlockId, List<PhiNode>>,
    /// Return values (SSA value IDs that flow to return)
    pub return_values: Set<u32>,
    /// Heap store targets (SSA value IDs stored to heap)
    pub heap_stores: Set<u32>,
    /// Values captured by closures
    pub closure_captures: Set<u32>,
    /// Values passed to thread spawns
    pub thread_escapes: Set<u32>,
    /// Next value ID to allocate
    next_id: u32,
    /// Original variable to SSA values mapping
    pub var_versions: Map<Text, List<u32>>,
}

/// Key for identifying use sites in maps
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UseSiteKey {
    /// Block containing the use
    pub block: BlockId,
    /// Instruction index within the block
    pub instruction: usize,
    /// Reference ID being used
    pub reference: RefId,
}

impl From<&UseeSite> for UseSiteKey {
    fn from(use_site: &UseeSite) -> Self {
        Self {
            block: use_site.block,
            instruction: 0, // Default instruction index
            reference: use_site.reference,
        }
    }
}

/// Phi node representing merge of multiple definitions
#[derive(Debug, Clone)]
pub struct PhiNode {
    /// Variable name this phi is for
    pub var_name: Text,
    /// SSA value ID for the phi result
    pub result_id: u32,
    /// Incoming values: (predecessor block, SSA value ID)
    pub incoming: List<(BlockId, u32)>,
}

impl SsaFunction {
    /// Create a new empty SSA function
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: Map::new(),
            use_def: HashMap::new(),
            def_use: Map::new(),
            phi_nodes: Map::new(),
            return_values: Set::new(),
            heap_stores: Set::new(),
            closure_captures: Set::new(),
            thread_escapes: Set::new(),
            next_id: 0,
            var_versions: Map::new(),
        }
    }

    /// Allocate a new value ID
    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Add an SSA value
    pub fn add_value(&mut self, value: SsaValue) {
        let id = value.id;
        if let Some(ref name) = value.name {
            self.var_versions.entry(name.clone()).or_default().push(id);
        }
        self.values.insert(id, value);
    }

    /// Record a use-def relationship
    pub fn record_use(&mut self, use_key: UseSiteKey, def_id: u32) {
        self.use_def.insert(use_key.clone(), def_id);
        self.def_use.entry(def_id).or_default().insert(use_key);
    }

    /// Get all reference values (for escape analysis)
    ///
    /// Returns only SSA values that represent references, which are the
    /// candidates for escape analysis and potential promotion.
    #[must_use]
    pub fn reference_values(&self) -> List<&SsaValue> {
        self.values.values().filter(|v| v.is_reference).collect()
    }

    /// Get use-def chain for a use site
    ///
    /// Returns the SSA value ID that provides the value at this use site.
    #[must_use]
    pub fn get_definition(&self, use_site: &UseeSite) -> Option<u32> {
        let key = UseSiteKey::from(use_site);
        self.use_def.get(&key).copied()
    }

    /// Get all uses of a value
    ///
    /// Returns all use sites that consume this SSA value.
    #[must_use]
    pub fn get_uses(&self, value_id: u32) -> Option<&Set<UseSiteKey>> {
        self.def_use.get(&value_id)
    }

    /// Check if value escapes via return
    ///
    /// A value escapes via return if it (or any value derived from it)
    /// is returned from the function.
    #[must_use]
    pub fn escapes_via_return(&self, value_id: u32) -> bool {
        // Direct check
        if self.return_values.contains(&value_id) {
            return true;
        }

        // Check phi nodes that might include this value
        for phi_nodes in self.phi_nodes.values() {
            for phi in phi_nodes {
                if phi.incoming.iter().any(|(_, id)| *id == value_id)
                    && self.return_values.contains(&phi.result_id)
                {
                    return true;
                }
            }
        }

        false
    }

    /// Check if value is stored to heap
    ///
    /// A value is stored to heap if it's written into a heap-allocated
    /// structure (Box, Heap, Arc, etc.).
    #[must_use]
    pub fn has_heap_store(&self, value_id: u32) -> bool {
        // Direct check
        if self.heap_stores.contains(&value_id) {
            return true;
        }

        // Check if value flows through phi to a heap store
        for phi_nodes in self.phi_nodes.values() {
            for phi in phi_nodes {
                if phi.incoming.iter().any(|(_, id)| *id == value_id)
                    && self.heap_stores.contains(&phi.result_id)
                {
                    return true;
                }
            }
        }

        false
    }

    /// Check if value is captured by a closure
    #[must_use]
    pub fn escapes_via_closure(&self, value_id: u32) -> bool {
        // Direct check
        if self.closure_captures.contains(&value_id) {
            return true;
        }

        // Check phi nodes
        for phi_nodes in self.phi_nodes.values() {
            for phi in phi_nodes {
                if phi.incoming.iter().any(|(_, id)| *id == value_id)
                    && self.closure_captures.contains(&phi.result_id)
                {
                    return true;
                }
            }
        }

        false
    }

    /// Check if value escapes via thread spawn
    #[must_use]
    pub fn escapes_via_thread(&self, value_id: u32) -> bool {
        // Direct check
        if self.thread_escapes.contains(&value_id) {
            return true;
        }

        // Check phi nodes
        for phi_nodes in self.phi_nodes.values() {
            for phi in phi_nodes {
                if phi.incoming.iter().any(|(_, id)| *id == value_id)
                    && self.thread_escapes.contains(&phi.result_id)
                {
                    return true;
                }
            }
        }

        false
    }

    /// Get the SSA value for a variable at a specific version
    #[must_use]
    pub fn get_version(&self, var_name: &str, version: usize) -> Option<u32> {
        let var_name_text: Text = var_name.to_string().into();
        self.var_versions
            .get(&var_name_text)
            .and_then(|versions| versions.get(version).copied())
    }

    /// Get all versions of a variable
    #[must_use]
    pub fn all_versions(&self, var_name: &str) -> Option<&List<u32>> {
        let var_name_text: Text = var_name.to_string().into();
        self.var_versions.get(&var_name_text)
    }

    /// Compute the reaching definition at a given block for a variable
    #[must_use]
    pub fn reaching_def(&self, var_name: &str, block: BlockId) -> Option<u32> {
        // Check phi nodes first
        if let Some(phis) = self.phi_nodes.get(&block) {
            for phi in phis {
                if phi.var_name == var_name {
                    return Some(phi.result_id);
                }
            }
        }

        // Otherwise return the most recent version
        let var_name_text: Text = var_name.to_string().into();
        self.var_versions
            .get(&var_name_text)
            .and_then(|versions| versions.last().copied())
    }
}

impl Default for SsaFunction {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing SSA form
///
/// Uses the Cytron et al. algorithm:
/// 1. Compute dominators and dominance frontiers
/// 2. Place phi nodes at dominance frontiers
/// 3. Rename variables using stack-based approach
pub struct SsaBuilder<'cfg> {
    /// Reference to the control flow graph
    cfg: &'cfg ControlFlowGraph,
    /// SSA function being built
    ssa: SsaFunction,
    /// Dominators: dom[b] = immediate dominator of b
    dominators: Map<BlockId, BlockId>,
    /// Dominator tree children: children[b] = blocks immediately dominated by b
    dom_tree_children: Map<BlockId, List<BlockId>>,
    /// Dominance frontiers: df[b] = dominance frontier of b
    dominance_frontiers: Map<BlockId, Set<BlockId>>,
    /// Variables defined in each block
    block_defs: Map<BlockId, Set<Text>>,
    /// All variables in the function
    all_variables: Set<Text>,
    /// Rename stacks: for each variable, stack of current SSA value IDs
    rename_stacks: HashMap<Text, List<u32>>,
    /// Blocks where each variable is defined
    var_def_blocks: HashMap<Text, HashSet<BlockId>>,
}

impl<'cfg> SsaBuilder<'cfg> {
    /// Create a new SSA builder
    #[must_use]
    pub fn new(cfg: &'cfg ControlFlowGraph) -> Self {
        Self {
            cfg,
            ssa: SsaFunction::new(),
            dominators: Map::new(),
            dom_tree_children: Map::new(),
            dominance_frontiers: Map::new(),
            block_defs: Map::new(),
            all_variables: Set::new(),
            rename_stacks: HashMap::new(),
            var_def_blocks: HashMap::new(),
        }
    }

    /// Build SSA form from CFG
    ///
    /// Implements the complete Cytron et al. algorithm for SSA construction.
    ///
    /// # Returns
    ///
    /// Returns the completed SSA function or an error if construction fails.
    ///
    /// # Algorithm
    ///
    /// 1. Compute dominators using iterative algorithm
    /// 2. Build dominator tree
    /// 3. Compute dominance frontiers
    /// 4. Place phi nodes using iterated dominance frontier
    /// 5. Rename variables using depth-first traversal of dominator tree
    /// 6. Build use-def chains
    pub fn build(mut self) -> Result<SsaFunction, SsaError> {
        // Validate CFG
        if self.cfg.blocks.is_empty() {
            return Err(SsaError::InvalidCfg("Empty CFG".to_string().into()));
        }

        if !self.cfg.blocks.contains_key(&self.cfg.entry) {
            return Err(SsaError::NoEntryBlock);
        }

        // Step 1: Collect variable information
        self.collect_variables();

        // Step 2: Compute dominators
        self.compute_dominators();

        // Step 3: Build dominator tree
        self.build_dominator_tree();

        // Step 4: Compute dominance frontiers
        self.compute_dominance_frontiers_algorithm();

        // Step 5: Place phi nodes
        self.place_phi_nodes();

        // Step 6: Rename variables (create SSA values)
        self.rename_variables();

        // Step 7: Build use-def chains
        self.build_use_def_chains();

        // Step 8: Analyze escape patterns
        self.analyze_escape_patterns();

        Ok(self.ssa)
    }

    /// Collect all variables and their definition sites
    fn collect_variables(&mut self) {
        for (block_id, block) in &self.cfg.blocks {
            let mut block_vars = Set::new();

            for def in &block.definitions {
                // Generate variable name from RefId
                let var_name: Text = format!("ref_{}", def.reference.0).into();
                block_vars.insert(var_name.clone());
                self.all_variables.insert(var_name.clone());

                self.var_def_blocks
                    .entry(var_name)
                    .or_default()
                    .insert(*block_id);
            }

            self.block_defs.insert(*block_id, block_vars);
        }
    }

    /// Compute dominators using the iterative algorithm
    ///
    /// Uses the Cooper-Harvey-Kennedy algorithm which is efficient in practice.
    ///
    /// # Safety
    /// Includes iteration limit to guarantee termination on malformed CFGs.
    fn compute_dominators(&mut self) {
        // Get blocks in reverse postorder for efficient iteration
        let rpo = self.reverse_postorder();

        // Initialize: entry dominates only itself
        self.dominators.insert(self.cfg.entry, self.cfg.entry);

        // Iterate until fixed point with safety limit
        // The algorithm should converge in O(n) iterations for well-formed CFGs
        let max_iterations = self.cfg.blocks.len() * self.cfg.blocks.len() + 10;
        let mut iteration_count = 0;
        let mut changed = true;

        while changed && iteration_count < max_iterations {
            iteration_count += 1;
            changed = false;

            for &block_id in &rpo {
                if block_id == self.cfg.entry {
                    continue;
                }

                if let Some(block) = self.cfg.blocks.get(&block_id) {
                    // Find new immediate dominator
                    let mut new_idom: Option<BlockId> = None;

                    for &pred_id in &block.predecessors {
                        if self.dominators.contains_key(&pred_id) {
                            new_idom = Some(match new_idom {
                                None => pred_id,
                                Some(idom) => self.intersect(pred_id, idom),
                            });
                        }
                    }

                    if let Some(idom) = new_idom
                        && self.dominators.get(&block_id) != Some(&idom)
                    {
                        self.dominators.insert(block_id, idom);
                        changed = true;
                    }
                }
            }
        }
    }

    /// Find the nearest common dominator (intersect in dominator algorithm)
    ///
    /// # Safety
    /// Includes iteration limit to prevent infinite loops on malformed CFGs.
    fn intersect(&self, mut b1: BlockId, mut b2: BlockId) -> BlockId {
        let postorder = self.postorder_numbers();
        let max_iterations = self.cfg.blocks.len() * 2 + 10;
        let mut iterations = 0;

        while b1 != b2 && iterations < max_iterations {
            iterations += 1;
            let n1 = postorder.get(&b1).copied().unwrap_or(0);
            let n2 = postorder.get(&b2).copied().unwrap_or(0);

            // Move b2 up the dominator tree while its postorder number is higher
            while n1 < n2 {
                if let Some(&dom) = self.dominators.get(&b2) {
                    if dom == b2 {
                        // Self-loop in dominator chain - break to avoid infinite loop
                        break;
                    }
                    b2 = dom;
                } else {
                    break;
                }
                let new_n2 = postorder.get(&b2).copied().unwrap_or(0);
                if new_n2 >= n2 {
                    break;
                }
            }

            let n1 = postorder.get(&b1).copied().unwrap_or(0);
            let n2 = postorder.get(&b2).copied().unwrap_or(0);

            // Move b1 up the dominator tree while its postorder number is higher
            while n2 < n1 {
                if let Some(&dom) = self.dominators.get(&b1) {
                    if dom == b1 {
                        // Self-loop in dominator chain - break to avoid infinite loop
                        break;
                    }
                    b1 = dom;
                } else {
                    break;
                }
                let new_n1 = postorder.get(&b1).copied().unwrap_or(0);
                if new_n1 >= n1 {
                    break;
                }
            }
        }

        b1
    }

    /// Compute postorder numbers for blocks
    fn postorder_numbers(&self) -> HashMap<BlockId, usize> {
        let mut numbers = HashMap::new();
        let mut visited = HashSet::new();
        let mut counter = 0;

        self.postorder_visit(self.cfg.entry, &mut visited, &mut numbers, &mut counter);

        numbers
    }

    /// Recursive postorder traversal
    fn postorder_visit(
        &self,
        block_id: BlockId,
        visited: &mut HashSet<BlockId>,
        numbers: &mut HashMap<BlockId, usize>,
        counter: &mut usize,
    ) {
        if visited.contains(&block_id) {
            return;
        }
        visited.insert(block_id);

        if let Some(block) = self.cfg.blocks.get(&block_id) {
            for &succ in &block.successors {
                self.postorder_visit(succ, visited, numbers, counter);
            }
        }

        numbers.insert(block_id, *counter);
        *counter += 1;
    }

    /// Compute reverse postorder for efficient dominator calculation
    fn reverse_postorder(&self) -> List<BlockId> {
        let mut result = List::new();
        let mut visited = HashSet::new();

        self.reverse_postorder_visit(self.cfg.entry, &mut visited, &mut result);

        result.reverse();
        result
    }

    /// Recursive reverse postorder traversal
    fn reverse_postorder_visit(
        &self,
        block_id: BlockId,
        visited: &mut HashSet<BlockId>,
        result: &mut List<BlockId>,
    ) {
        if visited.contains(&block_id) {
            return;
        }
        visited.insert(block_id);

        if let Some(block) = self.cfg.blocks.get(&block_id) {
            for &succ in &block.successors {
                self.reverse_postorder_visit(succ, visited, result);
            }
        }

        result.push(block_id);
    }

    /// Build dominator tree from computed dominators
    fn build_dominator_tree(&mut self) {
        // Initialize children map
        for &block_id in self.cfg.blocks.keys() {
            self.dom_tree_children.insert(block_id, List::new());
        }

        // Add each block to its dominator's children
        for (&block_id, &dom_id) in &self.dominators {
            if block_id != dom_id {
                // Not the entry block dominating itself
                self.dom_tree_children
                    .entry(dom_id)
                    .or_insert_with(List::new)
                    .push(block_id);
            }
        }
    }

    /// Compute dominance frontiers using the Cytron algorithm
    ///
    /// DF(X) = {Y : X dominates a predecessor of Y but does not strictly dominate Y}
    fn compute_dominance_frontiers_algorithm(&mut self) {
        // Initialize empty frontiers
        for &block_id in self.cfg.blocks.keys() {
            self.dominance_frontiers.insert(block_id, Set::new());
        }

        // Compute frontiers bottom-up
        for &block_id in self.cfg.blocks.keys() {
            if let Some(block) = self.cfg.blocks.get(&block_id) {
                // For each predecessor
                if block.predecessors.len() >= 2 {
                    // Block has multiple predecessors, so it's a join point
                    for &pred in &block.predecessors {
                        let mut runner = pred;
                        // Walk up dominator tree until we reach block's dominator
                        while let Some(&dom) = self.dominators.get(&block_id) {
                            if runner == dom {
                                break;
                            }

                            // Add block_id to runner's frontier
                            self.dominance_frontiers
                                .entry(runner)
                                .or_default()
                                .insert(block_id);

                            // Move up to runner's dominator
                            if let Some(&runner_dom) = self.dominators.get(&runner) {
                                if runner_dom == runner {
                                    break; // At entry
                                }
                                runner = runner_dom;
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Place phi nodes using the iterated dominance frontier algorithm
    ///
    /// For each variable, place phi nodes at the iterated dominance frontier
    /// of all its definition sites.
    fn place_phi_nodes(&mut self) {
        for var_name in &self.all_variables {
            let mut work_list: VecDeque<BlockId> = VecDeque::new();
            let mut has_phi: HashSet<BlockId> = HashSet::new();
            let mut ever_on_work_list: HashSet<BlockId> = HashSet::new();

            // Initialize work list with blocks that define this variable
            if let Some(def_blocks) = self.var_def_blocks.get(var_name) {
                for &block_id in def_blocks {
                    work_list.push_back(block_id);
                    ever_on_work_list.insert(block_id);
                }
            }

            // Iterate through dominance frontiers
            while let Some(block_id) = work_list.pop_front() {
                if let Some(frontier) = self.dominance_frontiers.get(&block_id) {
                    for &y in frontier {
                        if !has_phi.contains(&y) {
                            // Place phi node at Y
                            let phi_id = self.ssa.alloc_id();

                            let phi = PhiNode {
                                var_name: var_name.clone(),
                                result_id: phi_id,
                                incoming: List::new(), // Will be filled during renaming
                            };

                            self.ssa.phi_nodes.entry(y).or_insert_with(List::new).push(phi);

                            has_phi.insert(y);

                            if !ever_on_work_list.contains(&y) {
                                work_list.push_back(y);
                                ever_on_work_list.insert(y);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Rename variables using depth-first traversal of dominator tree
    ///
    /// This creates unique SSA values for each definition and fills in
    /// phi node operands.
    fn rename_variables(&mut self) {
        // Initialize rename stacks
        for var_name in &self.all_variables {
            self.rename_stacks
                .insert(var_name.clone(), List::<u32>::new());
        }

        // Start renaming from entry block
        self.rename_block(self.cfg.entry);
    }

    /// Rename variables in a block and recursively process dominated blocks
    fn rename_block(&mut self, block_id: BlockId) {
        // Track how many values we push to each stack (for cleanup)
        let mut pushed: HashMap<Text, usize> = HashMap::new();

        // Process phi nodes at the start of this block
        if let Some(phis) = self.ssa.phi_nodes.get(&block_id).cloned() {
            for phi in phis {
                // Create SSA value for phi result
                let def_site = DefSite {
                    block: block_id,
                    reference: RefId(u64::from(phi.result_id)),
                    is_stack_allocated: true, // Phi results are typically stack values
                    span: None, // Phi nodes don't have a specific source span
                };

                let value = SsaValue {
                    id: phi.result_id,
                    is_reference: true, // Assume phi might be a reference
                    name: Some(phi.var_name.clone()),
                    definition: def_site,
                    uses: List::new(),
                    def_kind: DefKind::Phi,
                };

                self.ssa.add_value(value);

                // Push to rename stack
                if let Some(stack) = self.rename_stacks.get_mut(&phi.var_name) {
                    stack.push(phi.result_id);
                    *pushed.entry(phi.var_name.clone()).or_insert(0) += 1;
                }
            }
        }

        // Process definitions in this block
        if let Some(block) = self.cfg.blocks.get(&block_id) {
            for def in &block.definitions {
                let var_name: Text = format!("ref_{}", def.reference.0).into();
                let value_id = self.ssa.alloc_id();

                let value = SsaValue {
                    id: value_id,
                    is_reference: true,
                    name: Some(var_name.clone()),
                    definition: def.clone(),
                    uses: List::new(),
                    def_kind: if def.is_stack_allocated {
                        DefKind::Regular
                    } else {
                        DefKind::HeapStore
                    },
                };

                self.ssa.add_value(value);

                // Push to rename stack
                if let Some(stack) = self.rename_stacks.get_mut(&var_name) {
                    stack.push(value_id);
                    *pushed.entry(var_name.into()).or_insert(0) += 1;
                }
            }
        }

        // Fill in phi operands for successors
        if let Some(block) = self.cfg.blocks.get(&block_id) {
            for &succ_id in &block.successors {
                if let Some(succ_phis) = self.ssa.phi_nodes.get_mut(&succ_id) {
                    for phi in succ_phis {
                        // Get current version from stack
                        if let Some(stack) = self.rename_stacks.get(&phi.var_name)
                            && let Some(&value_id) = stack.last()
                        {
                            phi.incoming.push((block_id, value_id));
                        }
                    }
                }
            }
        }

        // Recursively process dominated blocks
        if let Some(children) = self.dom_tree_children.get(&block_id).cloned() {
            for child in children {
                self.rename_block(child);
            }
        }

        // Pop values we pushed (cleanup for this block)
        for (var_name, count) in pushed {
            if let Some(stack) = self.rename_stacks.get_mut(&var_name) {
                for _ in 0..count {
                    stack.pop();
                }
            }
        }
    }

    /// Build use-def chains from the SSA representation
    fn build_use_def_chains(&mut self) {
        for (block_id, block) in &self.cfg.blocks {
            for use_site in &block.uses {
                let var_name: Text = format!("ref_{}", use_site.reference.0).into();

                // Find the reaching definition
                if let Some(stack) = self.rename_stacks.get(&var_name)
                    && let Some(&def_id) = stack.last()
                {
                    let key = UseSiteKey {
                        block: *block_id,
                        instruction: 0, // Would need instruction index in real impl
                        reference: use_site.reference,
                    };

                    self.ssa.record_use(key, def_id);
                }
            }
        }
    }

    /// Analyze escape patterns in the SSA representation
    fn analyze_escape_patterns(&mut self) {
        // Identify return values
        if let Some(exit_block) = self.cfg.blocks.get(&self.cfg.exit) {
            for use_site in &exit_block.uses {
                let key = UseSiteKey::from(use_site);
                if let Some(&def_id) = self.ssa.use_def.get(&key) {
                    self.ssa.return_values.insert(def_id);
                }
            }
        }

        // Identify heap stores (non-stack allocated definitions)
        for value in self.ssa.values.values() {
            if !value.definition.is_stack_allocated {
                self.ssa.heap_stores.insert(value.id);
            }
            if value.def_kind == DefKind::HeapStore {
                self.ssa.heap_stores.insert(value.id);
            }
        }
    }
}

/// Errors that can occur during SSA construction
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsaError {
    /// CFG is invalid
    InvalidCfg(Text),
    /// No entry block found
    NoEntryBlock,
    /// Cyclic definition detected (should not happen in valid SSA)
    CyclicDefinition(Text),
    /// Variable used before definition
    UndefinedVariable(Text),
}

impl fmt::Display for SsaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SsaError::InvalidCfg(msg) => write!(f, "Invalid CFG: {msg}"),
            SsaError::NoEntryBlock => write!(f, "No entry block found in CFG"),
            SsaError::CyclicDefinition(var) => {
                write!(f, "Cyclic definition detected for variable: {var}")
            }
            SsaError::UndefinedVariable(var) => {
                write!(f, "Variable used before definition: {var}")
            }
        }
    }
}

impl std::error::Error for SsaError {}

/// Helper trait for building SSA from analysis results
pub trait SsaBuildable {
    /// Build SSA representation for this function/block
    fn build_ssa(&self) -> Result<SsaFunction, SsaError>;
}

impl SsaBuildable for ControlFlowGraph {
    fn build_ssa(&self) -> Result<SsaFunction, SsaError> {
        SsaBuilder::new(self).build()
    }
}

/// SSA-based escape analysis results
#[derive(Debug, Clone)]
pub struct SsaEscapeInfo {
    /// SSA value ID
    pub value_id: u32,
    /// Whether value escapes via return
    pub returns: bool,
    /// Whether value is stored to heap
    pub heap_stored: bool,
    /// Whether value is captured by closure
    pub closure_captured: bool,
    /// Whether value escapes to another thread
    pub thread_escaped: bool,
    /// Summary: does the value escape at all?
    pub escapes: bool,
}

impl SsaFunction {
    /// Analyze escape information for a specific SSA value
    #[must_use]
    pub fn analyze_escape(&self, value_id: u32) -> SsaEscapeInfo {
        let returns = self.escapes_via_return(value_id);
        let heap_stored = self.has_heap_store(value_id);
        let closure_captured = self.escapes_via_closure(value_id);
        let thread_escaped = self.escapes_via_thread(value_id);

        let escapes = returns || heap_stored || closure_captured || thread_escaped;

        SsaEscapeInfo {
            value_id,
            returns,
            heap_stored,
            closure_captured,
            thread_escaped,
            escapes,
        }
    }

    /// Analyze all reference values for escape
    #[must_use]
    pub fn analyze_all_escapes(&self) -> List<SsaEscapeInfo> {
        self.reference_values()
            .iter()
            .map(|v| self.analyze_escape(v.id))
            .collect()
    }
}
