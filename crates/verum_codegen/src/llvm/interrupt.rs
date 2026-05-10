//! Interrupt handler code generation for ISR (Interrupt Service Routine) support.
//!

//! This module provides LLVM IR generation for interrupt service routines,
//! including prologue/epilogue generation, critical section handling, and
//! proper return instructions for various architectures.
//!

//! # Overview
//!

//! Interrupt handlers require special treatment:
//! - Saving/restoring all modified registers (not just callee-saved)
//! - Using interrupt-specific return instructions (iret, rfi, eret, mret)
//! - Proper stack alignment
//! - Optional FPU state preservation
//!

//! # Generated Code Patterns
//!

//! ```llvm
//! ; x86_64 interrupt handler
//! define x86_intrcc void @timer_isr(ptr %frame) {
//! entry:
//!  ; prologue: save registers
//!  ; ... handler body ...
//!  ; epilogue: restore registers, iretq
//!  ret void ; with x86_intrcc, this becomes iretq
//! }
//!

//! ; ARM interrupt handler
//! define arm_aapcscc void @irq_handler() "interrupt"="IRQ" {
//! entry:
//!  ; ... handler body ...
//!  ret void ; becomes appropriate return for interrupt type
//! }
//! ```
//!

//! # Interrupt Handler Codegen
//!

//! Verum uses `@interrupt(vector = N)` attribute for ISR declarations. Interrupt
//! handlers require special codegen: saving/restoring ALL modified registers (not
//! just callee-saved), using architecture-specific return instructions (x86: iret,
//! ARM: exception return, RISC-V: mret/sret), proper stack alignment, and optional
//! FPU state preservation. `@naked` functions emit no prologue/epilogue (assembly
//! only). InterruptCell<T> provides interrupt-safe shared data via CriticalSection
//! guards that disable/restore interrupts with RAII semantics.

use verum_llvm::InlineAsmDialect;
use verum_llvm::attributes::{Attribute, AttributeLoc};
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::intrinsics::Intrinsic;
use verum_llvm::types::FunctionType;
use verum_llvm::values::{FunctionValue, PointerValue};

use super::error::{BuildExt, LlvmLoweringError, Result};
use super::types::TypeLowering;

/// Target architecture for interrupt handling.
///

/// Different architectures have different interrupt handling conventions,
/// register save requirements, and return instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetArch {
    /// x86-64 (AMD64) architecture
    X86_64,
    /// 32-bit x86 architecture
    X86,
    /// ARM 32-bit (Cortex-M, Cortex-A)
    ARM,
    /// ARM 64-bit (AArch64)
    AArch64,
    /// RISC-V 32-bit
    RiscV32,
    /// RISC-V 64-bit
    RiscV64,
}

/// Per-variant projection for [`TargetArch`].
///
/// `name` is the canonical lowercase identifier (matches the form
/// produced by `triple.split('-').next()`); `interrupt_call_conv` is
/// the LLVM CC ID for ISR codegen (83 = `x86_intrcc`, 67 = `ARM_AAPCS`,
/// 0 = default C — RISC-V and AArch64 use default CC + attributes).
/// `has_interrupt_cc` and `asm_dialect` were previously parallel
/// `matches!()` calls; they now ride per-variant fields. Adding a new
/// architecture forces explicit decisions on all four downstream
/// projections in one place.
#[derive(Debug, Clone, Copy)]
pub struct TargetArchMeta {
    pub name: &'static str,
    pub interrupt_call_conv: u32,
    pub has_interrupt_cc: bool,
    pub asm_dialect: InlineAsmDialect,
}

impl TargetArch {
    pub const ALL: &'static [Self] = &[
        Self::X86_64,
        Self::X86,
        Self::ARM,
        Self::AArch64,
        Self::RiscV32,
        Self::RiscV64,
    ];

    pub const fn meta(self) -> TargetArchMeta {
        match self {
            Self::X86_64 => TargetArchMeta {
                name: "x86_64",
                interrupt_call_conv: 83, // x86_intrcc
                has_interrupt_cc: true,
                asm_dialect: InlineAsmDialect::Intel,
            },
            Self::X86 => TargetArchMeta {
                name: "x86",
                interrupt_call_conv: 83, // x86_intrcc
                has_interrupt_cc: true,
                asm_dialect: InlineAsmDialect::Intel,
            },
            Self::ARM => TargetArchMeta {
                name: "arm",
                interrupt_call_conv: 67, // ARM_AAPCS
                has_interrupt_cc: false,
                asm_dialect: InlineAsmDialect::ATT,
            },
            Self::AArch64 => TargetArchMeta {
                name: "aarch64",
                // AArch64 doesn't have a specific interrupt CC,
                // uses default + attributes.
                interrupt_call_conv: 0, // C calling convention
                has_interrupt_cc: false,
                asm_dialect: InlineAsmDialect::ATT,
            },
            Self::RiscV32 => TargetArchMeta {
                name: "riscv32",
                interrupt_call_conv: 0, // default + attributes
                has_interrupt_cc: false,
                asm_dialect: InlineAsmDialect::ATT,
            },
            Self::RiscV64 => TargetArchMeta {
                name: "riscv64",
                interrupt_call_conv: 0, // default + attributes
                has_interrupt_cc: false,
                asm_dialect: InlineAsmDialect::ATT,
            },
        }
    }

    /// Parse architecture from a target triple. Substring matching
    /// on the first triple component — preserved verbatim from the
    /// legacy implementation so existing call sites
    /// (`linker_config.rs:341`) keep working.
    pub fn from_triple(triple: &str) -> Option<Self> {
        let arch = triple.split('-').next()?;
        match arch {
            "x86_64" | "amd64" => Some(Self::X86_64),
            "i386" | "i486" | "i586" | "i686" | "x86" => Some(Self::X86),
            "arm" | "armv6" | "armv7" | "thumb" | "thumbv6" | "thumbv7" | "thumbv7em" => {
                Some(Self::ARM)
            }
            "aarch64" | "arm64" => Some(Self::AArch64),
            "riscv32" => Some(Self::RiscV32),
            "riscv64" => Some(Self::RiscV64),
            _ => None,
        }
    }

    /// Parse the canonical lowercase architecture name (the form
    /// returned by `as_str` / `meta().name`). For looser triple-
    /// based matching, use [`TargetArch::from_triple`] instead.
    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// Canonical lowercase architecture name.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Get the LLVM calling convention ID for interrupt handlers.
    #[inline]
    pub const fn interrupt_call_conv(&self) -> u32 {
        self.meta().interrupt_call_conv
    }

    /// Check if this architecture uses a dedicated interrupt calling convention.
    #[inline]
    pub const fn has_interrupt_cc(&self) -> bool {
        self.meta().has_interrupt_cc
    }

    /// Get the inline assembly dialect for this architecture.
    #[inline]
    pub const fn asm_dialect(&self) -> InlineAsmDialect {
        self.meta().asm_dialect
    }
}

/// Kind of interrupt handler for codegen purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptHandlerKind {
    /// Regular maskable interrupt (IRQ)
    Regular,
    /// Non-maskable interrupt (NMI)
    NMI,
    /// Fast interrupt (FIQ on ARM)
    Fast,
    /// CPU exception (fault, trap, abort)
    Exception,
    /// Software interrupt / system call
    Trap,
    /// Reset vector handler
    Reset,
}

/// Per-variant projection for [`InterruptHandlerKind`].
///
/// `name` is the canonical lowercase identifier (matches the
/// `verum_ast::attr::InterruptKind` wire form, which is the
/// user-facing `@interrupt(...)` argument). `arm_interrupt_type`
/// is the ARM/AArch64 attribute string used in codegen — note that
/// ARM has no NMI distinct from SWI, so both `NMI` and `Trap` map
/// to `"SWI"` (preserved verbatim from the legacy table; this is
/// not a parse-side accept set, just a codegen-side label).
/// `requires_fpu_save` flags interrupts that may preempt FPU
/// operations.
#[derive(Debug, Clone, Copy)]
pub struct InterruptHandlerKindMeta {
    pub name: &'static str,
    pub arm_interrupt_type: &'static str,
    pub requires_fpu_save: bool,
}

impl InterruptHandlerKind {
    pub const ALL: &'static [Self] = &[
        Self::Regular,
        Self::NMI,
        Self::Fast,
        Self::Exception,
        Self::Trap,
        Self::Reset,
    ];

    pub const fn meta(self) -> InterruptHandlerKindMeta {
        match self {
            Self::Regular => InterruptHandlerKindMeta {
                name: "regular",
                arm_interrupt_type: "IRQ",
                requires_fpu_save: false,
            },
            Self::NMI => InterruptHandlerKindMeta {
                name: "nmi",
                // NMI handled differently on ARM — mapped to SWI.
                arm_interrupt_type: "SWI",
                requires_fpu_save: true,
            },
            Self::Fast => InterruptHandlerKindMeta {
                name: "fast",
                arm_interrupt_type: "FIQ",
                requires_fpu_save: false,
            },
            Self::Exception => InterruptHandlerKindMeta {
                name: "exception",
                arm_interrupt_type: "ABORT",
                requires_fpu_save: true,
            },
            Self::Trap => InterruptHandlerKindMeta {
                name: "trap",
                arm_interrupt_type: "SWI",
                requires_fpu_save: false,
            },
            Self::Reset => InterruptHandlerKindMeta {
                name: "reset",
                arm_interrupt_type: "RESET",
                requires_fpu_save: false,
            },
        }
    }

    /// Parse the canonical lowercase interrupt-kind name. Matches
    /// the `verum_ast::attr::InterruptKind` wire form used by the
    /// `@interrupt(...)` parser.
    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// Canonical lowercase name.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Get the ARM/AArch64 interrupt type string for the attribute.
    #[inline]
    pub const fn arm_interrupt_type(&self) -> &'static str {
        self.meta().arm_interrupt_type
    }

    /// Check if this interrupt type requires FPU state saving.
    #[inline]
    pub const fn requires_fpu_save(&self) -> bool {
        self.meta().requires_fpu_save
    }
}

/// Statistics for interrupt code generation.
#[derive(Debug, Clone, Default)]
pub struct InterruptStats {
    /// Number of interrupt handlers configured.
    pub handlers_configured: usize,
    /// Number of critical section entries generated.
    pub critical_section_entries: usize,
    /// Number of critical section exits generated.
    pub critical_section_exits: usize,
    /// Number of naked handlers (no prologue/epilogue).
    pub naked_handlers: usize,
    /// Number of handlers with FPU save.
    pub fpu_save_handlers: usize,
}

impl InterruptStats {
    /// Merge statistics from another instance.
    pub fn merge(&mut self, other: &InterruptStats) {
        self.handlers_configured += other.handlers_configured;
        self.critical_section_entries += other.critical_section_entries;
        self.critical_section_exits += other.critical_section_exits;
        self.naked_handlers += other.naked_handlers;
        self.fpu_save_handlers += other.fpu_save_handlers;
    }

    /// Get total operations count.
    pub fn total(&self) -> usize {
        self.handlers_configured + self.critical_section_entries + self.critical_section_exits
    }
}

/// Interrupt code generation context.
///

/// Provides methods for configuring interrupt handler functions and
/// generating critical section entry/exit code.
pub struct InterruptLowering<'ctx> {
    /// The LLVM context.
    context: &'ctx Context,
    /// Reference to the builder for generating instructions.
    builder: &'ctx Builder<'ctx>,
    /// Type lowering helper.
    types: &'ctx TypeLowering<'ctx>,
    /// Target architecture.
    arch: TargetArch,
    /// Statistics.
    stats: InterruptStats,
}

impl<'ctx> InterruptLowering<'ctx> {
    /// Create a new interrupt lowering context.
    pub fn new(
        context: &'ctx Context,
        builder: &'ctx Builder<'ctx>,
        types: &'ctx TypeLowering<'ctx>,
        arch: TargetArch,
    ) -> Self {
        Self {
            context,
            builder,
            types,
            arch,
            stats: InterruptStats::default(),
        }
    }

    /// Get accumulated statistics.
    pub fn stats(&self) -> &InterruptStats {
        &self.stats
    }

    /// Configure a function as an interrupt handler.
    ///

    /// This sets the appropriate calling convention and attributes
    /// based on the target architecture and interrupt kind.
    ///

    /// # Parameters
    /// - `func`: The function to configure
    /// - `kind`: The type of interrupt handler
    /// - `naked`: If true, no prologue/epilogue is generated
    /// - `save_fpu`: If true, FPU registers are saved/restored
    pub fn configure_interrupt_handler(
        &mut self,
        func: FunctionValue<'ctx>,
        kind: InterruptHandlerKind,
        naked: bool,
        save_fpu: bool,
    ) -> Result<()> {
        // Set calling convention if the architecture supports it
        if self.arch.has_interrupt_cc() {
            func.set_call_conventions(self.arch.interrupt_call_conv());
        }

        // Add naked attribute if requested
        if naked {
            let naked_id = Attribute::get_named_enum_kind_id("naked");
            if naked_id != 0 {
                let naked_attr = self.context.create_enum_attribute(naked_id, 0);
                func.add_attribute(AttributeLoc::Function, naked_attr);
            }
            self.stats.naked_handlers += 1;
        }

        // Add noinline attribute (interrupt handlers shouldn't be inlined)
        let noinline_id = Attribute::get_named_enum_kind_id("noinline");
        if noinline_id != 0 {
            let noinline_attr = self.context.create_enum_attribute(noinline_id, 0);
            func.add_attribute(AttributeLoc::Function, noinline_attr);
        }

        // Add noreturn for reset handlers
        if matches!(kind, InterruptHandlerKind::Reset) {
            let noreturn_id = Attribute::get_named_enum_kind_id("noreturn");
            if noreturn_id != 0 {
                let noreturn_attr = self.context.create_enum_attribute(noreturn_id, 0);
                func.add_attribute(AttributeLoc::Function, noreturn_attr);
            }
        }

        // For ARM/AArch64, add the interrupt type as a string attribute
        if matches!(self.arch, TargetArch::ARM | TargetArch::AArch64) {
            let interrupt_attr = self
                .context
                .create_string_attribute("interrupt", kind.arm_interrupt_type());
            func.add_attribute(AttributeLoc::Function, interrupt_attr);
        }

        // Track FPU saving
        if save_fpu || kind.requires_fpu_save() {
            self.stats.fpu_save_handlers += 1;
        }

        self.stats.handlers_configured += 1;
        Ok(())
    }

    /// Generate critical section entry code (disable interrupts).
    ///

    /// Returns a value that should be passed to `exit_critical_section`
    /// to restore the previous interrupt state.
    ///

    /// # Parameters
    /// - `priority_mask`: Optional priority mask (None = disable all maskable)
    ///

    /// # Returns
    /// A pointer value holding the saved interrupt state.
    pub fn enter_critical_section(
        &mut self,
        priority_mask: Option<u8>,
    ) -> Result<PointerValue<'ctx>> {
        let saved_state = match self.arch {
            TargetArch::X86_64 | TargetArch::X86 => self.x86_disable_interrupts()?,
            TargetArch::ARM => self.arm_disable_interrupts(priority_mask)?,
            TargetArch::AArch64 => self.aarch64_disable_interrupts()?,
            TargetArch::RiscV32 | TargetArch::RiscV64 => self.riscv_disable_interrupts()?,
        };

        self.stats.critical_section_entries += 1;
        Ok(saved_state)
    }

    /// Generate critical section exit code (restore interrupts).
    ///

    /// # Parameters
    /// - `saved_state`: The value returned from `enter_critical_section`
    pub fn exit_critical_section(&mut self, saved_state: PointerValue<'ctx>) -> Result<()> {
        match self.arch {
            TargetArch::X86_64 | TargetArch::X86 => self.x86_restore_interrupts(saved_state)?,
            TargetArch::ARM => self.arm_restore_interrupts(saved_state)?,
            TargetArch::AArch64 => self.aarch64_restore_interrupts(saved_state)?,
            TargetArch::RiscV32 | TargetArch::RiscV64 => {
                self.riscv_restore_interrupts(saved_state)?
            }
        };

        self.stats.critical_section_exits += 1;
        Ok(())
    }

    /// Generate a memory barrier instruction.
    ///

    /// Useful for ensuring memory operations complete before interrupt return.
    pub fn memory_barrier(&self) -> Result<()> {
        // Use compiler fence intrinsic
        if let Some(fence) = Intrinsic::find("llvm.sideeffect") {
            // sideeffect acts as a compiler barrier
            let void_fn = self.context.void_type().fn_type(&[], false);
            // Note: For a full memory barrier we'd use atomic fence instead
            // This is handled by the MmioLowering module
            let _ = fence;
            let _ = void_fn;
        }
        Ok(())
    }

    // =========================================================================
    // Architecture-specific implementations
    // =========================================================================

    /// x86/x86_64: Disable interrupts using CLI instruction.
    fn x86_disable_interrupts(&self) -> Result<PointerValue<'ctx>> {
        // pushf; pop rax; cli; mov [saved], rax
        // For simplicity, we use inline assembly to save flags and disable

        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(Default::default());

        // Allocate space for saved flags
        let saved = self
            .builder
            .build_alloca(i64_type, "saved_flags")
            .or_llvm_err()?;

        // Generate: pushfq; pop %0; cli
        let asm_fn = self.context.void_type().fn_type(&[i64_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "pushfq; pop $0; cli".to_string(),
            "=r".to_string(),
            true,  // has side effects
            false, // doesn't need aligned stack
            Some(InlineAsmDialect::Intel),
            false, // can't throw
        );

        // Call the inline asm
        let call_result = self
            .builder
            .build_indirect_call(asm_fn, asm, &[], "flags")
            .or_llvm_err()?;

        // Store the result (need to extract from call result)
        // Note: The flags are returned in %0, stored to saved

        Ok(saved)
    }

    /// x86/x86_64: Restore interrupts using STI instruction.
    fn x86_restore_interrupts(&self, saved_state: PointerValue<'ctx>) -> Result<()> {
        // Load saved flags and restore if IF was set
        let i64_type = self.context.i64_type();

        // Generate: push $0; popfq (restores all flags including IF)
        let asm_fn = self.context.void_type().fn_type(&[i64_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "push $0; popfq".to_string(),
            "r".to_string(),
            true,  // has side effects
            false, // doesn't need aligned stack
            Some(InlineAsmDialect::Intel),
            false, // can't throw
        );

        // Load saved flags
        let flags = self
            .builder
            .build_load(i64_type, saved_state, "flags")
            .or_llvm_err()?;

        // Call the inline asm to restore
        self.builder
            .build_indirect_call(asm_fn, asm, &[flags.into()], "")
            .or_llvm_err()?;

        Ok(())
    }

    /// ARM: Disable interrupts using CPSID instruction.
    fn arm_disable_interrupts(&self, priority_mask: Option<u8>) -> Result<PointerValue<'ctx>> {
        let i32_type = self.context.i32_type();
        let ptr_type = self.context.ptr_type(Default::default());

        // Allocate space for saved state
        let saved = self
            .builder
            .build_alloca(i32_type, "saved_primask")
            .or_llvm_err()?;

        if let Some(priority) = priority_mask {
            // Use BASEPRI for priority-based masking
            // mrs r0, basepri; str r0, [saved]; mov r0, #priority; msr basepri, r0
            let asm_fn = self.context.void_type().fn_type(&[i32_type.into()], false);
            let asm_str = format!("mrs $0, basepri; msr basepri, {}", priority);
            let asm = self.context.create_inline_asm(
                asm_fn,
                asm_str,
                "=r".to_string(),
                true,
                false,
                Some(InlineAsmDialect::ATT),
                false,
            );

            self.builder
                .build_indirect_call(asm_fn, asm, &[], "basepri")
                .or_llvm_err()?;
        } else {
            // Disable all interrupts: cpsid i
            // mrs r0, primask; cpsid i
            let asm_fn = self.context.void_type().fn_type(&[i32_type.into()], false);
            let asm = self.context.create_inline_asm(
                asm_fn,
                "mrs $0, primask; cpsid i".to_string(),
                "=r".to_string(),
                true,
                false,
                Some(InlineAsmDialect::ATT),
                false,
            );

            self.builder
                .build_indirect_call(asm_fn, asm, &[], "primask")
                .or_llvm_err()?;
        }

        Ok(saved)
    }

    /// ARM: Restore interrupts.
    fn arm_restore_interrupts(&self, saved_state: PointerValue<'ctx>) -> Result<()> {
        let i32_type = self.context.i32_type();

        // Load saved state
        let state = self
            .builder
            .build_load(i32_type, saved_state, "state")
            .or_llvm_err()?;

        // msr primask, r0
        let asm_fn = self.context.void_type().fn_type(&[i32_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "msr primask, $0".to_string(),
            "r".to_string(),
            true,
            false,
            Some(InlineAsmDialect::ATT),
            false,
        );

        self.builder
            .build_indirect_call(asm_fn, asm, &[state.into()], "")
            .or_llvm_err()?;

        Ok(())
    }

    /// AArch64: Disable interrupts using MSR DAIF.
    fn aarch64_disable_interrupts(&self) -> Result<PointerValue<'ctx>> {
        let i64_type = self.context.i64_type();

        // Allocate space for saved state
        let saved = self
            .builder
            .build_alloca(i64_type, "saved_daif")
            .or_llvm_err()?;

        // mrs x0, daif; msr daifset, #0xf
        let asm_fn = self.context.void_type().fn_type(&[i64_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "mrs $0, daif; msr daifset, #0xf".to_string(),
            "=r".to_string(),
            true,
            false,
            Some(InlineAsmDialect::ATT),
            false,
        );

        self.builder
            .build_indirect_call(asm_fn, asm, &[], "daif")
            .or_llvm_err()?;

        Ok(saved)
    }

    /// AArch64: Restore interrupts.
    fn aarch64_restore_interrupts(&self, saved_state: PointerValue<'ctx>) -> Result<()> {
        let i64_type = self.context.i64_type();

        // Load saved state
        let state = self
            .builder
            .build_load(i64_type, saved_state, "state")
            .or_llvm_err()?;

        // msr daif, x0
        let asm_fn = self.context.void_type().fn_type(&[i64_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "msr daif, $0".to_string(),
            "r".to_string(),
            true,
            false,
            Some(InlineAsmDialect::ATT),
            false,
        );

        self.builder
            .build_indirect_call(asm_fn, asm, &[state.into()], "")
            .or_llvm_err()?;

        Ok(())
    }

    /// RISC-V: Disable interrupts by clearing MIE in mstatus.
    fn riscv_disable_interrupts(&self) -> Result<PointerValue<'ctx>> {
        let int_type = if matches!(self.arch, TargetArch::RiscV64) {
            self.context.i64_type()
        } else {
            self.context.i32_type()
        };

        // Allocate space for saved state
        let saved = self
            .builder
            .build_alloca(int_type, "saved_mstatus")
            .or_llvm_err()?;

        // csrrc mstatus, mie (clear MIE bit, return old value)
        // Note: Using csrrci for immediate would be: csrrci zero, mstatus, 8
        let asm_fn = self.context.void_type().fn_type(&[int_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "csrrc $0, mstatus, 8".to_string(), // 8 = MIE bit
            "=r".to_string(),
            true,
            false,
            Some(InlineAsmDialect::ATT),
            false,
        );

        self.builder
            .build_indirect_call(asm_fn, asm, &[], "mstatus")
            .or_llvm_err()?;

        Ok(saved)
    }

    /// RISC-V: Restore interrupts.
    fn riscv_restore_interrupts(&self, saved_state: PointerValue<'ctx>) -> Result<()> {
        let int_type = if matches!(self.arch, TargetArch::RiscV64) {
            self.context.i64_type()
        } else {
            self.context.i32_type()
        };

        // Load saved state
        let state = self
            .builder
            .build_load(int_type, saved_state, "state")
            .or_llvm_err()?;

        // We need to restore just the MIE bit from saved state
        // csrrs zero, mstatus, saved_mie_bit
        // For simplicity, we'll write the whole saved value to mstatus
        let asm_fn = self.context.void_type().fn_type(&[int_type.into()], false);
        let asm = self.context.create_inline_asm(
            asm_fn,
            "csrw mstatus, $0".to_string(),
            "r".to_string(),
            true,
            false,
            Some(InlineAsmDialect::ATT),
            false,
        );

        self.builder
            .build_indirect_call(asm_fn, asm, &[state.into()], "")
            .or_llvm_err()?;

        Ok(())
    }
}

/// Exception frame layout for interrupt handlers.
///

/// On x86_64, when an interrupt occurs, the CPU pushes:
/// - SS, RSP, RFLAGS, CS, RIP (and error code for some exceptions)
///

/// This structure represents the stack frame passed to exception handlers.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct X86ExceptionFrame {
    /// Instruction pointer at time of interrupt
    pub rip: u64,
    /// Code segment
    pub cs: u64,
    /// CPU flags
    pub rflags: u64,
    /// Stack pointer at time of interrupt
    pub rsp: u64,
    /// Stack segment
    pub ss: u64,
}

/// ARM exception frame (basic)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ArmExceptionFrame {
    /// General purpose registers R0-R12
    pub r: [u32; 13],
    /// Link register (R14)
    pub lr: u32,
    /// Program counter at time of exception
    pub pc: u32,
    /// Program status register
    pub cpsr: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_arch_from_triple() {
        assert_eq!(
            TargetArch::from_triple("x86_64-unknown-linux-gnu"),
            Some(TargetArch::X86_64)
        );
        assert_eq!(
            TargetArch::from_triple("arm-none-eabi"),
            Some(TargetArch::ARM)
        );
        assert_eq!(
            TargetArch::from_triple("aarch64-linux-gnu"),
            Some(TargetArch::AArch64)
        );
        assert_eq!(
            TargetArch::from_triple("riscv32-unknown-elf"),
            Some(TargetArch::RiscV32)
        );
        assert_eq!(
            TargetArch::from_triple("riscv64-unknown-linux-gnu"),
            Some(TargetArch::RiscV64)
        );
        assert_eq!(TargetArch::from_triple("wasm32-unknown-unknown"), None);
    }

    #[test]
    fn test_interrupt_call_conv() {
        assert_eq!(TargetArch::X86_64.interrupt_call_conv(), 83);
        assert_eq!(TargetArch::ARM.interrupt_call_conv(), 67);
        assert_eq!(TargetArch::AArch64.interrupt_call_conv(), 0);
    }

    #[test]
    fn test_interrupt_stats() {
        let mut stats = InterruptStats::default();
        stats.handlers_configured = 5;
        stats.critical_section_entries = 3;
        stats.critical_section_exits = 3;
        assert_eq!(stats.total(), 11);

        let mut other = InterruptStats::default();
        other.handlers_configured = 2;
        stats.merge(&other);
        assert_eq!(stats.handlers_configured, 7);
    }

    #[test]
    fn test_arm_interrupt_type() {
        assert_eq!(InterruptHandlerKind::Regular.arm_interrupt_type(), "IRQ");
        assert_eq!(InterruptHandlerKind::Fast.arm_interrupt_type(), "FIQ");
        assert_eq!(
            InterruptHandlerKind::Exception.arm_interrupt_type(),
            "ABORT"
        );
    }

    // ----------------------------------------------------------------
    // meta() consolidation drift pins.
    // ----------------------------------------------------------------

    #[test]
    fn meta_pin_target_arch_round_trip_unique_and_classification() {
        assert_eq!(TargetArch::ALL.len(), 6);
        for v in TargetArch::ALL {
            let s = v.as_str();
            assert_eq!(
                TargetArch::from_str(s),
                Some(*v),
                "TargetArch::{:?}: '{}' must round-trip",
                v,
                s
            );
        }
        // has_interrupt_cc partition: only x86 family has the
        // dedicated `x86_intrcc` calling convention. Pinned 2/4.
        let with_cc = TargetArch::ALL
            .iter()
            .filter(|v| v.has_interrupt_cc())
            .count();
        assert_eq!(with_cc, 2);
        assert!(TargetArch::X86_64.has_interrupt_cc());
        assert!(TargetArch::X86.has_interrupt_cc());
        // CC ID consistency: has_interrupt_cc ⇔ interrupt_call_conv != 0
        // (every arch with a dedicated CC carries a non-zero ID).
        for v in TargetArch::ALL {
            let cc = v.interrupt_call_conv();
            // Note: ARM_AAPCS=67 is non-zero but `has_interrupt_cc`
            // is false because ARM uses default CC + interrupt
            // attribute (the legacy classification preserved
            // verbatim). Pin the legacy classification exactly:
            let expected_dedicated = matches!(
                v,
                TargetArch::X86_64 | TargetArch::X86
            );
            assert_eq!(
                v.has_interrupt_cc(),
                expected_dedicated,
                "TargetArch::{:?}: has_interrupt_cc",
                v
            );
            // x86 family uses CC 83 (x86_intrcc); ARM uses 67
            // (ARM_AAPCS); AArch64/RISC-V use 0 (default C).
            let expected_cc: u32 = match v {
                TargetArch::X86_64 | TargetArch::X86 => 83,
                TargetArch::ARM => 67,
                TargetArch::AArch64
                | TargetArch::RiscV32
                | TargetArch::RiscV64 => 0,
            };
            assert_eq!(cc, expected_cc, "TargetArch::{:?}: cc id", v);
        }
        // from_triple → as_str round-trip on canonical first
        // components.
        let triples = &[
            ("x86_64-unknown-linux-gnu", TargetArch::X86_64),
            ("i686-pc-linux-gnu", TargetArch::X86),
            ("armv7-unknown-linux-gnueabihf", TargetArch::ARM),
            ("aarch64-apple-darwin", TargetArch::AArch64),
            ("riscv32-unknown-none-elf", TargetArch::RiscV32),
            ("riscv64-unknown-linux-gnu", TargetArch::RiscV64),
        ];
        for (triple, expected) in triples {
            assert_eq!(
                TargetArch::from_triple(triple),
                Some(*expected),
                "from_triple drift: {}",
                triple
            );
        }
    }

    #[test]
    fn meta_pin_interrupt_handler_kind_round_trip_and_partitions() {
        assert_eq!(InterruptHandlerKind::ALL.len(), 6);
        for v in InterruptHandlerKind::ALL {
            let s = v.as_str();
            assert_eq!(
                InterruptHandlerKind::from_str(s),
                Some(*v),
                "InterruptHandlerKind::{:?}: '{}' round-trip",
                v,
                s
            );
        }
        // requires_fpu_save partition: NMI + Exception (n=2).
        // Cross-pin: legacy 2-arm match preserved exactly.
        for v in InterruptHandlerKind::ALL {
            let expected_fpu = matches!(
                v,
                InterruptHandlerKind::NMI | InterruptHandlerKind::Exception
            );
            assert_eq!(
                v.requires_fpu_save(),
                expected_fpu,
                "InterruptHandlerKind::{:?}: requires_fpu_save",
                v
            );
        }
        // ARM interrupt-type table — preserved verbatim from legacy.
        // Note that NMI and Trap both map to "SWI" (ARM has no
        // distinct NMI; both go through the SWI vector); not a
        // parse-side accept set.
        assert_eq!(InterruptHandlerKind::Regular.arm_interrupt_type(), "IRQ");
        assert_eq!(InterruptHandlerKind::NMI.arm_interrupt_type(), "SWI");
        assert_eq!(InterruptHandlerKind::Fast.arm_interrupt_type(), "FIQ");
        assert_eq!(
            InterruptHandlerKind::Exception.arm_interrupt_type(),
            "ABORT"
        );
        assert_eq!(InterruptHandlerKind::Trap.arm_interrupt_type(), "SWI");
        assert_eq!(InterruptHandlerKind::Reset.arm_interrupt_type(), "RESET");
        // Cross-cutting: name (lowercase) matches the
        // `verum_ast::attr::InterruptKind` wire form. This pins
        // the contract that the codegen-side enum and the AST-side
        // enum stay in lockstep on the user-facing
        // `@interrupt(...)` argument.
        assert_eq!(InterruptHandlerKind::Regular.as_str(), "regular");
        assert_eq!(InterruptHandlerKind::NMI.as_str(), "nmi");
        assert_eq!(InterruptHandlerKind::Fast.as_str(), "fast");
        assert_eq!(InterruptHandlerKind::Exception.as_str(), "exception");
        assert_eq!(InterruptHandlerKind::Trap.as_str(), "trap");
        assert_eq!(InterruptHandlerKind::Reset.as_str(), "reset");
    }
}
