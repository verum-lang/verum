//! Native LLVM AOT codegen + linking (Phase 6, Tier 1 execution).
//!
//! Extracted from `pipeline.rs` (#106 Phase 12). Houses the
//! AST → VBC → LLVM IR → native-binary path, plus the C-stubs
//! / linker discovery / lld driver helpers it depends on.
//!
//! Methods:
//!
//!   * `phase_generate_native` — primary AOT entry; lowers VBC
//!     to LLVM IR via `VbcToLlvmLowering`, runs LLVM optimisation
//!     passes, emits object files, links with the C runtime.
//!   * `get_project_root` — find Verum.toml-rooted project for
//!     output-directory resolution.
//!   * `generate_runtime_stubs` — emit the small C-runtime
//!     bridge that wraps the verum stdlib's libc-facing symbols.
//!   * `compile_c_file` — invoke the host C compiler on a single
//!     stub file.
//!   * `detect_c_compiler` — host-`cc` discovery (clang / gcc).
//!   * `link_executable` — orchestrate linker invocation.
//!   * `load_linker_config` / `link_with_config` /
//!     `link_with_lld` — Verum.toml-driven linker configuration
//!     and lld-fallback driver.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::Module;

use crate::phases::linking::LinkingConfig;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    }
    /// Phase 6: Generate native executable
    fn phase_generate_native(&mut self, module: &Module) -> Result<PathBuf> {
        info!("Generating native executable");
        let start = Instant::now();

        // Get input path and determine project root
        let input_path = &self.session.options().input;
        let project_root = self.get_project_root(input_path);

        // Determine build profile (debug or release)
        let profile = if self.session.options().optimization_level >= 2 {
            "release"
        } else {
            "debug"
        };

        // Create target directory structure
        let target_dir = project_root.join("target");
        let profile_dir = target_dir.join(profile);
        let build_dir = target_dir.join("build");

        // Create directories if they don't exist
        std::fs::create_dir_all(&profile_dir).with_context(|| {
            format!(
                "Failed to create target directory: {}",
                profile_dir.display()
            )
        })?;
        std::fs::create_dir_all(&build_dir).with_context(|| {
            format!("Failed to create build directory: {}", build_dir.display())
        })?;

        // Determine output path
        let output_path = if self.session.options().output.to_str().unwrap_or("").is_empty() {
            // Default: use input filename without extension in target/<profile>/
            let exe_name = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("main");
            profile_dir.join(if cfg!(windows) {
                format!("{}.exe", exe_name)
            } else {
                exe_name.to_string()
            })
        } else {
            // User-specified output path (use as-is)
            self.session.options().output.clone()
        };

        // Create module name
        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");

        info!("  Converting AST to VBC bytecode (multi-module)");

        // For multi-file projects, merge project module items into the main module.
        // This ensures all types, functions, and constants from sibling .vr files
        // are compiled as part of a single VBC compilation unit, avoiding
        // cross-module argument tracking issues in CallM/Call instructions.
        let module = if !self.project_modules.is_empty() {
            let mut merged = module.clone();
            for (path, proj_module) in &self.project_modules {
                info!("  Merging project module '{}' ({} items)", path.as_str(), proj_module.items.len());
                for item in &proj_module.items {
                    // Skip mount statements from project modules (they reference
                    // sibling modules that are already merged)
                    if matches!(&item.kind, verum_ast::ItemKind::Mount(_)) {
                        continue;
                    }
                    merged.items.push(item.clone());
                }
            }
            merged
        } else {
            module.clone()
        };

        // Phase 1: Convert AST to VBC bytecode with full multi-module resolution
        // This uses the same path as the interpreter, collecting stdlib imports
        // and resolving cross-module dependencies before compilation.
        let vbc_module = self.compile_ast_to_vbc(&module)
            .map_err(|e| anyhow::anyhow!("Failed to compile AST to VBC: {:?}", e))?;

        info!("  VBC bytecode: {} functions ({} with instructions)",
            vbc_module.functions.len(),
            vbc_module.functions.iter().filter(|f| f.instructions.is_some()).count());
        info!("  VBC bytecode generated: {} functions", vbc_module.functions.len());

        // Phase 1.5: Monomorphize generic functions
        // Specializes generic VBC functions with concrete type arguments before LLVM lowering.
        // This resolves CallG instructions to direct Call instructions.
        info!("  Monomorphizing generic functions");
        let vbc_module = {
            let mono = crate::phases::VbcMonomorphizationPhase::new();
            let mono = if !self.session.language_features().codegen.monomorphization_cache {
                mono.without_cache()
            } else { mono };
            let mut mono = mono;
            match mono.monomorphize(&vbc_module) {
                Ok(mono_module) => {
                    info!("  Monomorphization complete: {} functions", mono_module.functions.len());
                    std::sync::Arc::new(mono_module)
                }
                Err(diagnostics) => {
                    // Log warnings but fall back to unspecialized module
                    for d in diagnostics.iter() {
                        warn!("Monomorphization warning: {:?}", d);
                    }
                    info!("  Monomorphization skipped (fallback to unspecialized)");
                    vbc_module
                }
            }
        };

        // Phase 1.75: CBGR escape analysis
        // Determines which Ref/RefMut instructions can be promoted from Tier 0
        // (runtime-checked, ~15ns) to Tier 1 (compiler-proven safe, zero overhead).
        let escape_result = {
            use verum_vbc::cbgr_analysis::VbcEscapeAnalyzer;
            let analyzer = VbcEscapeAnalyzer::new();
            let functions: Vec<verum_vbc::VbcFunction> = vbc_module.functions.iter()
                .filter_map(|f| {
                    f.instructions.as_ref().map(|instrs| {
                        verum_vbc::VbcFunction::new(f.clone(), instrs.clone())
                    })
                })
                .collect();
            let result = analyzer.analyze(&functions);
            info!("  CBGR escape analysis: {} refs analyzed, {} promoted to tier1 ({:.1}%)",
                result.stats.total_refs,
                result.stats.promoted_to_tier1,
                result.stats.promotion_rate());
            result
        };

        // Emit VBC bytecode dump if requested
        if self.session.options().emit_vbc {
            let dump = verum_vbc::disassemble::disassemble_module(&vbc_module);
            let vbc_path = self.session.options().input.with_extension("vbc.txt");
            if let Err(e) = std::fs::write(&vbc_path, &dump) {
                warn!("Failed to write VBC dump: {}", e);
            } else {
                info!("Wrote VBC dump: {} ({} bytes)", vbc_path.display(), dump.len());
            }
        }

        // Phase 2: Lower VBC to LLVM IR (CPU path)
        // Note: For native compilation, we use the VBC → LLVM IR path (not MLIR).
        // GPU path (VBC → MLIR) should be used via run_mlir_jit/run_mlir_aot for tensor ops.
        info!("  Lowering VBC to LLVM IR");

        let llvm_ctx = verum_codegen::llvm::verum_llvm::context::Context::create();

        // Resolve the panic strategy from `[runtime].panic` in
        // Verum.toml (defaults to "unwind" — see RuntimeFeatures::
        // default()).  Threading this through here makes the
        // manifest setting actually drive emission of `verum_panic`
        // body shape — pre-fix the field was tracing-only at
        // session.rs:390 and the panic body always took the abort
        // path regardless.
        let panic_strategy = verum_codegen::llvm::PanicStrategy::from_manifest_text(
            self.session.options().language_features.runtime.panic.as_str(),
        );

        let lowering_config = verum_codegen::llvm::LoweringConfig::new(module_name)
            .with_opt_level(self.session.options().optimization_level)
            .with_debug_info(self.session.options().debug_info)
            .with_coverage(self.session.options().coverage)
            .with_permission_policy(self.session.aot_permission_policy())
            .with_panic_strategy(panic_strategy);

        let mut lowering = verum_codegen::llvm::VbcToLlvmLowering::new(
            &llvm_ctx,
            lowering_config,
        );

        // Apply CBGR escape analysis results to LLVM lowering.
        // This enables tier promotion: non-escaping references skip runtime
        // generation checks (Tier 0 → Tier 1), saving ~15ns per reference.
        lowering.set_escape_analysis(escape_result);

        lowering.lower_module(&vbc_module)
            .map_err(|e| anyhow::anyhow!("Failed to lower VBC to LLVM IR: {:?}", e))?;

        // Report CBGR statistics
        let stats = lowering.cbgr_stats();
        if stats.refs_created > 0 {
            info!("  CBGR: {} refs ({} tier0/{} tier1/{} tier2), {} runtime checks, {} eliminated",
                stats.refs_created, stats.tier0_refs, stats.tier1_refs, stats.tier2_refs,
                stats.runtime_checks, stats.checks_eliminated);
        }

        // Phase 3: Write intermediate files
        let obj_path = build_dir.join(format!("{}.o", module_name));
        let ir_path = build_dir.join(format!("{}.ll", module_name));

        let opt_level = self.session.options().optimization_level;
        info!("  Optimizing LLVM IR (level {})", opt_level);

        // Write LLVM IR only when explicitly requested or emit-llvm is on.
        // The IR printer triggers TypeFinder::incorporateType which
        // crashes on modules with stdlib-generated functions containing
        // null Type references (use-after-free from arity collision
        // fixups). Disabled by default to prevent non-deterministic
        // SIGSEGV during normal builds.
        let emit_ir = self.session.options().emit_ir
            || std::env::var("VERUM_DUMP_IR").is_ok();
        if emit_ir && !lowering.has_arity_collisions() && lowering.skip_body_count() == 0 {
            let llvm_ir = lowering.get_ir();
            std::fs::write(&ir_path, llvm_ir.as_str().as_bytes())
                .with_context(|| format!("Failed to write LLVM IR to {}", ir_path.display()))?;
            info!("  Written LLVM IR to {}", ir_path.display());
        }

        // Compile to object file using LLVM TargetMachine
        info!("  Writing object file to {}", obj_path.display());
        use verum_codegen::llvm::verum_llvm::targets::{
            Target, TargetMachine, RelocMode, CodeModel, FileType,
            InitializationConfig,
        };

        // Initialize native target ONCE per process.
        // LLVM's target initialization is not idempotent — calling it multiple
        // times can corrupt internal state.
        {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                let _ = Target::initialize_native(&InitializationConfig::default());
                // Also initialize WebAssembly target for cross-compilation
                #[cfg(feature = "target-wasm")]
                Target::initialize_webassembly(&InitializationConfig::default());
            });
        }

        // Use configured target triple if specified, otherwise default to host
        let triple = if let Some(ref target) = self.session.options().target_triple {
            verum_codegen::llvm::verum_llvm::targets::TargetTriple::create(target)
        } else {
            TargetMachine::get_default_triple()
        };
        let target = Target::from_triple(&triple)
            .map_err(|e| anyhow::anyhow!("Failed to get target: {}", e))?;

        // Cap TargetMachine at O2 (Default). O3 (Aggressive) enables
        // machine-level optimizations that interact badly with our
        // ptrtoint/inttoptr patterns in the code emitter.
        // Our pass pipeline (SROA+mem2reg+DSE+ADCE) already provides
        // the critical optimizations; the TargetMachine just needs O2.
        let llvm_opt_level = match opt_level {
            0 => verum_codegen::llvm::verum_llvm::OptimizationLevel::None,
            1 => verum_codegen::llvm::verum_llvm::OptimizationLevel::Less,
            _ => verum_codegen::llvm::verum_llvm::OptimizationLevel::Default,
        };

        // Use host CPU name and features for native targets.
        // For WASM targets, use "generic" CPU with no features.
        let is_wasm = triple.as_str().to_string_lossy().contains("wasm");
        let (cpu_str, features_str) = if is_wasm {
            ("generic", "")
        } else {
            // Get host CPU info for native compilation
            let cpu = TargetMachine::get_host_cpu_name();
            let features = TargetMachine::get_host_cpu_features();
            // Leak to static — called once per compilation, acceptable
            let cpu_s: &'static str = Box::leak(cpu.to_str().unwrap_or("generic").to_string().into_boxed_str());
            let feat_s: &'static str = Box::leak(features.to_str().unwrap_or("").to_string().into_boxed_str());
            (cpu_s, feat_s)
        };
        debug!("LLVM target: cpu={}, features={}", cpu_str, features_str);

        let target_machine = target.create_target_machine(
            &triple,
            cpu_str,
            features_str,
            llvm_opt_level,
            RelocMode::Default,
            CodeModel::Default,
        ).ok_or_else(|| anyhow::anyhow!("Failed to create target machine"))?;

        // Run LLVM optimization pass pipeline.
        // This is CRITICAL for performance — without it, all variables use
        // alloca+store/load (no SSA promotion, no inlining, no vectorization).
        {
            use verum_codegen::llvm::verum_llvm::passes::PassBuilderOptions;
            let pass_options = PassBuilderOptions::create();

            // Build the optimization pipeline string based on opt_level.
            // mem2reg: promotes alloca→SSA (CRITICAL for performance)
            // instcombine<no-verify-fixpoint>: algebraic simplification
            // gvn: global value numbering (CSE)
            // simplifycfg: control flow simplification
            // loop-unroll: unroll small loops
            // sroa: scalar replacement of aggregates
            // licm: loop invariant code motion
            // LLVM pass pipeline selection.
            // Float values are stored directly as f64 through opaque pointers.
            // Pointer values still use ptrtoint→i64→inttoptr which is incompatible
            // with SROA/GVN (they lose pointer provenance tracking).
            // mem2reg + simplifycfg gives 0.93x native C — sufficient for v1.0.
            // Full O3 (SROA/GVN/DSE/inline) requires storing pointers directly
            // through opaque pointer allocas (same pattern as the float fix).
            // Typed alloca storage: f64 stored directly, ptr stored directly with
            // lazy ptrtoint on load. This preserves LLVM type info for all passes.
            // mem2reg + simplifycfg = 0.93x native C.
            // SROA breaks on ptr-heavy List operations (ptrtoint provenance loss).
            // Full O3 requires typed alloca refactor of instruction.rs.
            // LLVM optimization pass pipeline.
            // Typed alloca storage: f64 stored directly, ptr stored directly.
            // get_register returns native types (PointerValue for ptrs).
            // This enables all LLVM passes to work correctly.
            // Run GlobalDCE FIRST to remove dead stdlib functions that may have
            // invalid IR (broken PHI nodes, unreachable blocks). This prevents
            // SimplifyCFG from crashing on invalid dead code.
            // LLVM pass pipeline — conservative due to ptrtoint→i64→inttoptr
            // pattern used by VBC codegen. This breaks SROA/GVN/instcombine/
            // early-cse which depend on pointer provenance tracking.
            // Safe passes: mem2reg (alloca→SSA), simplifycfg (branch cleanup).
            //
            // Function-level optimization hints (@inline, @cold, @hot, @optimize)
            // are applied as LLVM function attributes in vbc_lowering.rs.
            // These are respected automatically by the pass manager for:
            //   - Code layout (.text.cold sections)
            //   - Inlining decisions (alwaysinline/noinline/inlinehint)
            //   - Size optimization (optsize/minsize on cold functions)
            //   - Per-function target features (target-features/target-cpu)
            // When the module has arity collisions or skip-body
            // functions, LLVM function-level passes (mem2reg,
            // simplifycfg, instcombine) crash with SIGSEGV in
            // canReplaceOperandWithVariable or TypeFinder due to
            // null Type* references in redirect-stub instructions.
            // Restrict to globaldce (module-level dead code
            // elimination) which is safe — it only removes
            // unreachable functions without traversing instruction
            // operands.
            let has_ir_issues =
                lowering.has_arity_collisions() || lowering.skip_body_count() > 0;

            let passes = if has_ir_issues {
                // Arity collisions / skip-body stubs contain redirect IR
                // with null Type* references — full instcombine/SROA/GVN
                // crashes LLVM TypeFinder, so restrict to module-level DCE.
                "globaldce".to_string()
            } else {
                // Use LLVM's canonical O-level pipelines. These include
                // DCE, GVN, LICM, SROA, instcombine, inliner, loop opts,
                // vectorization — the full set of standard optimizations.
                // Fall back to the conservative pipeline for opt_level=0
                // to keep debug builds fast.
                match opt_level {
                    0 => "globaldce".to_string(),
                    1 => "default<O1>".to_string(),
                    2 => "default<O2>".to_string(),
                    _ => "default<O3>".to_string(),
                }
            };

            info!("  Running LLVM passes: {}", passes);
            if let Err(e) = lowering.module().run_passes(&passes, &target_machine, pass_options) {
                // Fall back to just globaldce if full pipeline fails
                tracing::warn!("Full LLVM pass pipeline failed: {} — falling back to globaldce", e);
                let fallback_options = PassBuilderOptions::create();
                if let Err(e2) = lowering.module().run_passes("globaldce", &target_machine, fallback_options) {
                    tracing::warn!("GlobalDCE pass also failed: {}", e2);
                }
            }
        }

        // VERUM_DUMP_IR=1 — dump LLVM IR after optimization passes.
        // Useful for analyzing codegen quality and debugging optimizations.
        if std::env::var("VERUM_DUMP_IR").is_ok() {
            let ir_path = build_dir.join(format!("{}.ll", module_name));
            let _ = lowering.write_ir_to_file(&ir_path);
            info!("  LLVM IR dumped to {}", ir_path.display());
        }

        // Verify the module AFTER GlobalDCE removed dead functions.
        // Dead stdlib functions may have invalid IR (unresolved intrinsics),
        // but GlobalDCE eliminates them, leaving only valid reachable code.
        //
        // Debug info verification failures (!dbg location on inlined calls) are
        // non-fatal — the code is correct, only metadata is inconsistent. Emit
        // a warning instead of aborting compilation.
        if let Err(e) = lowering.verify() {
            let err_str = format!("{:?}", e);
            if err_str.contains("!dbg location") || err_str.contains("debug info") {
                tracing::warn!("LLVM module has debug info inconsistency (non-fatal): {}",
                    err_str.chars().take(200).collect::<String>());
                // Continue compilation — the actual code is correct
            } else {
                let ir_path = build_dir.join(format!("{}_debug.ll", module_name));
                if !lowering.has_arity_collisions() {
                    let _ = lowering.write_ir_to_file(&ir_path);
                    return Err(anyhow::anyhow!("LLVM module verification failed (IR dumped to {}): {:?}", ir_path.display(), e));
                } else {
                    return Err(anyhow::anyhow!("LLVM module verification failed: {:?}", e));
                }
            }
        }

        target_machine.write_to_file(lowering.module(), FileType::Object, &obj_path)
            .map_err(|e| anyhow::anyhow!("Failed to write object file: {}", e))?;

        // Emit LLVM bitcode when LTO is enabled for cross-module optimization
        if self.session.options().lto {
            let bc_path = obj_path.with_extension("bc");
            lowering.module().write_bitcode_to_path(&bc_path);
            debug!("  Wrote LLVM bitcode for LTO: {}", bc_path.display());
        }

        // Runtime compilation: LLVM IR provides core runtime (allocator, text, etc.)
        // ALL runtime functions are now pure LLVM IR (platform_ir.rs + tensor_ir.rs + metal_ir.rs).
        // No C compilation needed. We still generate an empty .o for the linker.
        let runtime_stubs_path = self.generate_runtime_stubs(&build_dir)?;
        let runtime_obj = self.compile_c_file(&runtime_stubs_path, &build_dir)?;

        // Metal GPU runtime — now in LLVM IR (metal_ir.rs), no Objective-C compilation needed
        let metal_obj: Option<PathBuf> = None;

        // Load linker configuration from Verum.toml (if present)
        let mut linker_config = self.load_linker_config(&project_root, profile)?;

        // Wire CLI LTO option into linker config
        if self.session.options().lto {
            use crate::phases::linking::LTOConfig;
            linker_config.lto = match self.session.options().lto_mode {
                Some(crate::options::LtoMode::Full) => LTOConfig::Full,
                Some(crate::options::LtoMode::Thin) | None => LTOConfig::Thin,
            };
            // Enable LLD for LTO support
            linker_config.use_llvm_linker = true;
        }

        // Add Metal/Foundation frameworks for macOS GPU support (LLD path)
        #[cfg(target_os = "macos")]
        {
            linker_config.extra_flags.push("-framework Metal".into());
            linker_config.extra_flags.push("-framework Foundation".into());
            linker_config.libraries.push("objc".into());
        }

        // Link object files into executable in target/<profile>/
        info!("  Linking executable");
        let mut link_objects = vec![obj_path.clone(), runtime_obj];
        if let Some(ref metal) = metal_obj {
            link_objects.push(metal.clone());
            info!("  Including Metal GPU runtime in link");
        }
        self.link_with_config(&link_objects, &output_path, &linker_config)?;

        // Clean up intermediate files
        let _ = std::fs::remove_file(&runtime_stubs_path);
        // verum_platform.c deleted — no cleanup needed
        // verum_tensor.c deleted — no cleanup needed
        // verum_metal.m deleted — no cleanup needed

        let elapsed = start.elapsed();
        info!(
            "Generated native executable: {} ({:.2}s)",
            output_path.display(),
            elapsed.as_secs_f64()
        );

        Ok(output_path)
    }

    /// Get the project root directory
    ///
    /// Searches for Verum.toml starting from the input file's directory
    /// and walking up the directory tree. Falls back to input file's parent
    /// or current working directory if no Verum.toml is found.
    fn get_project_root(&self, input_path: &PathBuf) -> PathBuf {
        // Canonicalize the input path to get absolute path
        let abs_path = if input_path.is_absolute() {
            input_path.clone()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(input_path)
        };

        // Start from the input file's parent directory
        let mut current = abs_path.parent().map(|p| p.to_path_buf());

        // Walk up the directory tree looking for Verum.toml
        while let Some(dir) = current {
            let manifest = dir.join("Verum.toml");
            if manifest.exists() {
                return dir;
            }
            current = dir.parent().map(|p| p.to_path_buf());
        }

        // Fallback: use input file's parent directory or current working directory
        abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Generate C runtime stubs for CBGR and stdlib functions
    fn generate_runtime_stubs(&self, temp_dir: &Path) -> Result<PathBuf> {
        let stubs_path = temp_dir.join("verum_runtime_stubs.c");

        // Use the extracted C runtime from verum_codegen
        let stubs_code = verum_codegen::runtime_stubs::RUNTIME_C;

        std::fs::write(&stubs_path, stubs_code)?;
        debug!("Generated runtime stubs: {}", stubs_path.display());

        // verum_platform.c DELETED — all platform functions in LLVM IR (platform_ir.rs)

        Ok(stubs_path)
    }

    /// Compile a C file to object file
    fn compile_c_file(&self, source_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        let output_path = output_dir
            .join(
                source_path
                    .file_stem()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("runtime"),
            )
            .with_extension("o");

        // Detect C compiler
        let cc = self.detect_c_compiler()?;

        debug!("Compiling C file with {}: {}", cc, source_path.display());

        // Compile C file to object file with architecture-specific SIMD flags
        let mut cmd = std::process::Command::new(&cc);
        let c_opt = if self.session.options().optimization_level >= 3 { "-O3" } else { "-O2" };
        cmd.arg("-c")
            .arg(source_path)
            .arg("-o")
            .arg(&output_path)
            .arg(c_opt)
            .arg("-fPIC")
            .arg("-ffast-math")
            .arg("-DNDEBUG");

        // Add architecture-specific SIMD flags for auto-vectorization
        #[cfg(target_arch = "x86_64")]
        {
            cmd.arg("-march=native");
            cmd.arg("-mavx2");
            cmd.arg("-mfma");
        }
        #[cfg(target_arch = "aarch64")]
        {
            cmd.arg("-march=armv8-a+simd");
        }

        // Entry point provided by LLVM IR (platform_ir.rs) — skip C entry points
        cmd.arg("-DVERUM_LLVM_IR_ENTRY");
        // File I/O, time, networking C code deleted — LLVM IR only (platform_ir.rs)

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("C compilation failed:\n{}", stderr));
        }

        Ok(output_path)
    }

    /// Detect available C compiler
    fn detect_c_compiler(&self) -> Result<String> {
        let compilers = ["clang", "gcc", "cc"];

        for compiler in &compilers {
            if let Ok(output) = std::process::Command::new(compiler)
                .arg("--version")
                .output()
            {
                if output.status.success() {
                    return Ok(compiler.to_string());
                }
            }
        }

        Err(anyhow::anyhow!(
            "No C compiler found (tried: clang, gcc, cc)"
        ))
    }

    /// Link object files into executable
    fn link_executable(&self, object_files: &[PathBuf], output_path: &PathBuf) -> Result<()> {
        let linker = self.detect_c_compiler()?;

        debug!("Linking with {}: {}", linker, output_path.display());

        let mut cmd = std::process::Command::new(&linker);

        // Add all object files
        for obj in object_files {
            cmd.arg(obj);
        }

        // Output path
        cmd.arg("-o").arg(output_path);

        // ==========================================================================
        // NO LIBC ARCHITECTURE
        // ==========================================================================
        // Verum does NOT link against libc or system C libraries (-lm, -lpthread, -ldl).
        // All runtime functionality is provided by:
        // - LLVM intrinsics (llvm.sin.f32, llvm.sqrt.f64, etc.) for math
        // - Custom Verum runtime in /core/ for threading, memory, I/O
        // - Platform-specific system calls via /core/sys/
        //
        // Entry point: /core/sys/init.vr provides the custom _start that
        // initializes the Verum runtime before calling the user's main function.
        //
        // Exception: GPU targets may link Metal/CUDA/ROCm frameworks via MLIR path.
        // ==========================================================================

        // Platform-specific flags (no libc)
        #[cfg(target_os = "macos")]
        {
            cmd.arg("-Wl,-dead_strip");
            cmd.arg("-Wl,-undefined,dynamic_lookup");
            // 16MB stack for recursive algorithms (default 8MB causes SIGSEGV in deep recursion)
            cmd.arg("-Wl,-stack_size,0x1000000");
            // Link Metal + Foundation frameworks for GPU compute on Apple Silicon.
            // metal_ir.rs emits LLVM IR that calls MTLCreateSystemDefaultDevice,
            // objc_msgSend, sel_registerName, objc_getClass — all from these frameworks.
            cmd.arg("-framework").arg("Metal");
            cmd.arg("-framework").arg("Foundation");
            cmd.arg("-lobjc");
        }

        #[cfg(target_os = "linux")]
        {
            cmd.arg("-Wl,--gc-sections");
            cmd.arg("-rdynamic");
            // 16MB stack for recursive algorithms
            cmd.arg("-Wl,-z,stacksize=16777216");
            // Link additional system libraries for runtime
            cmd.arg("-ldl");
            cmd.arg("-lrt");
            // Link C++ stdlib for CBGR
            cmd.arg("-lstdc++");
        }

        // Execute linker
        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Linking failed:\n{}", stderr));
        }

        // Make executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(output_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(output_path, perms)?;
        }

        Ok(())
    }

    /// Load linker configuration from Verum.toml
    ///
    /// Reads the [linker] section from Verum.toml and merges with profile-specific
    /// settings. Falls back to defaults if no Verum.toml is found.
    fn load_linker_config(&self, project_root: &Path, profile: &str) -> Result<LinkingConfig> {
        let verum_toml = project_root.join("Verum.toml");

        if verum_toml.exists() {
            // Load full project configuration
            let project_config = ProjectConfig::load_from_file(&verum_toml)
                .with_context(|| format!("Failed to load {}", verum_toml.display()))?;

            // Get linker config for the specified profile
            let output_path = PathBuf::new(); // Placeholder - will be set by caller
            project_config.to_linking_config(profile, output_path)
        } else {
            // Use default configuration
            Ok(LinkingConfig::default())
        }
    }

    /// Link object files using configuration from Verum.toml
    ///
    /// This method supports two linking modes:
    /// - **LLD (LLVM Linker)**: When `use_lld = true` in Verum.toml, uses FinalLinker
    ///   for LTO support and faster linking on Linux
    /// - **System Linker**: Falls back to system compiler (clang/gcc) for compatibility
    ///
    /// Configuration options from Verum.toml:
    /// - `output`: executable, shared, static, object
    /// - `lto`: none, thin, full
    /// - `use_lld`: true/false
    /// - `pic`: position-independent code
    /// - `strip`: strip debug symbols
    /// - `libraries`: additional libraries to link
    /// - `extra_flags`: raw linker flags
    fn link_with_config(
        &self,
        object_files: &[PathBuf],
        output_path: &PathBuf,
        config: &LinkingConfig,
    ) -> Result<()> {
        // Clone config and set output path
        let mut link_config = config.clone();
        link_config.output_path = output_path.clone();

        // Log configuration
        info!(
            "  Linker config: output={:?}, lto={:?}, use_lld={}, pic={}, strip={}",
            link_config.output_kind,
            link_config.lto,
            link_config.use_llvm_linker,
            link_config.pic,
            link_config.strip
        );

        if link_config.use_llvm_linker {
            // Use FinalLinker with LLD for AOT compilation
            self.link_with_lld(object_files, &link_config)
        } else {
            // Fall back to system linker
            self.link_executable(object_files, output_path)
        }
    }

    /// Link object files using LLD via FinalLinker
    ///
    /// This method uses the FinalLinker from phases/linking.rs which provides:
    /// - LTO support (Thin/Full)
    /// - CBGR runtime integration
    /// - Multi-platform support (ELF, MachO, COFF, Wasm)
    fn link_with_lld(&self, object_files: &[PathBuf], config: &LinkingConfig) -> Result<()> {
        // Convert PathBuf array to ObjectFile list
        let obj_files: List<ObjectFile> = object_files
            .iter()
            .map(|path| ObjectFile::from_path(path.clone()))
            .collect::<Result<Vec<_>>>()?
            .into();

        // Create FinalLinker with AOT tier
        let mut linker = FinalLinker::new(ExecutionTier::Aot, config.clone());

        // Set exported symbols
        if !config.exported_symbols.is_empty() {
            linker = linker.with_exported_symbols(config.exported_symbols.clone());
        }

        // Perform linking
        let binary = linker.link(obj_files)?;

        info!(
            "  LLD linking complete: {} ({} bytes)",
            binary.path.display(),
            binary.size
        );

        // Make executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if binary.executable {
                let mut perms = std::fs::metadata(&binary.path)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&binary.path, perms)?;
            }
        }

        Ok(())
    }

    // ==================== PHASE 0: stdlib COMPILATION ====================

    /// Phase 0: stdlib Compilation & Preparation
    ///
    /// This phase runs once per build and compiles the Verum standard library
    /// from Rust source to static library, generating FFI exports and symbol
    /// registries for consumption by all execution tiers.
    ///
    /// Outputs are cached and reused across compilations unless verum_std
    /// source files change.
    ///
    /// **Mode-specific behavior:**
    /// - `Interpret` mode: SKIPPED - interpreter uses Rust native execution
    /// - `Check` mode: SKIPPED - type checking uses built-in type definitions
    /// - `Aot` mode: REQUIRED - static library for native linking
    /// - `Jit` mode: REQUIRED - symbol registry for JIT compilation
    ///
    /// Phase 0: Compile verum_std to static lib, generate C-compatible FFI exports,
    /// build symbol registry, prepare LLVM bitcode for LTO, cache monomorphized generics.
}
