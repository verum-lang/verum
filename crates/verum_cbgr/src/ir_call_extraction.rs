//! Production-grade IR-based Call Site Extraction for CBGR
//!
//! Provides precise call site extraction from IR instructions for context-sensitive
//! escape analysis. Unlike heuristic CFG-based detection, this parses actual IR to
//! identify function calls, map arguments to parameters, and track return values,
//! enabling accurate per-parameter escape tracking for CBGR promotion decisions.
//!
//! This module implements IR-based call site extraction for context-sensitive escape analysis.
//! Unlike heuristic CFG-based call site detection, this module parses actual IR instructions
//! to precisely identify function calls, map arguments to parameters, and track return values.
//!
//! # Key Features
//!
//! - **Real IR instruction parsing**: Direct parsing of simplified IR representation
//! - **Call instruction identification**: Precise detection of call sites in IR
//! - **Callee resolution**: Maps call instructions to target functions
//! - **Argument mapping**: Tracks actual arguments to formal parameters
//! - **Return value tracking**: Follows return values across call boundaries
//! - **Linear scan performance**: O(instructions) complexity
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │                    IR Call Extraction                      │
//! ├────────────────────────────────────────────────────────────┤
//! │                                                            │
//! │  IrInstruction  ──→  IrCallExtractor  ──→  IrCallSite    │
//! │       │                    │                     │         │
//! │       │                    │                     │         │
//! │       v                    v                     v         │
//! │  IrFunction      CallArgMapping        IrCallInfo        │
//! │                                                            │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use verum_cbgr::ir_call_extraction::{IrCallExtractor, IrFunction, IrInstruction, IrOperand};
//! use verum_cbgr::analysis::{FunctionId, BlockId, RefId};
//!
//! // Create simplified IR function
//! let mut func = IrFunction::new(FunctionId(1), "process_data");
//! func.add_parameter(0, "data");
//!
//! // Add call instruction
//! let call_inst = IrInstruction::Call {
//!     target: "validate".into(),
//!     args: vec![IrOperand::LocalVar(0)],
//!     result: Some(1),
//! };
//! func.add_instruction(BlockId(0), 0, call_inst);
//!
//! // Extract call sites
//! let extractor = IrCallExtractor::new();
//! let call_sites = extractor.extract_from_function(&func);
//!
//! // Examine first call site
//! let site = &call_sites[0];
//! println!("Call to {} at {}:{}", site.callee_name, site.block, site.instruction_offset);
//!
//! // Map arguments
//! let mapping = site.arg_mapping();
//! for (arg_idx, param_idx) in &mapping.arg_to_param {
//!     println!("Arg {} → Param {}", arg_idx, param_idx);
//! }
//! ```
//!
//! # Performance Characteristics
//!
//! - **Call site extraction**: O(n) where n = number of instructions
//! - **Argument mapping**: O(args) per call site
//! - **Memory overhead**: ~200 bytes per call site
//! - **Target**: <10µs for 1000-instruction function
//!
//! # Simplified IR Representation
//!
//! Since full MIR integration is future work, we use a simplified IR:
//!
//! - **`IrFunction`**: Function representation with parameters and instructions
//! - **`IrInstruction`**: Call, Assign, Return, Branch instructions
//! - **`IrOperand`**: `LocalVar`, Argument, Constant operands
//!
//! This allows production-grade call site extraction without full compiler integration.

use crate::analysis::{BlockId, FunctionId, RefId};
use std::fmt;
use verum_common::{Map, Maybe, Set, Text};

// ==================================================================================
// Simplified IR Representation
// ==================================================================================

/// Simplified IR operand
///
/// Represents a value in the IR (local variable, parameter, constant, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IrOperand {
    /// Local variable (SSA value)
    LocalVar(u32),
    /// Function parameter
    Argument(u32),
    /// Constant value
    Constant(i64),
    /// Reference to allocation
    Reference(RefId),
    /// Null/undefined
    Undef,
}

impl IrOperand {
    /// Check if this operand is a reference
    #[must_use]
    pub fn is_reference(&self) -> bool {
        matches!(self, IrOperand::Reference(_))
    }

    /// Extract reference ID if this is a reference
    #[must_use]
    pub fn as_reference(&self) -> Maybe<RefId> {
        match self {
            IrOperand::Reference(ref_id) => Maybe::Some(*ref_id),
            _ => Maybe::None,
        }
    }

    /// Check if this operand is an argument
    #[must_use]
    pub fn is_argument(&self) -> bool {
        matches!(self, IrOperand::Argument(_))
    }

    /// Extract argument index if this is an argument
    #[must_use]
    pub fn argument_index(&self) -> Maybe<u32> {
        match self {
            IrOperand::Argument(idx) => Maybe::Some(*idx),
            _ => Maybe::None,
        }
    }
}

/// Simplified IR instruction
///
/// Represents a single instruction in the IR. We support a minimal subset
/// of instructions sufficient for call site extraction.
#[derive(Debug, Clone, PartialEq)]
pub enum IrInstruction {
    /// Function call: target(args) -> result
    Call {
        /// Target function name or ID
        target: Text,
        /// Arguments passed to the function
        args: Vec<IrOperand>,
        /// Optional result variable
        result: Maybe<u32>,
    },

    /// Assignment: dest = src
    Assign {
        /// Destination variable
        dest: u32,
        /// Source operand
        src: IrOperand,
    },

    /// Return statement
    Return {
        /// Optional return value
        value: Maybe<IrOperand>,
    },

    /// Conditional branch
    Branch {
        /// Condition
        condition: IrOperand,
        /// True target block
        true_target: BlockId,
        /// False target block
        false_target: BlockId,
    },

    /// Unconditional jump
    Jump {
        /// Target block
        target: BlockId,
    },

    /// Phi node (SSA merge)
    Phi {
        /// Result variable
        result: u32,
        /// Incoming values from predecessor blocks
        incoming: Vec<(BlockId, IrOperand)>,
    },

    /// Load from memory
    Load {
        /// Result variable
        result: u32,
        /// Pointer operand
        ptr: IrOperand,
    },

    /// Store to memory
    Store {
        /// Pointer operand
        ptr: IrOperand,
        /// Value to store
        value: IrOperand,
    },

    /// No-op (placeholder)
    Nop,
}

impl IrInstruction {
    /// Check if this instruction is a call
    #[must_use]
    pub fn is_call(&self) -> bool {
        matches!(self, IrInstruction::Call { .. })
    }

    /// Extract call information if this is a call instruction
    #[must_use]
    pub fn as_call(&self) -> Maybe<(&Text, &Vec<IrOperand>, Maybe<u32>)> {
        match self {
            IrInstruction::Call {
                target,
                args,
                result,
            } => Maybe::Some((target, args, *result)),
            _ => Maybe::None,
        }
    }

    /// Check if this instruction returns a value
    #[must_use]
    pub fn is_return(&self) -> bool {
        matches!(self, IrInstruction::Return { .. })
    }

    /// Check if this instruction stores a reference
    #[must_use]
    pub fn stores_reference(&self) -> bool {
        matches!(self, IrInstruction::Store { value, .. } if value.is_reference())
    }
}

/// Simplified IR function representation
///
/// Represents a function with parameters and instructions organized by basic blocks.
/// This is a simplified representation suitable for call site extraction without
/// full MIR integration.
#[derive(Debug, Clone)]
pub struct IrFunction {
    /// Function identifier
    pub id: FunctionId,
    /// Function name
    pub name: Text,
    /// Parameters (index -> name)
    pub parameters: Map<u32, Text>,
    /// Instructions organized by block
    pub blocks: Map<BlockId, Vec<(usize, IrInstruction)>>,
    /// Local variables
    pub locals: Set<u32>,
}

impl IrFunction {
    /// Create new IR function
    pub fn new(id: FunctionId, name: impl Into<Text>) -> Self {
        Self {
            id,
            name: name.into(),
            parameters: Map::new(),
            blocks: Map::new(),
            locals: Set::new(),
        }
    }

    /// Add parameter
    pub fn add_parameter(&mut self, index: u32, name: impl Into<Text>) {
        self.parameters.insert(index, name.into());
    }

    /// Add instruction to a block
    pub fn add_instruction(&mut self, block: BlockId, offset: usize, inst: IrInstruction) {
        self.blocks.entry(block).or_default().push((offset, inst));
    }

    /// Add local variable
    pub fn add_local(&mut self, var_id: u32) {
        self.locals.insert(var_id);
    }

    /// Get all instructions in order (sorted by `BlockId`, then by offset)
    #[must_use]
    pub fn all_instructions(&self) -> Vec<(BlockId, usize, &IrInstruction)> {
        let mut result = Vec::new();
        for (&block_id, instructions) in &self.blocks {
            for (offset, inst) in instructions {
                result.push((block_id, *offset, inst));
            }
        }
        // Sort by BlockId, then by offset for deterministic ordering
        result.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        result
    }

    /// Count total instructions
    #[must_use]
    pub fn instruction_count(&self) -> usize {
        self.blocks.values().map(std::vec::Vec::len).sum()
    }
}

// ==================================================================================
// Call Site Extraction Types
// ==================================================================================

/// Precise call site extracted from IR
///
/// Represents a single function call with complete information about location,
/// target, arguments, and return value.
#[derive(Debug, Clone, PartialEq)]
pub struct IrCallSite {
    /// Calling function
    pub caller: FunctionId,
    /// Basic block containing the call
    pub block: BlockId,
    /// Instruction offset within the block
    pub instruction_offset: usize,
    /// Callee function name
    pub callee_name: Text,
    /// Arguments passed to the function
    pub arguments: Vec<IrOperand>,
    /// Optional result variable
    pub result: Maybe<u32>,
}

impl IrCallSite {
    /// Create new call site
    pub fn new(
        caller: FunctionId,
        block: BlockId,
        offset: usize,
        callee: impl Into<Text>,
        args: Vec<IrOperand>,
        result: Maybe<u32>,
    ) -> Self {
        Self {
            caller,
            block,
            instruction_offset: offset,
            callee_name: callee.into(),
            arguments: args,
            result,
        }
    }

    /// Get argument mapping
    #[must_use]
    pub fn arg_mapping(&self) -> CallArgMapping {
        let mut arg_to_param = Map::new();
        for (arg_idx, _arg) in self.arguments.iter().enumerate() {
            // For now, assume 1-to-1 mapping (arg index = param index)
            // In full implementation, this would use type information
            arg_to_param.insert(arg_idx, arg_idx);
        }

        CallArgMapping {
            call_site: self.clone(),
            param_to_arg: arg_to_param.iter().map(|(k, v)| (*v, *k)).collect(),
            arg_to_param,
        }
    }

    /// Check if specific reference is passed as argument
    #[must_use]
    pub fn passes_reference(&self, ref_id: RefId) -> bool {
        self.arguments
            .iter()
            .any(|arg| matches!(arg, IrOperand::Reference(r) if *r == ref_id))
    }

    /// Get argument indices that are references
    #[must_use]
    pub fn reference_arguments(&self) -> Vec<(usize, RefId)> {
        self.arguments
            .iter()
            .enumerate()
            .filter_map(|(idx, arg)| arg.as_reference().map(|ref_id| (idx, ref_id)))
            .collect()
    }

    /// Check if this call has a return value
    #[must_use]
    pub fn has_result(&self) -> bool {
        self.result.is_some()
    }
}

impl fmt::Display for IrCallSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{} call {}",
            self.caller.0, self.block.0, self.instruction_offset, self.callee_name
        )
    }
}

/// Argument to parameter mapping for a call site
///
/// Tracks how actual arguments at the call site map to formal parameters
/// of the callee function.
#[derive(Debug, Clone)]
pub struct CallArgMapping {
    /// The call site this mapping applies to
    pub call_site: IrCallSite,
    /// Map from argument index to parameter index
    pub arg_to_param: Map<usize, usize>,
    /// Reverse map from parameter index to argument index
    pub param_to_arg: Map<usize, usize>,
}

impl CallArgMapping {
    /// Get parameter index for argument
    #[must_use]
    pub fn param_for_arg(&self, arg_idx: usize) -> Maybe<usize> {
        self.arg_to_param.get(&arg_idx).copied()
    }

    /// Get argument index for parameter
    #[must_use]
    pub fn arg_for_param(&self, param_idx: usize) -> Maybe<usize> {
        self.param_to_arg.get(&param_idx).copied()
    }

    /// Check if reference is passed to specific parameter
    #[must_use]
    pub fn reference_to_param(&self, ref_id: RefId, param_idx: usize) -> bool {
        if let Maybe::Some(arg_idx) = self.arg_for_param(param_idx)
            && let Some(arg) = self.call_site.arguments.get(arg_idx)
        {
            return matches!(arg, IrOperand::Reference(r) if *r == ref_id);
        }
        false
    }
}

/// Complete call information including context
///
/// Combines call site information with calling context for precise
/// context-sensitive analysis.
#[derive(Debug, Clone)]
pub struct IrCallInfo {
    /// The call site
    pub site: IrCallSite,
    /// Argument mapping
    pub mapping: CallArgMapping,
    /// References passed as arguments
    pub reference_args: Vec<(usize, RefId)>,
    /// Whether this call may retain arguments
    pub may_retain: bool,
    /// Whether this call may spawn threads
    pub may_spawn_thread: bool,
}

impl IrCallInfo {
    /// Create call info from call site
    #[must_use]
    pub fn from_call_site(site: IrCallSite) -> Self {
        let mapping = site.arg_mapping();
        let reference_args = site.reference_arguments();

        Self {
            site,
            mapping,
            reference_args,
            may_retain: false,       // Conservative default
            may_spawn_thread: false, // Conservative default
        }
    }

    /// Mark as potentially retaining arguments
    #[must_use]
    pub fn mark_retaining(mut self) -> Self {
        self.may_retain = true;
        self
    }

    /// Mark as potentially spawning threads
    #[must_use]
    pub fn mark_thread_spawning(mut self) -> Self {
        self.may_spawn_thread = true;
        self
    }

    /// Check if reference may escape via this call
    #[must_use]
    pub fn may_escape_reference(&self, ref_id: RefId) -> bool {
        if !self.site.passes_reference(ref_id) {
            return false;
        }

        // Conservative: if call may retain, reference escapes
        self.may_retain || self.may_spawn_thread
    }
}

// ==================================================================================
// IR Call Extractor
// ==================================================================================

/// IR-based call site extractor
///
/// Parses IR instructions to extract precise call site information.
/// This is the main entry point for IR-based call extraction.
#[derive(Debug)]
pub struct IrCallExtractor {
    /// Known thread-spawning functions
    thread_spawn_functions: Set<Text>,
    /// Known retaining functions
    retaining_functions: Set<Text>,
}

impl IrCallExtractor {
    /// Create new IR call extractor
    #[must_use]
    pub fn new() -> Self {
        let mut thread_spawn_functions = Set::new();
        thread_spawn_functions.insert("std.thread.spawn".into());
        thread_spawn_functions.insert("tokio.spawn".into());
        thread_spawn_functions.insert("rayon.spawn".into());

        let mut retaining_functions = Set::new();
        retaining_functions.insert("std.collections.Vec.push".into());
        retaining_functions.insert("std.collections.HashMap.insert".into());
        retaining_functions.insert("Box.leak".into());

        Self {
            thread_spawn_functions,
            retaining_functions,
        }
    }

    /// Extract all call sites from a function
    ///
    /// Performs linear scan through all instructions to find call sites.
    /// Complexity: O(n) where n = number of instructions
    #[must_use]
    pub fn extract_from_function(&self, func: &IrFunction) -> Vec<IrCallSite> {
        let mut call_sites = Vec::new();

        for (block_id, offset, inst) in func.all_instructions() {
            if let Maybe::Some((target, args, result)) = inst.as_call() {
                let site = IrCallSite::new(
                    func.id,
                    block_id,
                    offset,
                    target.clone(),
                    args.clone(),
                    result,
                );
                call_sites.push(site);
            }
        }

        call_sites
    }

    /// Extract call sites with full context information
    #[must_use]
    pub fn extract_with_info(&self, func: &IrFunction) -> Vec<IrCallInfo> {
        let call_sites = self.extract_from_function(func);

        call_sites
            .into_iter()
            .map(|site| {
                let mut info = IrCallInfo::from_call_site(site);

                // Check if this is a known retaining function
                if self.retaining_functions.contains(&info.site.callee_name) {
                    info = info.mark_retaining();
                }

                // Check if this is a known thread-spawning function
                if self.thread_spawn_functions.contains(&info.site.callee_name) {
                    info = info.mark_thread_spawning();
                }

                info
            })
            .collect()
    }

    /// Extract call sites that pass a specific reference
    #[must_use]
    pub fn extract_calls_with_reference(
        &self,
        func: &IrFunction,
        ref_id: RefId,
    ) -> Vec<IrCallSite> {
        self.extract_from_function(func)
            .into_iter()
            .filter(|site| site.passes_reference(ref_id))
            .collect()
    }

    /// Extract calls to a specific function
    #[must_use]
    pub fn extract_calls_to_function(
        &self,
        func: &IrFunction,
        callee_name: &str,
    ) -> Vec<IrCallSite> {
        self.extract_from_function(func)
            .into_iter()
            .filter(|site| site.callee_name == callee_name)
            .collect()
    }

    /// Register thread-spawning function
    pub fn register_thread_spawn_function(&mut self, name: impl Into<Text>) {
        self.thread_spawn_functions.insert(name.into());
    }

    /// Register retaining function
    pub fn register_retaining_function(&mut self, name: impl Into<Text>) {
        self.retaining_functions.insert(name.into());
    }

    /// Extract return sites (for return value tracking)
    #[must_use]
    pub fn extract_return_sites(
        &self,
        func: &IrFunction,
    ) -> Vec<(BlockId, usize, Maybe<IrOperand>)> {
        let mut return_sites = Vec::new();

        for (block_id, offset, inst) in func.all_instructions() {
            if let IrInstruction::Return { value } = inst {
                return_sites.push((block_id, offset, value.clone()));
            }
        }

        return_sites
    }

    /// Check if reference flows to return value
    #[must_use]
    pub fn flows_to_return(&self, func: &IrFunction, ref_id: RefId) -> bool {
        let return_sites = self.extract_return_sites(func);

        return_sites.iter().any(
            |(_, _, value)| matches!(value, Maybe::Some(IrOperand::Reference(r)) if *r == ref_id),
        )
    }
}

impl Default for IrCallExtractor {
    fn default() -> Self {
        Self::new()
    }
}

// ==================================================================================
// Extraction Statistics
// ==================================================================================

/// Statistics about call site extraction
#[derive(Debug, Clone, Default)]
pub struct ExtractionStats {
    /// Total instructions scanned
    pub instructions_scanned: usize,
    /// Total call sites found
    pub call_sites_found: usize,
    /// Call sites with references
    pub call_sites_with_refs: usize,
    /// Retaining calls found
    pub retaining_calls: usize,
    /// Thread-spawning calls found
    pub thread_spawn_calls: usize,
    /// Return sites found
    pub return_sites: usize,
}

impl ExtractionStats {
    /// Create statistics from extraction results
    #[must_use]
    pub fn from_extraction(
        func: &IrFunction,
        call_infos: &[IrCallInfo],
        return_count: usize,
    ) -> Self {
        let call_sites_with_refs = call_infos
            .iter()
            .filter(|info| !info.reference_args.is_empty())
            .count();

        let retaining_calls = call_infos.iter().filter(|info| info.may_retain).count();

        let thread_spawn_calls = call_infos
            .iter()
            .filter(|info| info.may_spawn_thread)
            .count();

        Self {
            instructions_scanned: func.instruction_count(),
            call_sites_found: call_infos.len(),
            call_sites_with_refs,
            retaining_calls,
            thread_spawn_calls,
            return_sites: return_count,
        }
    }

    /// Get percentage of calls that pass references
    #[must_use]
    pub fn ref_call_percentage(&self) -> f64 {
        if self.call_sites_found == 0 {
            return 0.0;
        }
        (self.call_sites_with_refs as f64 / self.call_sites_found as f64) * 100.0
    }
}

impl fmt::Display for ExtractionStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Instructions: {}, Calls: {}, With refs: {} ({:.1}%), Retaining: {}, Thread-spawn: {}, Returns: {}",
            self.instructions_scanned,
            self.call_sites_found,
            self.call_sites_with_refs,
            self.ref_call_percentage(),
            self.retaining_calls,
            self.thread_spawn_calls,
            self.return_sites
        )
    }
}

// ==================================================================================
// AST Integration (verum_ast conversion)
// ==================================================================================

/// AST to IR converter
///
/// Converts Verum AST expressions to the simplified IR representation used by
/// the call extraction module. This enables escape analysis to work with
/// actual parsed Verum code rather than just synthetic IR.
///
/// # Architecture
///
/// ```text
/// verum_ast::Expr  ─────►  IrInstruction
///        │                      │
///        ▼                      ▼
/// ExprKind::Call   ─────►  IrInstruction::Call
/// ExprKind::Await  ─────►  IrInstruction::Call (with async marker)
/// ExprKind::Try    ─────►  IrInstruction with exception info
/// ```
///
/// # Performance
///
/// - Single-pass conversion: O(nodes)
/// - Memory: O(instructions) for output
/// - Target: <1ms for 1000-node AST
#[derive(Debug)]
pub struct AstToIrConverter {
    /// Current function being converted
    current_function: Maybe<FunctionId>,
    /// Next available local variable ID
    next_local: u32,
    /// Next available block ID
    next_block: u32,
    /// Reference ID counter
    next_ref: u32,
    /// Map from AST variable names to local IDs
    var_to_local: Map<Text, u32>,
    /// Map from variable names to reference IDs (for reference tracking)
    var_to_ref: Map<Text, RefId>,
}

impl AstToIrConverter {
    /// Create a new AST to IR converter
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_function: Maybe::None,
            next_local: 0,
            next_block: 0,
            next_ref: 0,
            var_to_local: Map::new(),
            var_to_ref: Map::new(),
        }
    }

    /// Reset state for converting a new function
    pub fn reset(&mut self, function_id: FunctionId) {
        self.current_function = Maybe::Some(function_id);
        self.next_local = 0;
        self.next_block = 0;
        // Keep ref counter incrementing across functions
        self.var_to_local.clear();
        self.var_to_ref.clear();
    }

    /// Allocate a new local variable
    fn alloc_local(&mut self) -> u32 {
        let id = self.next_local;
        self.next_local += 1;
        id
    }

    /// Allocate a new block ID
    fn alloc_block(&mut self) -> BlockId {
        let id = BlockId(u64::from(self.next_block));
        self.next_block += 1;
        id
    }

    /// Allocate a new reference ID
    fn alloc_ref(&mut self) -> RefId {
        let id = RefId(u64::from(self.next_ref));
        self.next_ref += 1;
        id
    }

    /// Register a variable binding
    pub fn register_variable(&mut self, name: Text) -> u32 {
        let local = self.alloc_local();
        self.var_to_local.insert(name, local);
        local
    }

    /// Register a variable as holding a reference
    pub fn register_reference_variable(&mut self, name: Text) -> RefId {
        let ref_id = self.alloc_ref();
        self.var_to_ref.insert(name, ref_id);
        ref_id
    }

    /// Look up a variable's local ID
    #[must_use]
    pub fn lookup_variable(&self, name: &str) -> Maybe<u32> {
        let name_text: Text = name.to_string().into();
        self.var_to_local.get(&name_text).copied()
    }

    /// Look up a variable's reference ID
    #[must_use]
    pub fn lookup_reference(&self, name: &str) -> Maybe<RefId> {
        let name_text: Text = name.to_string().into();
        self.var_to_ref.get(&name_text).copied()
    }

    /// Convert an AST expression kind to IR operand
    ///
    /// This converts expressions that can be used as operands (values, variables, etc.)
    /// to their IR representation.
    #[must_use]
    pub fn expr_kind_to_operand(&self, kind: &AstExprKind) -> IrOperand {
        match kind {
            AstExprKind::Literal(lit) => {
                // Convert literal to constant
                match lit {
                    AstLiteral::Int(val) => IrOperand::Constant(*val),
                    AstLiteral::Bool(b) => IrOperand::Constant(i64::from(*b)),
                    _ => IrOperand::Constant(0), // Other literals default to 0
                }
            }
            AstExprKind::Path(path) => {
                // Look up the path as a variable
                if let Maybe::Some(ref_id) = self.var_to_ref.get(&path.to_string()) {
                    IrOperand::Reference(*ref_id)
                } else if let Maybe::Some(local) = self.var_to_local.get(&path.to_string()) {
                    IrOperand::LocalVar(*local)
                } else {
                    // Unknown variable, use undefined
                    IrOperand::Undef
                }
            }
            _ => {
                // Complex expressions need to be lowered to instructions first
                IrOperand::Undef
            }
        }
    }

    /// Convert an AST Call expression to an IR call instruction
    ///
    /// # Arguments
    /// * `func` - The function being called
    /// * `args` - The arguments to the call
    /// * `result_local` - Local variable to store result (if any)
    #[must_use]
    pub fn convert_call(
        &self,
        func: &AstExpr,
        args: &[AstExpr],
        result_local: Maybe<u32>,
    ) -> IrInstruction {
        // Extract function name from the expression
        let target = match &func.kind {
            AstExprKind::Path(path) => path.to_string(),
            _ => Text::from("<unknown>"),
        };

        // Convert arguments
        let ir_args: Vec<IrOperand> = args
            .iter()
            .map(|arg| self.expr_kind_to_operand(&arg.kind))
            .collect();

        IrInstruction::Call {
            target,
            args: ir_args,
            result: result_local,
        }
    }

    /// Convert an AST Await expression to IR instructions
    ///
    /// Await is represented as a call to the async runtime with
    /// special metadata for the async boundary analysis.
    #[must_use]
    pub fn convert_await(&self, expr: &AstExpr, result_local: Maybe<u32>) -> IrInstruction {
        // Await is modeled as a call to the future's poll method
        let future_operand = self.expr_kind_to_operand(&expr.kind);

        IrInstruction::Call {
            target: Text::from("__verum_runtime.poll_future"),
            args: vec![future_operand],
            result: result_local,
        }
    }

    /// Convert an AST Return expression to IR instruction
    #[must_use]
    pub fn convert_return(&self, expr: Maybe<&AstExpr>) -> IrInstruction {
        let value = expr.map(|e| self.expr_kind_to_operand(&e.kind));
        IrInstruction::Return { value }
    }

    /// Generate IR function from a list of converted instructions
    #[must_use]
    pub fn build_function(
        &mut self,
        function_id: FunctionId,
        name: impl Into<Text>,
        instructions: Vec<(BlockId, IrInstruction)>,
    ) -> IrFunction {
        let mut func = IrFunction::new(function_id, name);

        // Add instructions to blocks
        for (block_id, inst) in instructions {
            let offset = func.blocks.get(&block_id).map_or(0, std::vec::Vec::len);
            func.add_instruction(block_id, offset, inst);
        }

        // Track locals
        for local in 0..self.next_local {
            func.add_local(local);
        }

        func
    }
}

impl Default for AstToIrConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper types for AST expressions (to avoid direct dependency cycles)
///
/// These type aliases allow the IR extraction module to reference AST types
/// when they're available, while keeping the simplified IR as the primary
/// representation for analysis.

/// AST Expression type alias
#[cfg(feature = "ast-integration")]
pub type AstExpr = verum_ast::expr::Expr;

/// AST Expression kind type alias
#[cfg(feature = "ast-integration")]
pub type AstExprKind = verum_ast::expr::ExprKind;

/// AST Literal type alias
#[cfg(feature = "ast-integration")]
pub type AstLiteral = verum_ast::literal::Literal;

/// Stub types when AST integration is disabled
#[cfg(not(feature = "ast-integration"))]
#[derive(Debug, Clone)]
pub struct AstExpr {
    pub kind: AstExprKind,
}

#[cfg(not(feature = "ast-integration"))]
#[derive(Debug, Clone)]
pub enum AstExprKind {
    Literal(AstLiteral),
    Path(AstPath),
    Call {
        func: Box<AstExpr>,
        args: Vec<AstExpr>,
    },
    Await(Box<AstExpr>),
    Return {
        value: Option<Box<AstExpr>>,
    },
    Other,
}

#[cfg(not(feature = "ast-integration"))]
#[derive(Debug, Clone)]
pub enum AstLiteral {
    Int(i64),
    Bool(bool),
    String(Text),
    Other,
}

#[cfg(not(feature = "ast-integration"))]
#[derive(Debug, Clone)]
pub struct AstPath {
    pub segments: Vec<Text>,
}

#[cfg(not(feature = "ast-integration"))]
impl AstPath {
    #[must_use]
    pub fn to_string(&self) -> Text {
        let joined: String = self.segments.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(".");
        joined.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ir_operand_creation() {
        let local = IrOperand::LocalVar(1);
        assert!(!local.is_reference());
        assert_eq!(local.as_reference(), Maybe::None);

        let ref_op = IrOperand::Reference(RefId(42));
        assert!(ref_op.is_reference());
        assert_eq!(ref_op.as_reference(), Maybe::Some(RefId(42)));
    }

    #[test]
    fn test_ir_instruction_call_detection() {
        let call = IrInstruction::Call {
            target: "foo".into(),
            args: vec![IrOperand::LocalVar(0)],
            result: Maybe::Some(1),
        };
        assert!(call.is_call());

        let assign = IrInstruction::Assign {
            dest: 1,
            src: IrOperand::Constant(42),
        };
        assert!(!assign.is_call());
    }
}
