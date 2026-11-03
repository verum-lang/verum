//! Measurements of actual, decoded CallM attempts in a single interpreter.
use super::dispatch_table::DispatchResult;
use super::{InterpreterResult, InterpreterState};
use crate::module::{FunctionId, VbcModule};
use crate::types::StringId;
use indexmap::IndexMap;
use std::sync::Arc;
use std::time::Instant;
use verum_common::Text;
use verum_common::runtime_diagnostics::{
    MAX_DISPATCH_NAME_BYTES, MAX_METHOD_DISPATCH_SITES, MethodDispatchOutcome,
    MethodDispatchReport, MethodDispatchRow, MethodDispatchSchema,
};

#[derive(Debug, Hash, PartialEq, Eq)]
struct Site {
    module: usize,
    function: FunctionId,
    pc: u32,
    selector: StringId,
    outcome: MethodDispatchOutcome,
}

#[derive(Debug)]
struct Sample {
    module: Arc<VbcModule>,
    module_instance: usize,
    count: u64,
    inclusive_ns: u64,
    max_ns: u64,
}

#[derive(Debug)]
pub(super) struct MethodProfile {
    capacity: usize,
    dropped: u64,
    rows: IndexMap<Site, Sample>,
}

impl InterpreterState {
    /// Start a fresh profile. A reset or another enable discards all observations.
    pub fn enable_method_dispatch_profile(
        &mut self,
        capacity: usize,
    ) -> Result<(), indexmap::TryReserveError> {
        self.method_profile = None;
        let capacity = capacity.min(MAX_METHOD_DISPATCH_SITES);
        let mut rows = IndexMap::new();
        rows.try_reserve(capacity)?;
        self.method_profile = Some(Box::new(MethodProfile {
            capacity,
            dropped: 0,
            rows,
        }));
        Ok(())
    }

    /// Called only on the instrumented path, after decoding the call operands.
    pub(super) fn profile_method_dispatch(
        &mut self,
        pc: u32,
        selector: StringId,
        dispatch: impl FnOnce(&mut Self) -> InterpreterResult<DispatchResult>,
    ) -> InterpreterResult<DispatchResult> {
        let caller = self
            .call_stack
            .current()
            .expect("dispatch requires a caller")
            .function;
        let depth = self.call_stack.depth();
        let module = Arc::clone(&self.module);
        let start = Instant::now();
        let result = dispatch(self);
        let elapsed = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let outcome = match &result {
            Err(_) => MethodDispatchOutcome::Failed,
            Ok(DispatchResult::Yield(_)) => MethodDispatchOutcome::Yielded,
            Ok(DispatchResult::Return(_) | DispatchResult::FinalReturn(_)) => {
                MethodDispatchOutcome::Returned
            }
            Ok(DispatchResult::Continue) if self.call_stack.depth() > depth => {
                let callee = self.call_stack.current().expect("entered frame");
                MethodDispatchOutcome::EnteredFrame {
                    function_id: callee.function.0,
                }
            }
            Ok(DispatchResult::Continue)
                if self.pc() == pc && self.pending_deref_receiver.is_some() =>
            {
                MethodDispatchOutcome::Retried
            }
            Ok(DispatchResult::Continue) => MethodDispatchOutcome::CompletedInline,
        };
        // Keep each recorded module alive so address reuse cannot merge two runs.
        let site = Site {
            module: Arc::as_ptr(&module) as usize,
            function: caller,
            pc,
            selector,
            outcome,
        };
        if let Some(capture) = &mut self.method_profile {
            capture.record(site, module, elapsed);
        }
        result
    }

    pub fn method_dispatch_report(&self) -> Option<MethodDispatchReport> {
        let capture = self.method_profile.as_ref()?;
        fn label(name: &str) -> Option<Text> {
            (name.len() <= MAX_DISPATCH_NAME_BYTES).then(|| name.into())
        }
        let rows = capture
            .rows
            .iter()
            .map(|(site, sample)| {
                let module = &sample.module;
                MethodDispatchRow {
                    module_instance: sample.module_instance,
                    module: label(&module.name),
                    function_id: site.function.0,
                    function: module
                        .get_function(site.function)
                        .and_then(|f| module.get_string(f.name))
                        .and_then(label),
                    pc: site.pc,
                    selector_id: site.selector.0,
                    selector: module.get_string(site.selector).and_then(label),
                    outcome: site.outcome,
                    count: sample.count,
                    inclusive_ns: sample.inclusive_ns,
                    max_ns: sample.max_ns,
                }
            })
            .collect();
        Some(MethodDispatchReport {
            schema: MethodDispatchSchema::V1,
            capacity: capture.capacity,
            dropped_calls: capture.dropped,
            rows,
        })
    }
}

impl MethodProfile {
    fn record(&mut self, site: Site, module: Arc<VbcModule>, elapsed: u64) {
        if let Some(sample) = self.rows.get_mut(&site) {
            sample.count = sample.count.saturating_add(1);
            sample.inclusive_ns = sample.inclusive_ns.saturating_add(elapsed);
            sample.max_ns = sample.max_ns.max(elapsed);
        } else if self.rows.len() == self.capacity {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            let module_instance = self
                .rows
                .values()
                .find(|sample| Arc::ptr_eq(&sample.module, &module))
                .map_or(self.rows.len(), |sample| sample.module_instance);
            self.rows.insert(
                site,
                Sample {
                    module,
                    module_instance,
                    count: 1,
                    inclusive_ns: elapsed,
                    max_ns: elapsed,
                },
            );
        }
    }
}
