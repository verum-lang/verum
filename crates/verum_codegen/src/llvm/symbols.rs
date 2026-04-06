//! Symbol attribute handling for LLVM codegen.
//!
//! This module provides support for linker-level symbol attributes:
//! - `@alias(target)` - Symbol aliasing
//! - `@weak` - Weak symbol linkage
//! - `@linkage(kind)` - Fine-grained linkage control
//! - `@init_priority(N)` - Initialization order
//! - `@section(name)` - Section placement
//! - `@export(abi)` - Export with ABI
//!
//! # LLVM Mapping
//!
//! | Verum Attribute | LLVM Feature |
//! |-----------------|--------------|
//! | `@alias(target)` | `llvm.global.alias` |
//! | `@weak` | `WeakAny` linkage |
//! | `@linkage(external)` | `External` linkage |
//! | `@linkage(internal)` | `Internal` linkage |
//! | `@linkage(private)` | `Private` linkage |
//! | `@linkage(weak)` | `WeakAny` linkage |
//! | `@linkage(linkonce)` | `LinkOnceAny` linkage |
//! | `@linkage(linkonce_odr)` | `LinkOnceODR` linkage |
//! | `@linkage(common)` | `Common` linkage |
//! | `@init_priority(N)` | `llvm.global_ctors` with priority |
//! | `@section(name)` | Function/GlobalVar section attribute |
//! | `@export("C")` | External linkage + C calling convention |
//!
//! Verum provides unified linker control through attributes:
//! - `@link_section(".text.hot")` places functions/data in specific ELF/Mach-O/PE sections
//! - `@no_mangle` / `@export` control symbol visibility and naming
//! - `@weak` creates weak symbols that can be overridden by strong definitions
//! - `@alias("target")` creates symbol aliases via LLVM GlobalAlias
//! - `@link_name = "name"` overrides the linked symbol name (useful for FFI)
//! - External linker symbols can be declared and accessed for linker script integration

use verum_ast::attr::{
    AliasAttr, ExportAttr, InitPriorityAttr, LinkageAttr, LinkageKind, SectionAttr,
    SymbolVisibility, VisibilityAttr, WeakAttr,
};
use verum_common::Text;
use verum_llvm::module::Linkage;
use verum_llvm::values::{BasicValueEnum, FunctionValue, GlobalValue};
use verum_llvm::AddressSpace;
use verum_llvm::GlobalVisibility;

use super::error::Result;

/// Symbol attributes collected from AST for a function or global.
#[derive(Debug, Clone, Default)]
pub struct SymbolAttributes {
    /// Symbol alias target (if @alias present).
    pub alias_target: Option<Text>,

    /// Is this a weak symbol (@weak or @linkage(weak)).
    pub is_weak: bool,

    /// Explicit linkage kind (from @linkage).
    pub linkage: Option<LinkageKind>,

    /// Symbol visibility (from @visibility).
    pub visibility: Option<SymbolVisibility>,

    /// Section name (from @section).
    pub section: Option<Text>,

    /// Init priority (from @init_priority).
    pub init_priority: Option<u32>,

    /// Export ABI (from @export).
    pub export_abi: Option<Text>,

    /// Custom export name (from @export with name parameter).
    pub export_name: Option<Text>,
}

impl SymbolAttributes {
    /// Create new empty symbol attributes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add alias attribute.
    pub fn with_alias(mut self, attr: &AliasAttr) -> Self {
        self.alias_target = Some(attr.target.clone());
        self
    }

    /// Add weak attribute.
    pub fn with_weak(mut self, _attr: &WeakAttr) -> Self {
        self.is_weak = true;
        self
    }

    /// Add linkage attribute.
    pub fn with_linkage(mut self, attr: &LinkageAttr) -> Self {
        self.linkage = Some(attr.kind);
        self
    }

    /// Add visibility attribute.
    pub fn with_visibility(mut self, attr: &VisibilityAttr) -> Self {
        self.visibility = Some(attr.visibility);
        self
    }

    /// Add section attribute.
    pub fn with_section(mut self, attr: &SectionAttr) -> Self {
        self.section = Some(attr.name.clone());
        self
    }

    /// Add init priority attribute.
    pub fn with_init_priority(mut self, attr: &InitPriorityAttr) -> Self {
        self.init_priority = Some(attr.priority);
        self
    }

    /// Add export attribute.
    pub fn with_export(mut self, attr: &ExportAttr) -> Self {
        self.export_abi = Some(attr.abi.clone());
        if let verum_common::Maybe::Some(name) = &attr.export_name {
            self.export_name = Some(name.clone());
        }
        self
    }

    /// Get the effective linkage kind.
    pub fn effective_linkage(&self) -> LinkageKind {
        if self.is_weak {
            LinkageKind::Weak
        } else {
            self.linkage.unwrap_or(LinkageKind::External)
        }
    }

    /// Check if symbol should be externally visible.
    pub fn is_externally_visible(&self) -> bool {
        match self.visibility {
            Some(SymbolVisibility::Hidden) => false,
            Some(SymbolVisibility::Default) | Some(SymbolVisibility::Protected) => true,
            None => {
                // Default based on linkage
                match self.effective_linkage() {
                    LinkageKind::External | LinkageKind::Weak | LinkageKind::Common => true,
                    LinkageKind::Internal
                    | LinkageKind::Private
                    | LinkageKind::AvailableExternally => false,
                    LinkageKind::Linkonce | LinkageKind::LinkonceOdr => true,
                }
            }
        }
    }
}

/// Apply symbol attributes to a function.
pub fn apply_to_function(func: FunctionValue<'_>, attrs: &SymbolAttributes) -> Result<()> {
    // Apply linkage
    let llvm_linkage = linkage_to_llvm(attrs.effective_linkage());
    func.set_linkage(llvm_linkage);

    // Apply visibility via GlobalValue
    if let Some(vis) = attrs.visibility {
        let llvm_vis = visibility_to_llvm(vis);
        func.as_global_value().set_visibility(llvm_vis);
    }

    // Apply section
    if let Some(ref section) = attrs.section {
        func.set_section(Some(section.as_str()));
    }

    Ok(())
}

/// Apply symbol attributes to a global variable.
pub fn apply_to_global(global: GlobalValue<'_>, attrs: &SymbolAttributes) -> Result<()> {
    // Apply linkage
    let llvm_linkage = linkage_to_llvm(attrs.effective_linkage());
    global.set_linkage(llvm_linkage);

    // Apply visibility
    if let Some(vis) = attrs.visibility {
        let llvm_vis = visibility_to_llvm(vis);
        global.set_visibility(llvm_vis);
    }

    // Apply section
    if let Some(ref section) = attrs.section {
        global.set_section(Some(section.as_str()));
    }

    Ok(())
}

/// Convert Verum LinkageKind to LLVM Linkage.
pub fn linkage_to_llvm(kind: LinkageKind) -> Linkage {
    match kind {
        LinkageKind::External => Linkage::External,
        LinkageKind::Internal => Linkage::Internal,
        LinkageKind::Private => Linkage::Private,
        LinkageKind::Weak => Linkage::WeakAny,
        LinkageKind::Linkonce => Linkage::LinkOnceAny,
        LinkageKind::LinkonceOdr => Linkage::LinkOnceODR,
        LinkageKind::Common => Linkage::Common,
        LinkageKind::AvailableExternally => Linkage::AvailableExternally,
    }
}

/// Convert Verum SymbolVisibility to LLVM GlobalVisibility.
pub fn visibility_to_llvm(vis: SymbolVisibility) -> GlobalVisibility {
    match vis {
        SymbolVisibility::Default => GlobalVisibility::Default,
        SymbolVisibility::Hidden => GlobalVisibility::Hidden,
        SymbolVisibility::Protected => GlobalVisibility::Protected,
    }
}

/// Create a symbol alias via `LLVMAddAlias2`.
///
/// Looks up the target function or global in the module and creates
/// an LLVM GlobalAlias pointing to it. Falls back to a debug log
/// if the target cannot be found (e.g., not yet defined).
pub fn create_alias(
    module: &verum_llvm::module::Module<'_>,
    alias_name: &str,
    target_name: &str,
    _attrs: &SymbolAttributes,
) -> Result<()> {
    use verum_llvm::values::AsValueRef;
    use verum_llvm::types::AsTypeRef;

    // Try to find the target as a function first, then as a global
    if let Some(target_fn) = module.get_function(target_name) {
        let aliasee = target_fn.as_value_ref();
        let value_ty = target_fn.get_type().as_type_ref();
        let c_name = std::ffi::CString::new(alias_name).map_err(|_| {
            super::error::LlvmLoweringError::Internal(
                format!("Invalid alias name: {}", alias_name).into()
            )
        })?;
        unsafe {
            verum_llvm_sys::core::LLVMAddAlias2(
                module.as_mut_ptr(),
                value_ty,
                0, // default address space
                aliasee,
                c_name.as_ptr(),
            );
        }
        tracing::debug!("Created LLVM alias @{} -> @{}", alias_name, target_name);
    } else {
        tracing::debug!(
            "Symbol alias @{} -> @{} deferred: target not found in module",
            alias_name,
            target_name,
        );
    }
    Ok(())
}

/// Default priority for global constructors/destructors (lowest priority = runs last among
/// constructors, first among destructors). This is the standard default used by C++ static
/// initializers and `__attribute__((constructor))`.
pub const DEFAULT_CTOR_DTOR_PRIORITY: u32 = 65535;

/// Add a single function to the global constructors list with the given priority.
///
/// This creates an `llvm.global_ctors` entry with `appending` linkage.
/// On ELF platforms, entries are placed in the `.init_array` section.
/// On Mach-O, they go into `__mod_init_func`. On PE/COFF, `.CRT$XCU`.
///
/// Lower priority values run first. Priorities 0-100 are reserved for system use.
/// The standard default is 65535.
///
/// Each call appends a separate `llvm.global_ctors` global; LLVM's linker will
/// merge all such globals with `appending` linkage into a single array.
pub fn add_global_ctor<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    func: FunctionValue<'ctx>,
    priority: u32,
) -> Result<()> {
    emit_global_ctor_dtor_array(module, "llvm.global_ctors", &[(func, priority)])?;
    tracing::debug!(
        "Registered global constructor '{}' with priority {}",
        func.get_name().to_string_lossy(),
        priority
    );
    Ok(())
}

/// Add a single function to the global destructors list with the given priority.
///
/// This creates an `llvm.global_dtors` entry with `appending` linkage.
/// On ELF platforms, entries are placed in the `.fini_array` section.
///
/// Lower priority values run first (so for destructors, lower priority = runs first
/// during teardown).
pub fn add_global_dtor<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    func: FunctionValue<'ctx>,
    priority: u32,
) -> Result<()> {
    emit_global_ctor_dtor_array(module, "llvm.global_dtors", &[(func, priority)])?;
    tracing::debug!(
        "Registered global destructor '{}' with priority {}",
        func.get_name().to_string_lossy(),
        priority
    );
    Ok(())
}

/// Emit multiple global constructors at once.
///
/// More efficient than calling `add_global_ctor` in a loop because it creates a single
/// `llvm.global_ctors` array global with all entries. Each entry is a
/// `(FunctionValue, priority)` pair.
///
/// If `entries` is empty, this is a no-op.
pub fn emit_global_ctors<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    entries: &[(FunctionValue<'ctx>, u32)],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    emit_global_ctor_dtor_array(module, "llvm.global_ctors", entries)?;
    tracing::debug!("Registered {} global constructor(s)", entries.len());
    Ok(())
}

/// Emit multiple global destructors at once.
///
/// More efficient than calling `add_global_dtor` in a loop because it creates a single
/// `llvm.global_dtors` array global with all entries. Each entry is a
/// `(FunctionValue, priority)` pair.
///
/// If `entries` is empty, this is a no-op.
pub fn emit_global_dtors<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    entries: &[(FunctionValue<'ctx>, u32)],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    emit_global_ctor_dtor_array(module, "llvm.global_dtors", entries)?;
    tracing::debug!("Registered {} global destructor(s)", entries.len());
    Ok(())
}

/// Create an `llvm.global_ctors` or `llvm.global_dtors` array global.
///
/// The LLVM specification requires these globals to have:
/// - Type: `[N x { i32, ptr, ptr }]`
/// - Linkage: `appending`
///
/// Each element is a struct `{ i32 priority, ptr function, ptr data }` where:
/// - `priority`: Lower values run first (0-100 reserved for system)
/// - `function`: Pointer to a `void()` function
/// - `data`: Associated data pointer (always null for Verum constructors/destructors)
///
/// When multiple modules define the same named global with `appending` linkage,
/// the LLVM linker concatenates all arrays into one.
fn emit_global_ctor_dtor_array<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    global_name: &str,
    entries: &[(FunctionValue<'ctx>, u32)],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let context = module.get_context();

    // Build the entry struct type: { i32, ptr, ptr }
    let i32_type = context.i32_type();
    let ptr_type = context.ptr_type(AddressSpace::default());
    let entry_struct_type = context.struct_type(
        &[i32_type.into(), ptr_type.into(), ptr_type.into()],
        false,
    );

    // Build the array of constant struct entries.
    let null_ptr = ptr_type.const_null();
    let struct_entries: Vec<_> = entries
        .iter()
        .map(|(func, priority)| {
            let priority_val: BasicValueEnum = i32_type.const_int(*priority as u64, false).into();
            // In opaque-pointer LLVM, a function IS a pointer value already.
            let func_ptr: BasicValueEnum = func.as_global_value().as_pointer_value().into();
            let data_ptr: BasicValueEnum = null_ptr.into();
            context.const_struct(&[priority_val, func_ptr, data_ptr], false)
        })
        .collect();

    // Create the array type [N x { i32, ptr, ptr }] and the constant array value.
    let array_val = entry_struct_type.const_array(&struct_entries);

    // Create the global with appending linkage.
    let array_type = entry_struct_type.array_type(entries.len() as u32);
    let global = module.add_global(array_type, None, global_name);
    global.set_linkage(Linkage::Appending);
    global.set_initializer(&array_val);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_llvm::context::Context;

    #[test]
    fn test_symbol_attributes_default() {
        let attrs = SymbolAttributes::new();
        assert!(!attrs.is_weak);
        assert!(attrs.alias_target.is_none());
        assert!(attrs.section.is_none());
        assert_eq!(attrs.effective_linkage(), LinkageKind::External);
    }

    #[test]
    fn test_symbol_attributes_weak() {
        let mut attrs = SymbolAttributes::new();
        attrs.is_weak = true;
        assert_eq!(attrs.effective_linkage(), LinkageKind::Weak);
    }

    #[test]
    fn test_symbol_visibility_externally_visible() {
        let mut attrs = SymbolAttributes::new();
        assert!(attrs.is_externally_visible());

        attrs.visibility = Some(SymbolVisibility::Hidden);
        assert!(!attrs.is_externally_visible());

        attrs.visibility = Some(SymbolVisibility::Default);
        assert!(attrs.is_externally_visible());
    }

    #[test]
    fn test_linkage_to_llvm() {
        assert!(matches!(
            linkage_to_llvm(LinkageKind::External),
            Linkage::External
        ));
        assert!(matches!(
            linkage_to_llvm(LinkageKind::Internal),
            Linkage::Internal
        ));
        assert!(matches!(
            linkage_to_llvm(LinkageKind::Weak),
            Linkage::WeakAny
        ));
        assert!(matches!(
            linkage_to_llvm(LinkageKind::LinkonceOdr),
            Linkage::LinkOnceODR
        ));
    }

    #[test]
    fn test_visibility_to_llvm() {
        assert!(matches!(
            visibility_to_llvm(SymbolVisibility::Default),
            GlobalVisibility::Default
        ));
        assert!(matches!(
            visibility_to_llvm(SymbolVisibility::Hidden),
            GlobalVisibility::Hidden
        ));
        assert!(matches!(
            visibility_to_llvm(SymbolVisibility::Protected),
            GlobalVisibility::Protected
        ));
    }

    #[test]
    fn test_default_ctor_dtor_priority() {
        assert_eq!(DEFAULT_CTOR_DTOR_PRIORITY, 65535);
    }

    #[test]
    fn test_add_global_ctor_single() {
        let context = Context::create();
        let module = context.create_module("test_ctors");
        let void_type = context.void_type();
        let fn_type = void_type.fn_type(&[], false);
        let ctor_fn = module.add_function("my_ctor", fn_type, None);

        // Should succeed without error.
        add_global_ctor(&module, ctor_fn, DEFAULT_CTOR_DTOR_PRIORITY).unwrap();

        // Verify the llvm.global_ctors global was created.
        let global = module.get_global("llvm.global_ctors");
        assert!(global.is_some(), "llvm.global_ctors should exist");

        // Verify appending linkage.
        let global = global.unwrap();
        assert!(matches!(global.get_linkage(), Linkage::Appending));

        // Verify the module is valid LLVM IR.
        assert!(module.verify().is_ok(), "Module should verify after adding ctor");
    }

    #[test]
    fn test_add_global_dtor_single() {
        let context = Context::create();
        let module = context.create_module("test_dtors");
        let void_type = context.void_type();
        let fn_type = void_type.fn_type(&[], false);
        let dtor_fn = module.add_function("my_dtor", fn_type, None);

        add_global_dtor(&module, dtor_fn, DEFAULT_CTOR_DTOR_PRIORITY).unwrap();

        let global = module.get_global("llvm.global_dtors");
        assert!(global.is_some(), "llvm.global_dtors should exist");
        assert!(matches!(global.unwrap().get_linkage(), Linkage::Appending));
        assert!(module.verify().is_ok(), "Module should verify after adding dtor");
    }

    #[test]
    fn test_emit_global_ctors_batch() {
        let context = Context::create();
        let module = context.create_module("test_batch_ctors");
        let void_type = context.void_type();
        let fn_type = void_type.fn_type(&[], false);

        let ctor1 = module.add_function("ctor_early", fn_type, None);
        let ctor2 = module.add_function("ctor_late", fn_type, None);
        let ctor3 = module.add_function("ctor_default", fn_type, None);

        // Emit all at once with different priorities.
        emit_global_ctors(
            &module,
            &[(ctor1, 101), (ctor2, 65535), (ctor3, 500)],
        )
        .unwrap();

        let global = module.get_global("llvm.global_ctors");
        assert!(global.is_some(), "llvm.global_ctors should exist");
        assert!(module.verify().is_ok(), "Module should verify with 3 ctors");
    }

    #[test]
    fn test_emit_global_ctors_empty_is_noop() {
        let context = Context::create();
        let module = context.create_module("test_empty_ctors");

        // Empty entries should be a no-op.
        emit_global_ctors(&module, &[]).unwrap();

        // No global should be created.
        let global = module.get_global("llvm.global_ctors");
        assert!(global.is_none(), "Empty emit should not create global");
    }

    #[test]
    fn test_emit_global_dtors_empty_is_noop() {
        let context = Context::create();
        let module = context.create_module("test_empty_dtors");

        emit_global_dtors(&module, &[]).unwrap();

        let global = module.get_global("llvm.global_dtors");
        assert!(global.is_none(), "Empty emit should not create global");
    }

    #[test]
    fn test_both_ctors_and_dtors() {
        let context = Context::create();
        let module = context.create_module("test_both");
        let void_type = context.void_type();
        let fn_type = void_type.fn_type(&[], false);

        let ctor = module.add_function("my_init", fn_type, None);
        let dtor = module.add_function("my_fini", fn_type, None);

        add_global_ctor(&module, ctor, 200).unwrap();
        add_global_dtor(&module, dtor, 200).unwrap();

        assert!(module.get_global("llvm.global_ctors").is_some());
        assert!(module.get_global("llvm.global_dtors").is_some());
        assert!(module.verify().is_ok(), "Module should verify with both ctors and dtors");
    }

    #[test]
    fn test_ctor_with_custom_priority() {
        let context = Context::create();
        let module = context.create_module("test_priority");
        let void_type = context.void_type();
        let fn_type = void_type.fn_type(&[], false);
        let ctor = module.add_function("early_init", fn_type, None);

        // Priority 101 is the lowest user-level priority (0-100 reserved for system).
        add_global_ctor(&module, ctor, 101).unwrap();

        assert!(module.verify().is_ok(), "Module should verify with priority 101");
    }

    #[test]
    fn test_global_ctor_ir_contains_expected_structure() {
        let context = Context::create();
        let module = context.create_module("test_ir");
        let void_type = context.void_type();
        let fn_type = void_type.fn_type(&[], false);
        let ctor = module.add_function("test_ctor", fn_type, None);

        add_global_ctor(&module, ctor, 65535).unwrap();

        let ir = module.to_string();
        // The IR should contain the global_ctors declaration with appending linkage.
        assert!(
            ir.contains("@llvm.global_ctors"),
            "IR should contain @llvm.global_ctors"
        );
        assert!(
            ir.contains("appending"),
            "IR should contain appending linkage"
        );
        // Should reference the constructor function.
        assert!(
            ir.contains("@test_ctor"),
            "IR should reference the constructor function"
        );
    }
}
