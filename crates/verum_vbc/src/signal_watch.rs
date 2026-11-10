//! The wire contract for observing asynchronous Linux signal deliveries.
use crate::encoding::encode_reg;
use crate::instruction::{Instruction, Reg, SysSubOpcode};

/// Install takes a signal number and returns a positive ownership ticket.
/// Take and Restore accept that ticket. Negative results are Linux errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SignalWatchOp {
    /// Reserve the disposition and install the native counter.
    Install = 0,
    /// Atomically drain completed handler invocations.
    Take = 1,
    /// Restore the complete disposition saved at installation.
    Restore = 2,
}

impl SignalWatchOp {
    /// Process permission target. Tickets grant no authority over a different signal.
    pub fn permission_target(self, operand: i64) -> u64 {
        if self == Self::Install {
            operand as u64
        } else {
            ((operand as u64).wrapping_sub(1) & 63) + 1
        }
    }

    /// ABI for a source-less declaration; matching register widths alone is insufficient.
    pub fn accepts_declaration(self, decl: &crate::module::FunctionDescriptor) -> bool {
        matches!(decl.params.as_slice(), [param] if self.accepts_signature(Some(&param.type_ref), Some(&decl.return_type)))
    }

    /// Both the source producer and consumers of decoded VBC enforce the ABI.
    pub fn accepts_signature(
        self,
        arg: Option<&crate::types::TypeRef>,
        ret: Option<&crate::types::TypeRef>,
    ) -> bool {
        let int = crate::types::TypeRef::Concrete(crate::types::TypeId::INT);
        arg == Some(&int) && ret == Some(&int)
    }
    /// Refuse unrecognized operations, including on the wire.
    pub fn decode(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Install),
            1 => Some(Self::Take),
            2 => Some(Self::Restore),
            _ => None,
        }
    }

    /// The same instruction is used at call sites and inside callable thunks.
    pub fn instruction(self, dst: Reg, operand: Reg) -> Instruction {
        let mut operands = vec![self as u8];
        encode_reg(dst, &mut operands);
        encode_reg(operand, &mut operands);
        Instruction::SysExtended {
            sub_op: SysSubOpcode::LinuxSignalWatch.to_byte(),
            operands,
        }
    }

    /// A ticket's low six bits (after subtracting one) identify its signal.
    pub fn signal(self, operand: i64) -> Option<usize> {
        if operand < 1 || (self == Self::Install && operand > 64) {
            return None;
        }
        Some(((operand - 1) as usize & 63) + 1)
    }
}

/// Linux LP64 kernel signal ABI (not libc's sigaction structure).
pub mod abi {
    /// Kernel signal set size on x86-64 and AArch64.
    pub const MASK_BYTES: u64 = 8;
    /// Restart interruptible system calls where Linux permits it.
    pub const SA_RESTART: u64 = 0x1000_0000;
    /// x86-64 requires a userspace rt_sigreturn trampoline; AArch64 supplies a vDSO trampoline.
    pub const SA_RESTORER: u64 = 0x0400_0000;
    pub use verum_common::linux_syscalls::{
        aarch64::SYS_RT_SIGACTION as ARM_SIGACTION,
        x86_64::{SYS_RT_SIGACTION as X86_SIGACTION, SYS_RT_SIGRETURN as X86_SIGRETURN},
    };
}
