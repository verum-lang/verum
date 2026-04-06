//! Call stack management.
//!
//! The call stack tracks the execution context for nested function calls.
//! Each call creates a new [`CallFrame`] that stores:
//! - Function being executed
//! - Program counter (instruction offset)
//! - Register base offset
//! - Return information
//!
//! # Stack Limits
//!
//! The default stack limit is 1024 frames, which is sufficient for most
//! programs while preventing stack overflow attacks.

use crate::instruction::Reg;
use crate::module::FunctionId;
use super::error::{InterpreterError, InterpreterResult};

/// Default maximum call stack depth.
pub const DEFAULT_MAX_DEPTH: usize = 1024;

/// Single call frame on the stack.
///
/// Each frame represents a function invocation with its own:
/// - Instruction pointer (pc)
/// - Register window (base offset)
/// - Return destination
#[derive(Debug, Clone, Copy)]
pub struct CallFrame {
    /// Function being executed.
    pub function: FunctionId,

    /// Program counter (byte offset into bytecode).
    pub pc: u32,

    /// Base offset in register file.
    pub reg_base: u32,

    /// Number of registers in this frame.
    pub reg_count: u16,

    /// Return address (caller's pc to resume at).
    pub return_pc: u32,

    /// Register to store return value in caller's frame.
    pub return_reg: Reg,

    /// Caller's register base (for storing return value).
    pub caller_base: u32,
}

impl CallFrame {
    /// Creates a new call frame.
    pub fn new(
        function: FunctionId,
        reg_base: u32,
        reg_count: u16,
        return_pc: u32,
        return_reg: Reg,
        caller_base: u32,
    ) -> Self {
        Self {
            function,
            pc: 0,
            reg_base,
            reg_count,
            return_pc,
            return_reg,
            caller_base,
        }
    }
}

/// Call stack for managing function invocations.
///
/// The call stack is a LIFO structure that tracks the execution context
/// for nested function calls. It provides:
/// - Push/pop for call/return
/// - Access to current and parent frames
/// - Stack depth limiting
#[derive(Debug)]
pub struct CallStack {
    /// Stack of call frames.
    frames: Vec<CallFrame>,

    /// Maximum allowed depth.
    max_depth: usize,
}

impl Default for CallStack {
    fn default() -> Self {
        Self::new()
    }
}

impl CallStack {
    /// Creates a new call stack with default maximum depth.
    pub fn new() -> Self {
        Self::with_max_depth(DEFAULT_MAX_DEPTH)
    }

    /// Creates a new call stack with the specified maximum depth.
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self {
            frames: Vec::with_capacity(32),
            max_depth,
        }
    }

    /// Pushes a new call frame.
    ///
    /// Returns the register base for the new frame.
    pub fn push_frame(
        &mut self,
        function: FunctionId,
        reg_count: u16,
        return_pc: u32,
        return_reg: Reg,
    ) -> InterpreterResult<u32> {
        if self.frames.len() >= self.max_depth {
            return Err(InterpreterError::StackOverflow {
                depth: self.frames.len(),
                max_depth: self.max_depth,
            });
        }

        // Calculate new base (after previous frame's registers)
        let (reg_base, caller_base) = if let Some(current) = self.frames.last() {
            (current.reg_base + current.reg_count as u32, current.reg_base)
        } else {
            (0, 0)
        };

        let frame = CallFrame::new(
            function,
            reg_base,
            reg_count,
            return_pc,
            return_reg,
            caller_base,
        );

        self.frames.push(frame);
        Ok(reg_base)
    }

    /// Pops the current call frame.
    ///
    /// Returns the popped frame.
    pub fn pop_frame(&mut self) -> InterpreterResult<CallFrame> {
        self.frames.pop().ok_or(InterpreterError::StackUnderflow)
    }

    /// Returns the current (topmost) frame.
    pub fn current(&self) -> Option<&CallFrame> {
        self.frames.last()
    }

    /// Returns a mutable reference to the current frame.
    pub fn current_mut(&mut self) -> Option<&mut CallFrame> {
        self.frames.last_mut()
    }

    /// Returns the parent frame (one level up).
    pub fn parent(&self) -> Option<&CallFrame> {
        if self.frames.len() >= 2 {
            self.frames.get(self.frames.len() - 2)
        } else {
            None
        }
    }

    /// Returns the frame at the specified depth (0 = bottom, len-1 = top).
    pub fn at(&self, depth: usize) -> Option<&CallFrame> {
        self.frames.get(depth)
    }

    /// Returns the current stack depth.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Returns true if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Clears all frames.
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Returns an iterator over frames from bottom to top.
    pub fn iter(&self) -> impl Iterator<Item = &CallFrame> {
        self.frames.iter()
    }

    /// Returns an iterator over frames from top to bottom.
    pub fn iter_rev(&self) -> impl Iterator<Item = &CallFrame> {
        self.frames.iter().rev()
    }

    /// Generates a stack trace as a vector of (function_id, pc) pairs.
    pub fn stack_trace(&self) -> Vec<(FunctionId, u32)> {
        self.frames
            .iter()
            .rev()
            .map(|f| (f.function, f.pc))
            .collect()
    }

    /// Returns the current register base.
    pub fn reg_base(&self) -> u32 {
        self.frames.last().map(|f| f.reg_base).unwrap_or(0)
    }

    /// Advances the program counter of the current frame.
    #[inline]
    pub fn advance_pc(&mut self, delta: u32) {
        if let Some(frame) = self.frames.last_mut() {
            frame.pc += delta;
        }
    }

    /// Sets the program counter of the current frame.
    #[inline]
    pub fn set_pc(&mut self, pc: u32) {
        if let Some(frame) = self.frames.last_mut() {
            frame.pc = pc;
        }
    }

    /// Gets the program counter of the current frame.
    #[inline]
    pub fn pc(&self) -> u32 {
        self.frames.last().map(|f| f.pc).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_stack_creation() {
        let stack = CallStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn test_push_pop_frame() {
        let mut stack = CallStack::new();

        // Push first frame
        let base1 = stack.push_frame(FunctionId(0), 16, 0, Reg(0)).unwrap();
        assert_eq!(base1, 0);
        assert_eq!(stack.depth(), 1);
        assert!(!stack.is_empty());

        // Push second frame
        let base2 = stack.push_frame(FunctionId(1), 8, 10, Reg(5)).unwrap();
        assert_eq!(base2, 16);
        assert_eq!(stack.depth(), 2);

        // Check current frame
        let current = stack.current().unwrap();
        assert_eq!(current.function, FunctionId(1));
        assert_eq!(current.reg_base, 16);
        assert_eq!(current.reg_count, 8);

        // Pop frame
        let popped = stack.pop_frame().unwrap();
        assert_eq!(popped.function, FunctionId(1));
        assert_eq!(stack.depth(), 1);

        // Pop last frame
        let popped = stack.pop_frame().unwrap();
        assert_eq!(popped.function, FunctionId(0));
        assert!(stack.is_empty());
    }

    #[test]
    fn test_stack_overflow() {
        let mut stack = CallStack::with_max_depth(3);

        stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        stack.push_frame(FunctionId(1), 4, 0, Reg(0)).unwrap();
        stack.push_frame(FunctionId(2), 4, 0, Reg(0)).unwrap();

        // Fourth push should fail
        let result = stack.push_frame(FunctionId(3), 4, 0, Reg(0));
        assert!(matches!(result, Err(InterpreterError::StackOverflow { .. })));
    }

    #[test]
    fn test_stack_underflow() {
        let mut stack = CallStack::new();
        let result = stack.pop_frame();
        assert!(matches!(result, Err(InterpreterError::StackUnderflow)));
    }

    #[test]
    fn test_parent_frame() {
        let mut stack = CallStack::new();

        // No parent for first frame
        stack.push_frame(FunctionId(0), 16, 0, Reg(0)).unwrap();
        assert!(stack.parent().is_none());

        // Parent exists after second push
        stack.push_frame(FunctionId(1), 8, 10, Reg(5)).unwrap();
        let parent = stack.parent().unwrap();
        assert_eq!(parent.function, FunctionId(0));
    }

    #[test]
    fn test_stack_trace() {
        let mut stack = CallStack::new();

        stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        stack.current_mut().unwrap().pc = 100;

        stack.push_frame(FunctionId(1), 4, 0, Reg(0)).unwrap();
        stack.current_mut().unwrap().pc = 200;

        stack.push_frame(FunctionId(2), 4, 0, Reg(0)).unwrap();
        stack.current_mut().unwrap().pc = 300;

        let trace = stack.stack_trace();
        assert_eq!(trace.len(), 3);
        assert_eq!(trace[0], (FunctionId(2), 300)); // Top
        assert_eq!(trace[1], (FunctionId(1), 200));
        assert_eq!(trace[2], (FunctionId(0), 100)); // Bottom
    }

    #[test]
    fn test_pc_operations() {
        let mut stack = CallStack::new();
        stack.push_frame(FunctionId(0), 16, 0, Reg(0)).unwrap();

        assert_eq!(stack.pc(), 0);

        stack.advance_pc(5);
        assert_eq!(stack.pc(), 5);

        stack.set_pc(100);
        assert_eq!(stack.pc(), 100);

        stack.advance_pc(10);
        assert_eq!(stack.pc(), 110);
    }

    #[test]
    fn test_frame_at() {
        let mut stack = CallStack::new();

        stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        stack.push_frame(FunctionId(1), 4, 0, Reg(0)).unwrap();
        stack.push_frame(FunctionId(2), 4, 0, Reg(0)).unwrap();

        assert_eq!(stack.at(0).unwrap().function, FunctionId(0));
        assert_eq!(stack.at(1).unwrap().function, FunctionId(1));
        assert_eq!(stack.at(2).unwrap().function, FunctionId(2));
        assert!(stack.at(3).is_none());
    }

    #[test]
    fn test_clear() {
        let mut stack = CallStack::new();
        stack.push_frame(FunctionId(0), 4, 0, Reg(0)).unwrap();
        stack.push_frame(FunctionId(1), 4, 0, Reg(0)).unwrap();

        assert_eq!(stack.depth(), 2);

        stack.clear();
        assert!(stack.is_empty());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn test_iterators() {
        let mut stack = CallStack::new();

        for i in 0..5 {
            stack.push_frame(FunctionId(i), 4, 0, Reg(0)).unwrap();
        }

        // Forward iterator (bottom to top)
        let forward: Vec<_> = stack.iter().map(|f| f.function.0).collect();
        assert_eq!(forward, vec![0, 1, 2, 3, 4]);

        // Reverse iterator (top to bottom)
        let reverse: Vec<_> = stack.iter_rev().map(|f| f.function.0).collect();
        assert_eq!(reverse, vec![4, 3, 2, 1, 0]);
    }

    #[test]
    fn test_return_info() {
        let mut stack = CallStack::new();

        // Push caller
        stack.push_frame(FunctionId(0), 16, 0, Reg(0)).unwrap();
        stack.current_mut().unwrap().pc = 50; // Position in caller

        // Push callee with return info
        stack.push_frame(FunctionId(1), 8, 50, Reg(10)).unwrap();

        let callee = stack.current().unwrap();
        assert_eq!(callee.return_pc, 50);
        assert_eq!(callee.return_reg, Reg(10));
        assert_eq!(callee.caller_base, 0);
    }
}
