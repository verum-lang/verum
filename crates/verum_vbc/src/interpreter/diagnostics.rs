//! Bounded observations from one interpreter's actual dispatch boundaries.
//! This records CBGR errors, not every reference/check/allocation. An empty
//! capture is not a proof of memory safety. Nested propagation can produce
//! several observations of one error, at different caller instructions.
use super::{CbgrFailure, CbgrTarget, InterpreterError, InterpreterState};
use crate::module::{FunctionId, VbcModule};
use std::sync::Arc;
use verum_common::{List, Text};

pub use verum_common::runtime_diagnostics::{
    CbgrEvent, CbgrReport, CbgrSchema, CbgrSourceLocation, CbgrTargetKind, MAX_CBGR_EVENTS,
};

/// An enabled capture belongs to one interpreter state. It neither mutates
/// program memory nor reads an address from a rejected reference. All event
/// storage is reserved up front; error recording itself does not allocate.
#[derive(Debug)]
pub(super) struct CbgrCapture {
    capacity: usize,
    checks_enabled_at_start: bool,
    truncated: bool,
    dispatched: bool,
    events: List<Observation>,
}

#[derive(Debug)]
struct Observation {
    site: DispatchSite,
    failure: CbgrFailure,
}

#[derive(Debug)]
pub(super) struct DispatchSite {
    module: Arc<VbcModule>,
    function: Option<FunctionId>,
    pc: u32,
    opcode: u8,
}

impl CbgrCapture {
    fn new(
        capacity: usize,
        checks_enabled_at_start: bool,
    ) -> Result<Self, std::collections::TryReserveError> {
        let capacity = capacity.min(MAX_CBGR_EVENTS);
        let mut events = List::new();
        events.try_reserve_exact(capacity)?;
        Ok(Self {
            capacity,
            checks_enabled_at_start,
            truncated: false,
            dispatched: false,
            events,
        })
    }

    fn observe(&mut self, site: DispatchSite, error: &InterpreterError) {
        let failure = match error {
            InterpreterError::CbgrViolation { kind, ptr } => CbgrFailure {
                kind: *kind,
                target: CbgrTarget::Address(*ptr),
                expected: None,
                actual: None,
            },
            InterpreterError::CbgrPanic { failure, .. } => *failure,
            _ => return,
        };
        if self.events.len() == self.capacity {
            self.truncated = true;
            return;
        }
        self.events.push(Observation { site, failure });
    }

    fn report(&self) -> CbgrReport {
        CbgrReport {
            schema: CbgrSchema::DispatchV1,
            capacity: self.capacity,
            checks_enabled_at_start: self.checks_enabled_at_start,
            truncated: self.truncated,
            events: self
                .events
                .iter()
                .map(|event| {
                    let site = &event.site;
                    let function = site.function.and_then(|id| site.module.get_function(id));
                    let name = function
                        .and_then(|f| site.module.get_string(f.name))
                        .map(|name| name.chars().take(256).collect::<Text>());
                    let source = site.function
                        .and_then(|id| site.module.instruction_source(id, site.pc))
                        .filter(|(path, _, _)| path.len() <= 2048)
                        .map(|(path, line, column)| CbgrSourceLocation {
                            path: path.into(), line, column,
                        });
                    let (target_kind, target) = match event.failure.target {
                        CbgrTarget::Register(index) => {
                            (CbgrTargetKind::Register, index.to_string())
                        }
                        CbgrTarget::Address(address) => {
                            (CbgrTargetKind::Address, format!("0x{address:x}"))
                        }
                    };
                    CbgrEvent {
                        kind: event.failure.kind,
                        target_kind,
                        target: target.into(),
                        expected: event.failure.expected.map(|value| value.to_string().into()),
                        actual: event.failure.actual.map(|value| value.to_string().into()),
                        function_id: site.function.map(|id| id.0),
                        function: name,
                        pc: site.pc,
                        opcode: site.opcode,
                        source,
                    }
                })
                .collect(),
        }
    }
}

impl InterpreterState {
    /// Enable a fresh, bounded capture. Calling again replaces the old run's
    /// observations. Disabled interpreters do not retain modules or events.
    pub fn enable_cbgr_diagnostics(
        &mut self,
        capacity: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        // If reserving the new capture fails, no earlier run may remain visible.
        self.cbgr_diagnostics = None;
        self.cbgr_diagnostics = Some(CbgrCapture::new(capacity, self.config.cbgr_enabled)?);
        Ok(())
    }

    /// No report before an actual handler dispatch: creating an interpreter or
    /// failing entry resolution is not an observed execution with zero faults.
    pub fn cbgr_report(&self) -> Option<CbgrReport> {
        self.cbgr_diagnostics
            .as_ref()
            .filter(|capture| capture.dispatched)
            .map(CbgrCapture::report)
    }

    #[inline]
    pub(super) fn cbgr_dispatch_site(&mut self, pc: u32, opcode: u8) -> Option<DispatchSite> {
        self.cbgr_diagnostics.as_mut()?.dispatched = true;
        Some(DispatchSite {
            module: Arc::clone(&self.module),
            function: self.call_stack.current().map(|frame| frame.function),
            pc,
            opcode,
        })
    }

    pub(super) fn observe_cbgr_dispatch(
        &mut self,
        site: Option<DispatchSite>,
        error: &InterpreterError,
    ) {
        if let (Some(capture), Some(site)) = (&mut self.cbgr_diagnostics, site) {
            capture.observe(site, error);
        }
    }
}
