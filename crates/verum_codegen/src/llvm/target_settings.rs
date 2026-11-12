//! One explicit CPU/feature selection for reflection, lowering and emission.
use super::error::{LlvmLoweringError, Result};
use verum_common::Text;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine, TargetTriple},
};

/// Owned target settings; no leaked host strings or architecture-name heuristics.
#[derive(Debug, Clone)]
pub struct TargetSettings {
    /// LLVM target triple.
    pub triple: Text,
    /// CPU name; empty selects the target's default CPU.
    pub cpu: Text,
    /// Explicit LLVM feature toggles.
    pub features: Text,
}

impl TargetSettings {
    /// A native invocation uses its detected host CPU. An explicit target uses
    /// LLVM's target defaults unless the caller supplies CPU/features.
    pub fn resolve(
        triple: Option<&str>,
        cpu: Option<&str>,
        features: Option<&str>,
    ) -> Result<Self> {
        let host = TargetMachine::get_default_triple();
        let host = host
            .as_str()
            .to_str()
            .map_err(|_| LlvmLoweringError::internal("invalid LLVM host triple"))?;
        let native = triple.is_none();
        let triple = triple.unwrap_or(host);
        if cpu == Some("native") && triple != host {
            return Err(LlvmLoweringError::unsupported(
                "native CPU requested for a different target",
            ));
        }
        let use_host = cpu == Some("native") || (native && cpu.is_none());
        let cpu = if use_host {
            TargetMachine::get_host_cpu_name()
                .to_str()
                .map_err(|_| LlvmLoweringError::internal("invalid LLVM CPU name"))?
                .into()
        } else {
            cpu.unwrap_or("").into()
        };
        let features = if features.is_none() && use_host {
            TargetMachine::get_host_cpu_features()
                .to_str()
                .map_err(|_| LlvmLoweringError::internal("invalid LLVM CPU features"))?
                .into()
        } else {
            features.unwrap_or("").into()
        };
        Ok(Self {
            triple: triple.into(),
            cpu,
            features,
        })
    }

    /// Construct a validated machine using the same selection as code emission.
    pub fn machine(&self) -> Result<TargetMachine> {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| Target::initialize_all(&InitializationConfig::default()));
        let triple = TargetTriple::create(&self.triple);
        let target = Target::from_triple(&triple)
            .map_err(|error| LlvmLoweringError::unsupported(error.to_string()))?;
        if !target.validate_subtarget(&triple, &self.cpu, &self.features) {
            return Err(LlvmLoweringError::unsupported(format!(
                "invalid CPU/features for target {}: {} / {}",
                self.triple, self.cpu, self.features
            )));
        }
        target
            .create_target_machine(
                &triple,
                &self.cpu,
                &self.features,
                OptimizationLevel::None,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| LlvmLoweringError::unsupported("target has no machine backend"))
    }

    /// Immutable VM facts from the selected machine, not the VM's build host.
    pub fn capabilities(&self) -> Result<verum_vbc::target::TargetCapabilities> {
        let machine = self.machine()?;
        let context = Context::create();
        let module = context.create_module("target_capabilities");
        module.set_triple(&TargetTriple::create(&self.triple));
        let function = module.add_function(
            "target_capabilities",
            context.void_type().fn_type(&[], false),
            None,
        );
        let info = machine
            .subtarget_info(function)
            .ok_or_else(|| LlvmLoweringError::unsupported("target has no capability metadata"))?;
        Ok(verum_vbc::target::TargetCapabilities {
            max_atomic_bits: info.max_atomic_bits,
            pointer_bits: machine.get_target_data().get_pointer_byte_size(None) * 8,
            features: info.features,
        })
    }
}
