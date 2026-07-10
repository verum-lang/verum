//! Native LLVM AOT codegen + linking (Phase 6, Tier 1 execution).
//!

//! Extracted from `pipeline.rs` (#106 Phase 12). Houses the
//! AST → VBC → LLVM IR → native-binary path, plus the C-stubs
//! / linker discovery / lld driver helpers it depends on.
//!

//! Methods:
//!

//!  * `phase_generate_native` — primary AOT entry; lowers VBC
//!  to LLVM IR via `VbcToLlvmLowering`, runs LLVM optimisation
//!  passes, emits object files, links with the C runtime.
//!  * `get_project_root` — find Verum.toml-rooted project for
//!  output-directory resolution.
//!  * `generate_runtime_stubs` — emit the small C-runtime
//!  bridge that wraps the verum stdlib's libc-facing symbols.
//!  * `compile_c_file` — invoke the host C compiler on a single
//!  stub file.
//!  * `detect_c_compiler` — host-`cc` discovery (clang / gcc).
//!  * `link_executable` — orchestrate linker invocation.
//!  * `load_linker_config` / `link_with_config` /
//!  `link_with_lld` — Verum.toml-driven linker configuration
//!  and lld-fallback driver.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_common::List;

use crate::linker_config::ProjectConfig;
use crate::phases::ExecutionTier;
use crate::phases::linking::{FinalLinker, LinkingConfig, ObjectFile};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Phase 6: Generate native executable.
    pub(super) fn phase_generate_native(&mut self, module: &Module) -> Result<PathBuf> {
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
        let output_path = if self
            .session
            .options()
            .output
            .to_str()
            .unwrap_or("")
            .is_empty()
        {
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
                info!(
                    "  Merging project module '{}' ({} items)",
                    path.as_str(),
                    proj_module.items.len()
                );
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
        let vbc_module = self
            .compile_ast_to_vbc(&module)
            .map_err(|e| anyhow::anyhow!("Failed to compile AST to VBC: {:?}", e))?;

        info!(
            "  VBC bytecode: {} functions ({} with instructions)",
            vbc_module.functions.len(),
            vbc_module
                .functions
                .iter()
                .filter(|f| f.instructions.is_some())
                .count()
        );
        info!(
            "  VBC bytecode generated: {} functions",
            vbc_module.functions.len()
        );

        // Phase 1.5: Monomorphize generic functions
        // Specializes generic VBC functions with concrete type arguments before LLVM lowering.
        // This resolves CallG instructions to direct Call instructions.
        info!("  Monomorphizing generic functions");
        let vbc_module = {
            let mono = crate::phases::VbcMonomorphizationPhase::new();
            let mono = if !self
                .session
                .language_features()
                .codegen
                .monomorphization_cache
            {
                mono.without_cache()
            } else {
                mono
            };
            let mut mono = mono;
            match mono.monomorphize(&vbc_module) {
                Ok(mono_module) => {
                    info!(
                        "  Monomorphization complete: {} functions",
                        mono_module.functions.len()
                    );
                    std::sync::Arc::new(mono_module)
                }
                Err(diagnostics) => {
                    // #45: VERUM_STRICT_MONO=1 (CI) turns the silent
                    // fallback-to-unspecialized into a hard error so
                    // monomorphization drift cannot pass a green build.
                    if std::env::var("VERUM_STRICT_MONO").is_ok() {
                        return Err(anyhow::anyhow!(
                            "monomorphization failed under VERUM_STRICT_MONO ({} diagnostics): {:?}",
                            diagnostics.len(),
                            diagnostics
                        ));
                    }
                    // Log warnings but fall back to unspecialized module
                    for d in diagnostics.iter() {
                        warn!("Monomorphization warning: {:?}", d);
                    }
                    info!("  Monomorphization skipped (fallback to unspecialized)");
                    vbc_module
                }
            }
        };

        // Emit VBC bytecode dump if requested
        if self.session.options().emit_vbc {
            let dump = verum_vbc::disassemble::disassemble_module(&vbc_module);
            let vbc_path = self.session.options().input.with_extension("vbc.txt");
            if let Err(e) = std::fs::write(&vbc_path, &dump) {
                warn!("Failed to write VBC dump: {}", e);
            } else {
                info!(
                    "Wrote VBC dump: {} ({} bytes)",
                    vbc_path.display(),
                    dump.len()
                );
            }
        }

        // ════════════════════════════════════════════════════════════
        // AOT-STDLIB-NATIVE-CACHE-1 — whole-module native-object cache.
        //
        // The LLVM leg (`lower_optimize_and_emit_object`: VBC→IR
        // lowering of the merged stdlib+user module, pass pipeline,
        // machine codegen) is a multi-second fixed baseline per
        // compile, dominated by the ~9K precompiled-stdlib functions
        // that are byte-identical across runs.  Key the final object
        // by the compile's deterministic INPUTS — input source bytes
        // + on-disk stdlib tree + compiler-binary stamp (covers the
        // embedded archive/metadata) + a fingerprint of every
        // out-of-module knob the lowering reads — and skip the whole
        // leg on a hit.  (NOT keyed on the VBC module: its
        // serialization is per-process-HashMap-order nondeterministic
        // — measured identical length / different bytes every run —
        // so a VBC key never hits.)  See
        // `pipeline/aot_object_cache.rs` for the boundary rationale
        // (whole-module vs stdlib-split) and the on-disk contract.
        //
        // Everything the key needs (pass string, target identity) is
        // hoisted ABOVE the lowering; the hoisted values are then
        // passed into `lower_optimize_and_emit_object` unchanged so
        // the miss path compiles exactly as before.
        //
        // Kill switch: VERUM_NO_OBJECT_CACHE=1.  LTO and the IR-dump
        // diagnostic paths bypass the cache (they need the live LLVM
        // module), as do compiles with merged project modules /
        // external cogs (their file set isn't part of the key) and
        // archive-less builds (stdlib compiled from source).
        // ════════════════════════════════════════════════════════════
        let opt_level = self.session.options().optimization_level;
        let obj_path = build_dir.join(format!("{}.o", module_name));

        // LLVM pass-pipeline selection (hoisted ahead of lowering so
        // the chosen pipeline participates in the object-cache key;
        // the selection depends only on env vars + opt level).
        //
        // **Tiered pipeline** (#94 / #91 perf roadmap).
        //
        // Pre-fix: any skip-body or arity-collided function dropped
        // the entire module to `globaldce`-only — defeating EVERY
        // perf pass including `always-inline`.  Hello-world hits
        // this because the stdlib has 9 `swap` overloads (one of
        // which hits `InvalidRegister(2)` during VBC→LLVM lowering),
        // silently degrading every binary's perf to "linked but
        // unoptimised".
        //
        // Post-fix tiering:
        //
        //   * **Clean modules** (no skip-body, no arity collisions):
        //     run the full canonical `default<O2>` / `default<O3>`
        //     pipeline.  Inliner, SROA, GVN, instcombine, loop opts,
        //     vectorization — everything LLVM offers.
        //
        //   * **Modules with IR issues**: run a curated SAFE subset
        //     anchored on `always-inline` + `globaldce`.  The
        //     `always-inline` pass respects `alwaysinline` function
        //     attributes (#92's `verum_text_get_ptr` etc.) and
        //     critically does NOT traverse into the bodies of
        //     non-alwaysinline functions — so the broken IR in
        //     skip-body stubs can't trip it.  `globaldce` then
        //     deletes any function that became unreachable after
        //     inlining, including the stubs themselves.
        //
        // Result: even on dirty modules the user's hot Text helpers
        // get inlined into call sites, cutting hello-world's
        // `verum_main` from `bl _verum_text_get_ptr; bl _verum_internal_puts`
        // to inline `_verum_internal_puts(adrp+add)`.
        // VERUM_FORCE_FULL_O2=1 — diagnostic override; force the
        // full O2/O3 pipeline regardless of IR-issue status.  Used
        // to investigate codegen-level type-conflict bugs (arity-
        // collided functions with multi-typed register slots that
        // crash SROA / SimplifyCFG / GVN).  See #91 follow-up.
        let force_full = std::env::var("VERUM_FORCE_FULL_O2").is_ok();
        // **VERUM_PASSES_OVERRIDE** — diagnostic env-var that
        // overrides the pass pipeline string entirely.  Used to
        // bisect which specific pass in `default<O2>` triggers
        // the bitcode-write / loop-pass SIGBUS on arity-collided
        // modules (#98).
        let override_passes = std::env::var("VERUM_PASSES_OVERRIDE").ok();
        // **Task #24 mitigation** (LLVM PassManager SIGBUS at
        // `appendLoopsToWorklist` / `IntervalMap::deleteNode`):
        // `default<O[1-3]>` triggers LLVM's loop-pass pipeline,
        // which walks Verum-emitted IR's loop-info graph and
        // SIGBUSes on metadata corruption / dangling Use chains
        // (see the investigation log in the pass-run block below —
        // bitcode + text roundtrip both crash, root cause is
        // metadata-corruption-at-write-time not pass-side bug).
        //
        // The crash is deterministic for every non-trivial
        // module; even `fn main() { let x: Int = 42; }` reproduces
        // it.  Until the metadata-corruption root cause lands,
        // **default to `always-inline,globaldce`** — the same
        // safe pipeline that `has_ir_issues` branch uses.  Users
        // who need aggressive optimisation can opt in explicitly
        // via `VERUM_FORCE_FULL_O2=1` (force the broken
        // `default<O2>`) or `VERUM_PASSES_OVERRIDE=<pipeline>`
        // (custom pipeline).
        //
        // Validated 2026-05-18: `verum build --release` on a
        // minimal program produces a 17.1 KB binary linking only
        // libSystem.B.dylib (no-libc invariant per CLAUDE.md);
        // running the binary exits 0.
        let passes = if let Some(p) = override_passes {
            p
        } else if force_full {
            match opt_level {
                0 => "globaldce".to_string(),
                1 => "default<O1>".to_string(),
                2 => "default<O2>".to_string(),
                _ => "default<O3>".to_string(),
            }
        } else {
            // Anchor on always-inline so our `verum_text_get_ptr` /
            // `verum_is_text_object` (and any future `@inline` Verum
            // attribute) get honoured even when full O2 isn't safe.
            // Order matters: always-inline first → globaldce last
            // so the stubs that become unreachable post-inlining
            // get removed.
            "always-inline,globaldce".to_string()
        };

        // Target identity (triple / CPU / features) — hoisted ahead
        // of lowering: these strings are pure host/options queries
        // (no LLVM target-registry init required) and participate in
        // the object-cache key.  The TargetMachine itself is still
        // created inside `lower_optimize_and_emit_object` (miss path
        // only), after `Target::initialize_all`.
        //
        // **CPU/features dispatch** (#82 cross-compile correctness).
        //
        // For native (host == target) builds: use host CPU + features
        // for maximum performance.
        //
        // For cross builds: use "generic" CPU + empty features.
        // Pre-fix the host CPU name (e.g. `apple-m3`) was forwarded
        // into every TargetMachine regardless of arch, which LLVM
        // rejects with `'apple-m3' is not a recognized processor for
        // this target` followed by `LLVM ERROR: 64-bit code requested
        // on a subtarget that doesn't support it!` — every cross-arch
        // build crashed.
        //
        // Detection: compare target architecture to host architecture
        // via the TargetMachine's default-host triple.
        let (triple_str, cpu_str, features_str) = {
            use verum_codegen::llvm::verum_llvm::targets::TargetMachine;
            let triple_str: String = match self.session.options().target_triple {
                Some(ref target) => target.as_str().to_string(),
                None => TargetMachine::get_default_triple()
                    .as_str()
                    .to_string_lossy()
                    .into_owned(),
            };
            let is_wasm = triple_str.contains("wasm");
            let host_triple_for_cpu = TargetMachine::get_default_triple()
                .as_str()
                .to_string_lossy()
                .into_owned();
            // Heuristic: if target arch differs from host, host CPU is
            // not applicable.  Use simple substring match on arch prefix.
            let host_arch = host_triple_for_cpu.split('-').next().unwrap_or("");
            let target_arch = triple_str.split('-').next().unwrap_or("");
            let cross_arch = !target_arch.is_empty() && target_arch != host_arch;

            let (cpu_str, features_str): (&'static str, &'static str) = if is_wasm || cross_arch {
                ("generic", "")
            } else {
                // Native build — use host CPU info for max perf.
                let cpu = TargetMachine::get_host_cpu_name();
                let features = TargetMachine::get_host_cpu_features();
                // Leak to static — called once per compilation, acceptable
                let cpu_s: &'static str = Box::leak(
                    cpu.to_str()
                        .unwrap_or("generic")
                        .to_string()
                        .into_boxed_str(),
                );
                let feat_s: &'static str =
                    Box::leak(features.to_str().unwrap_or("").to_string().into_boxed_str());
                (cpu_s, feat_s)
            };
            debug!(
                "LLVM target: triple={}, cpu={}, features={} (cross_arch={})",
                triple_str, cpu_str, features_str, cross_arch
            );
            (triple_str, cpu_str, features_str)
        };

        // Fingerprint of every out-of-module input the LLVM leg
        // reads: compiler binary stamp (also covers the EMBEDDED
        // stdlib archive + metadata), lowering-config knobs (the
        // same session fields `lower_optimize_and_emit_object`
        // resolves), monomorphization-cache flag (can reorder the
        // emitted module), permission policy (Debug over BTreeSets —
        // deterministic), target identity, pass string, and the
        // module-mutating diagnostic env (orphan sweep).
        let object_cache = {
            let lf = &self.session.options().language_features;
            let fingerprint = format!(
                "exe={exe}|module={module}|opt={opt}|dbg={dbg}|cov={cov}|panic={panic}|\
                 tco={tco}|vect={vect}|inline={inline}|futures={fut}|nurseries={nur}|\
                 awt={awt}|tss={tss}|mono_cache={mc}|perm={perm:?}|triple={triple}|\
                 cpu={cpu}|features={features}|passes={passes}|orphan_sweep={osweep}",
                exe = super::aot_object_cache::compiler_stamp(),
                module = module_name,
                opt = opt_level,
                dbg = self.session.options().debug_info,
                cov = self.session.options().coverage,
                panic = lf.runtime.panic.as_str(),
                tco = lf.codegen.tail_call_optimization,
                vect = lf.codegen.vectorize,
                inline = lf.codegen.inline_depth,
                fut = lf.runtime.futures,
                nur = lf.runtime.nurseries,
                awt = lf.runtime.async_worker_threads,
                tss = lf.runtime.task_stack_size,
                mc = lf.codegen.monomorphization_cache,
                perm = self.session.aot_permission_policy(),
                triple = triple_str,
                cpu = cpu_str,
                features = features_str,
                passes = passes,
                osweep = std::env::var_os("VERUM_SKIP_ORPHAN_SWEEP").is_some(),
            );
            if super::aot_object_cache::bypassed(
                self.session.options().lto,
                // Any non-Binary emit mode needs the LIVE LLVM module /
                // target machine to produce its artifact (.ll/.bc/.s/.o
                // next to the input) — a cache hit would skip the whole
                // lowering leg and silently emit nothing (#45: the
                // --emit-llvm flag was dead).
                self.session.options().emit_ir
                    || self.session.options().emit_mode != crate::options::EmitMode::Binary,
            ) {
                None
            } else if !self.project_modules.is_empty() {
                // Merged project modules / external cogs are inputs
                // this key does not hash — correctness first.
                debug!(
                    "aot-object-cache: bypass — {} merged project module(s) not part of the key",
                    self.project_modules.len()
                );
                None
            } else if crate::embedded_stdlib_vbc::get_runtime_archive().is_none() {
                // Archive-less build: stdlib function bodies come from
                // the legacy source-compile path, whose input set is
                // broader than the stdlib tree stamp — bypass.
                debug!("aot-object-cache: bypass — no embedded stdlib archive");
                None
            } else {
                super::aot_object_cache::AotObjectCache::prepare(
                    &target_dir,
                    &self.session.options().input,
                    &fingerprint,
                )
            }
        };

        let needs_metal = match object_cache
            .as_ref()
            .and_then(|cache| cache.try_fetch(&obj_path))
        {
            Some(hit) => hit.needs_metal,
            None => {
                let needs_metal = self.lower_optimize_and_emit_object(
                    &vbc_module,
                    module_name,
                    &build_dir,
                    &obj_path,
                    cpu_str,
                    features_str,
                    &passes,
                )?;
                if let Some(cache) = object_cache.as_ref() {
                    cache.store(&obj_path, needs_metal);
                }
                needs_metal
            }
        };

        // #45: terminal emit modes (asm / obj) REPLACE the executable as
        // the final pipeline artifact — short-circuit before runtime-stub
        // generation and linking (the EmitMode::meta().is_terminal
        // contract; LlvmIr/Bitcode emit *alongside* the binary and fall
        // through).  The .s was written next to the input inside
        // `lower_optimize_and_emit_object`; the .o is copied from the
        // build dir here.
        let emit_mode = self.session.options().emit_mode;
        if emit_mode.meta().is_terminal && emit_mode != crate::options::EmitMode::Binary {
            let artifact = self
                .session
                .options()
                .input
                .with_extension(emit_mode.meta().extension);
            if emit_mode == crate::options::EmitMode::Object {
                std::fs::copy(&obj_path, &artifact).with_context(|| {
                    format!("Failed to copy object file to {}", artifact.display())
                })?;
            }
            info!(
                "Generated {} artifact: {} ({:.2}s)",
                emit_mode.meta().name,
                artifact.display(),
                start.elapsed().as_secs_f64()
            );
            return Ok(artifact);
        }

        // Runtime compilation: LLVM IR provides core runtime (allocator, text, etc.)
        // ALL runtime functions are now pure LLVM IR (platform_ir.rs + tensor_ir.rs + metal_ir.rs).
        // No C compilation needed. We still generate an empty .o for the linker.
        let runtime_stubs_path = self.generate_runtime_stubs(&build_dir, module_name)?;
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

        // Wire CLI strip flags into linker config. Closes the
        // inert-defense pattern: pre-fix the CLI's `--strip` and
        // `--strip-debug` flags populated
        // `CompilerOptions.strip_symbols` / `.strip_debug` but
        // `link_with_config` only read from `LinkingConfig`, so
        // CLI strip overrides were silently dropped when no
        // `Verum.toml` was present.
        apply_strip_options_to_linker_config(self.session.options(), &mut linker_config);

        // Wire Windows subsystem (console / GUI) into linker config.
        // The CLI (`--windows-subsystem`) and manifest
        // (`[build].windows_subsystem`) get resolved into
        // `options.windows_subsystem` upstream by `verum_cli::commands::build`.
        // Here we apply it to the no-libc Windows configuration so the
        // produced .exe carries `/SUBSYSTEM:WINDOWS` (GUI) instead of
        // the default `/SUBSYSTEM:CONSOLE`. Ignored on non-Windows
        // targets — the linker config's flags table is unchanged when
        // `for_platform` returns a non-Windows variant.
        if let Some(ref subsystem_flag) = self.session.options().windows_subsystem {
            // Only apply when we have a no-libc config AND it's a
            // Windows config (otherwise the flag would be a silent
            // no-op). Detect via the platform field on the existing
            // config, falling back to inspection of the target triple.
            let is_windows_target = match &linker_config.no_libc_config {
                Some(cfg) => matches!(cfg.platform, verum_codegen::link::Platform::Windows),
                None => self
                    .session
                    .options()
                    .target_triple
                    .as_ref()
                    .map(|t| {
                        t.as_str().contains("windows")
                            || t.as_str().contains("msvc")
                            || t.as_str().contains("mingw")
                    })
                    .unwrap_or(false),
            };
            if is_windows_target {
                use verum_codegen::link::NoLibcConfig;
                linker_config.no_libc_config = Some(NoLibcConfig::windows_with_subsystem(
                    subsystem_flag.as_str(),
                ));
            }
        }

        // Add Metal/Foundation frameworks for macOS GPU support (LLD path).
        // **Target-aware** (#80): driven by configured target triple via
        // the canonical `triple_str_is_darwin` helper, not host
        // `#[cfg(target_os)]`.  Pre-fix this gate used host-cfg,
        // which silently dropped the Metal frameworks when cross-compiling
        // to macOS from Linux.
        let triple_for_frameworks: String = match self.session.options().target_triple.clone() {
            Some(t) => t.as_str().to_string(),
            None => {
                use verum_codegen::llvm::verum_llvm::targets::TargetMachine;
                TargetMachine::get_default_triple()
                    .as_str()
                    .to_string_lossy()
                    .into_owned()
            }
        };
        if verum_codegen::llvm::target_triple::triple_str_is_darwin(&triple_for_frameworks)
            && needs_metal
        {
            // Only link Metal/Foundation/objc when the program
            // actually uses GPU (post-globaldce probe — see #100).
            // Programs with no `@device(GPU)` and no tensor ops
            // above the GPU threshold get a leaner binary
            // (libSystem-only `LC_LOAD_DYLIB` table).
            linker_config.extra_flags.push("-framework Metal".into());
            linker_config
                .extra_flags
                .push("-framework Foundation".into());
            linker_config.libraries.push("objc".into());
        }

        // Link object files into executable in target/<profile>/
        info!("  Linking executable");
        let mut link_objects = vec![obj_path.clone(), runtime_obj];
        if let Some(ref metal) = metal_obj {
            link_objects.push(metal.clone());
            info!("  Including Metal GPU runtime in link");
        }
        self.link_with_config(&link_objects, &output_path, &linker_config, needs_metal)?;

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

    /// The AOT LLVM leg: CBGR escape analysis → VBC→LLVM lowering →
    /// pass pipeline → verification → GPU-usage probe → object-file
    /// emission (+ LTO bitcode sidecar).  Extracted verbatim from
    /// `phase_generate_native` so the whole leg can be SKIPPED on an
    /// AOT object-cache hit (AOT-STDLIB-NATIVE-CACHE-1).
    ///

    /// Returns the post-globaldce `needs_metal` probe result (#100),
    /// the only lowering-derived fact the link step still needs.
    ///

    /// `cpu_str` / `features_str` / `passes` are hoisted by the
    /// caller (they participate in the object-cache key) and used
    /// here unchanged.
    #[allow(clippy::too_many_arguments)]
    fn lower_optimize_and_emit_object(
        &self,
        vbc_module: &std::sync::Arc<verum_vbc::module::VbcModule>,
        module_name: &str,
        build_dir: &Path,
        obj_path: &Path,
        cpu_str: &'static str,
        features_str: &'static str,
        passes: &str,
    ) -> Result<bool> {
        // Phase 1.75: CBGR escape analysis
        // Determines which Ref/RefMut instructions can be promoted from Tier 0
        // (runtime-checked, ~15ns) to Tier 1 (compiler-proven safe, zero overhead).
        let escape_result = {
            use verum_vbc::cbgr_analysis::VbcEscapeAnalyzer;
            let analyzer = VbcEscapeAnalyzer::new();
            let functions: Vec<verum_vbc::VbcFunction> = vbc_module
                .functions
                .iter()
                .filter_map(|f| {
                    f.instructions
                        .as_ref()
                        .map(|instrs| verum_vbc::VbcFunction::new(f.clone(), instrs.clone()))
                })
                .collect();
            let result = analyzer.analyze(&functions);
            info!(
                "  CBGR escape analysis: {} refs analyzed, {} promoted to tier1 ({:.1}%)",
                result.stats.total_refs,
                result.stats.promoted_to_tier1,
                result.stats.promotion_rate()
            );
            result
        };

        // Phase 2: Lower VBC to LLVM IR (CPU path)
        // Note: For native compilation, we use the VBC → LLVM IR path (not MLIR).
        // GPU path (VBC → MLIR) should be used via run_mlir_jit/run_mlir_aot for tensor ops.
        info!("  Lowering VBC to LLVM IR");

        let llvm_ctx = verum_codegen::llvm::verum_llvm::context::Context::create();

        // Resolve the panic strategy from `[runtime].panic` in
        // Verum.toml (defaults to "unwind" — see RuntimeFeatures::
        // default()). Threading this through here makes the
        // manifest setting actually drive emission of `verum_panic`
        // body shape — pre-fix the field was tracing-only at
        // session.rs:390 and the panic body always took the abort
        // path regardless.
        let panic_strategy = verum_codegen::llvm::PanicStrategy::from_manifest_text(
            self.session
                .options()
                .language_features
                .runtime
                .panic
                .as_str(),
        );

        // Resolve `[codegen].tail_call_optimization` from
        // Verum.toml. When false, every emitted function gets
        // `disable-tail-calls=true` so the LLVM backend skips TCO.
        // Pre-fix the manifest field was tracing-only at
        // session.rs:432; setting it false had zero effect on
        // generated code.
        let tail_call_optimization = self
            .session
            .options()
            .language_features
            .codegen
            .tail_call_optimization;

        // Resolve `[codegen].vectorize` from Verum.toml. When
        // false, every emitted function gets `no-loop-vectorize`
        // + `no-slp-vectorize` so LLVM's autovectorizer skips
        // those functions regardless of opt level. Sibling wire
        // to tail_call_optimization above; same pattern.
        let vectorize = self.session.options().language_features.codegen.vectorize;

        // Resolve `[codegen].inline_depth` from Verum.toml. Maps
        // to per-function `"inline-threshold"` LLVM string
        // attribute (threshold = inline_depth * 75; default 3 →
        // 225 = LLVM default, no IR emission). Pre-fix the
        // manifest field was tracing-only at session.rs:448;
        // setting it had zero effect on generated code. Closes
        // task #267.
        let inline_depth = self
            .session
            .options()
            .language_features
            .codegen
            .inline_depth;

        // Resolve manifest-driven runtime-bridge values
        // (architectural prerequisite #261). Each field flows
        // through to a `__verum_runtime_*` LLVM global at codegen
        // time and reaches stdlib code through the
        // `verum_get_runtime_*` getter functions. Default 0 keeps
        // the historical stdlib defaults (auto-detect via
        // num_cpus, platform stack size).
        let rt = &self.session.options().language_features.runtime;
        let runtime_bridge = verum_codegen::llvm::platform_ir::RuntimeBridgeValues {
            async_worker_threads: rt.async_worker_threads,
            task_stack_size: rt.task_stack_size,
        };

        let lowering_config = verum_codegen::llvm::LoweringConfig::new(module_name)
            .with_opt_level(self.session.options().optimization_level)
            .with_debug_info(self.session.options().debug_info)
            .with_coverage(self.session.options().coverage)
            .with_permission_policy(self.session.aot_permission_policy())
            .with_panic_strategy(panic_strategy)
            .with_tail_call_optimization(tail_call_optimization)
            .with_vectorize(vectorize)
            .with_runtime_bridge(runtime_bridge)
            .with_inline_depth(inline_depth)
            .with_futures_enabled(rt.futures)
            .with_nurseries_enabled(rt.nurseries);

        let mut lowering = verum_codegen::llvm::VbcToLlvmLowering::new(&llvm_ctx, lowering_config);

        // Apply CBGR escape analysis results to LLVM lowering.
        // This enables tier promotion: non-escaping references skip runtime
        // generation checks (Tier 0 → Tier 1), saving ~15ns per reference.
        lowering.set_escape_analysis(escape_result);

        lowering
            .lower_module(&vbc_module)
            .map_err(|e| anyhow::anyhow!("Failed to lower VBC to LLVM IR: {:?}", e))?;

        // Report CBGR statistics
        let stats = lowering.cbgr_stats();
        if stats.refs_created > 0 {
            info!(
                "  CBGR: {} refs ({} tier0/{} tier1/{} tier2), {} runtime checks, {} eliminated",
                stats.refs_created,
                stats.tier0_refs,
                stats.tier1_refs,
                stats.tier2_refs,
                stats.runtime_checks,
                stats.checks_eliminated
            );
        }

        // Phase 3: Write intermediate files
        let ir_path = build_dir.join(format!("{}.ll", module_name));

        let opt_level = self.session.options().optimization_level;
        info!("  Optimizing LLVM IR (level {})", opt_level);

        // Write LLVM IR only when explicitly requested or emit-llvm is on.
        // The IR printer triggers TypeFinder::incorporateType which
        // crashes on modules with stdlib-generated functions containing
        // null Type references (use-after-free from arity collision
        // fixups). Disabled by default to prevent non-deterministic
        // SIGSEGV during normal builds.
        let emit_ir = self.session.options().emit_ir || std::env::var("VERUM_DUMP_IR").is_ok();
        if emit_ir && !lowering.has_arity_collisions() && lowering.skip_body_count() == 0 {
            let llvm_ir = lowering.get_ir();
            std::fs::write(&ir_path, llvm_ir.as_str().as_bytes())
                .with_context(|| format!("Failed to write LLVM IR to {}", ir_path.display()))?;
            info!("  Written LLVM IR to {}", ir_path.display());
        }

        // Compile to object file using LLVM TargetMachine
        info!("  Writing object file to {}", obj_path.display());
        use verum_codegen::llvm::verum_llvm::targets::{
            CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
        };

        // **Initialize ALL LLVM targets** ONCE per process (#82
        // cross-compile correctness).  Pre-fix only the native target
        // was initialized, so `--target T` from x86_64 host failed
        // with "No available targets are compatible with triple T"
        // for any non-native T (every cross-compile broke).
        //
        // Per LLVM docs, `LLVM_InitializeAllTargets` registers all
        // architectures (x86, ARM, AArch64, RISC-V, WebAssembly,
        // PowerPC, etc.); per-OS support is automatic via the same
        // backends.  This adds ~5MB to the verum binary but unlocks
        // every supported target without per-target init switches.
        // LLVM's target initialization is not idempotent — calling it
        // multiple times can corrupt internal state, so we gate via
        // `Once`.
        {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                // Always initialize all targets — verum supports
                // cross-compilation by default (#82).  The cost is
                // upfront one-time and bounded; runtime impact is zero.
                Target::initialize_all(&InitializationConfig::default());
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

        // CPU/features dispatch (#82) — computed by the caller ahead
        // of the object-cache key derivation and passed in unchanged
        // (`cpu_str` / `features_str` parameters).

        let target_machine = target
            .create_target_machine(
                &triple,
                cpu_str,
                features_str,
                llvm_opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| anyhow::anyhow!("Failed to create target machine"))?;

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
            //  - Code layout (.text.cold sections)
            //  - Inlining decisions (alwaysinline/noinline/inlinehint)
            //  - Size optimization (optsize/minsize on cold functions)
            //  - Per-function target features (target-features/target-cpu)
            // When the module has arity collisions or skip-body
            // functions, LLVM function-level passes (mem2reg,
            // simplifycfg, instcombine) crash with SIGSEGV in
            // canReplaceOperandWithVariable or TypeFinder due to
            // null Type* references in redirect-stub instructions.
            // Restrict to globaldce (module-level dead code
            // elimination) which is safe — it only removes
            // unreachable functions without traversing instruction
            // operands.
            let has_ir_issues = lowering.has_arity_collisions() || lowering.skip_body_count() > 0;

            // #45: VERUM_STRICT_SIGNATURES=1 (CI) refuses to ship a
            // module with signature drift.  Arity-collided / skip-body
            // functions are forward-declared without bodies — the binary
            // links but the calls resolve to garbage at runtime, and the
            // degraded tiered pass pipeline masks the drift.  Strict mode
            // makes the drift a build error so CI catches the class.
            if has_ir_issues && std::env::var("VERUM_STRICT_SIGNATURES").is_ok() {
                return Err(anyhow::anyhow!(
                    "signature drift under VERUM_STRICT_SIGNATURES: \
                     arity_collisions={}, skip_body_functions={}",
                    lowering.has_arity_collisions(),
                    lowering.skip_body_count(),
                ));
            }

            // Pass-pipeline STRING selection is hoisted into
            // `phase_generate_native` (it participates in the
            // object-cache key) and arrives via the `passes`
            // parameter.  The tiering rationale lives with the
            // selection.

            if has_ir_issues {
                tracing::info!(
                    "  IR issues detected (arity_collisions={}, skip_body={}) — \
                     using tiered pipeline '{}' (#94)",
                    lowering.has_arity_collisions(),
                    lowering.skip_body_count(),
                    passes,
                );
            }

            info!("  Running LLVM passes: {}", passes);
            if std::env::var("VERUM_TRACE_PASSES").is_ok() {
                eprintln!(
                    "[verum-passes] running: {} (arity_collisions={}, skip_body={})",
                    passes,
                    lowering.has_arity_collisions(),
                    lowering.skip_body_count(),
                );
            }
            // **#98 diagnostic**: VERUM_SKIP_ORPHAN_SWEEP=1 disables the
            // module-wide orphan-block sweep to bisect whether the
            // sweep's block deletions are leaving stale LoopInfo /
            // analysis-manager state that crashes `run_passes`.
            let _skip_orphan_sweep = std::env::var_os("VERUM_SKIP_ORPHAN_SWEEP").is_some();

            // **Module-wide unreachable-block sweep** (#96).
            //
            // Runs after ALL codegen completes — VBC lowering, platform
            // IR (main wrapper), tensor IR, runtime helpers — so it
            // catches orphans from every emission site.  Per-function
            // cleanup inside `lower_function` already handles the
            // common case; this sweep mops up the residual orphans
            // from non-VBC paths (`no_main` from platform_ir, `zero_data`
            // from tensor_ir, etc.) that would otherwise crash
            // `SimplifyCFG::TryToSimplifyUncondBranchFromEmptyBlock`
            // with SIGBUS under `default<O2>`.
            let module_orphans = if _skip_orphan_sweep {
                0
            } else {
                lowering.sweep_module_orphan_blocks()
            };
            if module_orphans > 0 && std::env::var("VERUM_TRACE_PASSES").is_ok() {
                eprintln!(
                    "[verum-passes] module-wide orphan sweep deleted {} blocks",
                    module_orphans,
                );
            }

            // **Pre-pass IR dump** — capture IR before LLVM passes run.
            // Used for diagnosing crashes inside SimplifyCFG/SROA/GVN.
            if std::env::var("VERUM_DUMP_PRE_PASS").is_ok() {
                let pre_path = build_dir.join(format!("{}.pre-pass.ll", module_name));
                let _ = lowering.write_ir_to_file(&pre_path);
                eprintln!("[verum-passes] pre-pass IR -> {}", pre_path.display());
            }

            // **#98 bitcode round-trip** — fresh-parse the module
            // before `run_passes` to flush dirty analysis-manager
            // state.
            //
            // **Why this is needed**: LLVM's `Module::run_passes`
            // builds analyses (LoopInfo, DominatorTree, etc.) on
            // demand and caches them in a per-function
            // `AnalysisManager`.  Codegen-time mutations (orphan-
            // block deletion, function attribute updates, debug-info
            // touches) can leave the cache pointing to deleted /
            // moved IR — when `default<O2>` triggers loop-pass
            // pipelines, `LoopInfoBase::verify` walks the stale loop
            // graph and SIGBUSes / SIGSEGVs in
            // `appendLoopsToWorklist` → `IntervalMap::deleteNode`.
            //
            // **Why text-roundtrip works**: serialising to bitcode
            // and parsing back into a fresh `Module` rebuilds every
            // internal data structure from canonical IR — there's
            // no analysis cache to be stale.  Standalone `opt`
            // succeeds because it does exactly this on every input.
            //
            // **Cost**: one bitcode write + one parse per
            // compilation, both bounded by IR size.  For the
            // hello-world cross-smoke build this is ~50ms additional
            // time vs ~3s total compile — negligible.  In return we
            // unblock the full `default<O2>` pipeline for every
            // arity-collided / multi-typed-alloca module (the perf
            // wins from #96 commits f019b131 + 31e6587e land for
            // *every* AOT binary, not just clean modules).
            //
            // **VERUM_NO_BITCODE_ROUNDTRIP=1** opts out for
            // bisection / regression diagnostics.
            // **#98** — IR round-trip explored but doesn't fix the
            // SIGBUS in `default<O2>` for arity-collided modules.
            //
            // Findings (this session):
            //   * `module.write_bitcode_to_memory()` SIGSEGVs.
            //   * `module.print_to_string()` SIGSEGVs in the same
            //     run.  (Pre-`d5195189` the text dump succeeded —
            //     probable interaction with stdlib growth from
            //     concurrent agent commits.)
            //   * Standalone `opt --passes='default<O2>'` against a
            //     PREVIOUSLY-dumped text IR exits 0 — the textual
            //     form is structurally valid; the in-memory module
            //     has SOMETHING that LLVM's bitcode/text serializer
            //     AND loop-pass infrastructure both walk into.
            //   * Disabling per-function and module-wide orphan
            //     deletion (`VERUM_SKIP_ORPHAN_SWEEP=1`) does not
            //     change the crash — it's not orphan-deletion-induced.
            //
            // The actual root cause is metadata corruption AT a
            // level that even textual `printToString` can't traverse
            // safely — likely dangling Use chains, debug-info refs
            // to unlinked instructions, or a metadata cycle.
            //
            // Future investigation (deferred):
            //   1. Bisect via `--passes='argpromotion'`,
            //      `'instcombine'`, etc. one at a time to find
            //      which pass triggers IR walk that segfaults.
            //   2. Run `module.verify()` with
            //      `LLVMVerifierFailureAction::ReturnStatus` to
            //      enumerate every invalid instruction (the existing
            //      `verify()` short-circuits on first error).
            //   3. Check if `skip_body_count() > 0` modules emit
            //      partially-initialised bodies that confuse Use-list
            //      walks.
            if let Err(e) = lowering
                .module()
                .run_passes(&passes, &target_machine, pass_options)
            {
                if std::env::var("VERUM_TRACE_PASSES").is_ok() {
                    eprintln!("[verum-passes] FAILED: {} — falling back to globaldce", e);
                }
                // Fall back to just globaldce if full pipeline fails
                tracing::warn!(
                    "Full LLVM pass pipeline failed: {} — falling back to globaldce",
                    e
                );
                let fallback_options = PassBuilderOptions::create();
                if let Err(e2) =
                    lowering
                        .module()
                        .run_passes("globaldce", &target_machine, fallback_options)
                {
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
                tracing::warn!(
                    "LLVM module has debug info inconsistency (non-fatal): {}",
                    err_str.chars().take(200).collect::<String>()
                );
                // Continue compilation — the actual code is correct
            } else {
                let ir_path = build_dir.join(format!("{}_debug.ll", module_name));
                if !lowering.has_arity_collisions() {
                    let _ = lowering.write_ir_to_file(&ir_path);
                    return Err(anyhow::anyhow!(
                        "LLVM module verification failed (IR dumped to {}): {:?}",
                        ir_path.display(),
                        e
                    ));
                } else {
                    return Err(anyhow::anyhow!("LLVM module verification failed: {:?}", e));
                }
            }
        }

        // **#100 GPU-usage detection** — after `run_passes` (which
        // includes globaldce), check whether any Metal/GPU runtime
        // function survived elimination.  If the entire Metal IR was
        // DCE'd, the user's program doesn't use GPU and we can skip
        // the framework links downstream.
        //
        // Pre-fix every macOS Verum AOT binary unconditionally
        // dragged in `-framework Metal -framework Foundation -lobjc`
        // even when `nm <bin>` showed zero Metal symbols.  That's
        // pure dead weight in the binary's `LC_LOAD_DYLIB` table.
        //
        // Detection probe: `verum_metal_ensure_init` is the entry
        // point that every Metal-using path eventually calls
        // (`metal_ir.rs::emit_ensure_init`).  If it has a body, GPU
        // is used; if globaldce stripped it, GPU is unused.
        let needs_metal = lowering
            .module()
            .get_function("verum_metal_ensure_init")
            .map(|f| f.count_basic_blocks() > 0)
            .unwrap_or(false);
        if std::env::var("VERUM_TRACE_PASSES").is_ok() {
            eprintln!(
                "[verum-passes] gpu-usage probe: verum_metal_ensure_init has body = {}",
                needs_metal,
            );
        }

        target_machine
            .write_to_file(lowering.module(), FileType::Object, &obj_path)
            .map_err(|e| anyhow::anyhow!("Failed to write object file: {}", e))?;

        // #45: consume `options.emit_mode` — the CLI parsed
        // --emit-llvm/--emit-asm/--emit-bc into the enum but nothing in
        // the pipeline ever read it, so every mode was silently dead.
        // User-facing artifacts land NEXT TO THE INPUT (the emit_vbc
        // convention), not in the build dir.  Runs on the live-module
        // leg only; the object-cache bypass upstream guarantees non-
        // Binary modes always reach here.
        match self.session.options().emit_mode {
            crate::options::EmitMode::LlvmIr => {
                // Same file-writer path as the VERUM_DUMP_IR diagnostic
                // above — LLVMPrintModuleToFile tolerates skip-body /
                // arity-collision modules (unlike the get_ir() STRING
                // printer, which has a TypeFinder SIGSEGV history and
                // stays guarded).
                let ll_path = self.session.options().input.with_extension("ll");
                lowering
                    .write_ir_to_file(&ll_path)
                    .map_err(|e| anyhow::anyhow!("Failed to write LLVM IR: {}", e))?;
                info!("  Wrote LLVM IR: {}", ll_path.display());
            }
            crate::options::EmitMode::Bitcode => {
                let bc_path = self.session.options().input.with_extension("bc");
                if lowering.module().write_bitcode_to_path(&bc_path) {
                    info!("  Wrote LLVM bitcode: {}", bc_path.display());
                } else {
                    warn!("Failed to write LLVM bitcode to {}", bc_path.display());
                }
            }
            crate::options::EmitMode::Assembly => {
                let asm_path = self.session.options().input.with_extension("s");
                target_machine
                    .write_to_file(lowering.module(), FileType::Assembly, &asm_path)
                    .map_err(|e| anyhow::anyhow!("Failed to write assembly: {}", e))?;
                info!("  Wrote assembly: {}", asm_path.display());
            }
            // Object artifact is copied from the build dir by the caller's
            // terminal short-circuit; Binary emits nothing extra.
            crate::options::EmitMode::Object | crate::options::EmitMode::Binary => {}
        }

        // Emit LLVM bitcode when LTO is enabled for cross-module optimization
        if self.session.options().lto {
            let bc_path = obj_path.with_extension("bc");
            lowering.module().write_bitcode_to_path(&bc_path);
            debug!("  Wrote LLVM bitcode for LTO: {}", bc_path.display());
        }

        Ok(needs_metal)
    }

    /// Get the project root directory
    ///

    /// Searches for Verum.toml starting from the input file's directory
    /// and walking up the directory tree. Falls back to input file's parent
    /// or current working directory if no Verum.toml is found.
    pub(super) fn get_project_root(&self, input_path: &PathBuf) -> PathBuf {
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
    pub(super) fn generate_runtime_stubs(&self, temp_dir: &Path, tag: &str) -> Result<PathBuf> {
        // Per-compilation-unique stub path. The C source is a fixed
        // constant, but the `.c` is written here and `remove_file`d after
        // linking, and `compile_c_file` derives the `.o` name from this
        // stem. Under `verum test --aot`'s `par_iter`, a shared
        // `verum_runtime_stubs.c` / `.o` would be written, clang-compiled,
        // and deleted by many workers at once — racing into corrupt object
        // files or missing-source clang errors (sporadic link failures).
        // Tagging by the unit's module name gives every worker its own.
        let stubs_path = temp_dir.join(format!("verum_runtime_stubs_{}.c", tag));

        // Use the extracted C runtime from verum_codegen
        let stubs_code = verum_codegen::runtime_stubs::RUNTIME_C;

        std::fs::write(&stubs_path, stubs_code)?;
        debug!("Generated runtime stubs: {}", stubs_path.display());

        // verum_platform.c DELETED — all platform functions in LLVM IR (platform_ir.rs)

        Ok(stubs_path)
    }

    /// Compile a C file to object file
    pub(super) fn compile_c_file(&self, source_path: &Path, output_dir: &Path) -> Result<PathBuf> {
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
        let c_opt = if self.session.options().optimization_level >= 3 {
            "-O3"
        } else {
            "-O2"
        };
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
    pub(super) fn detect_c_compiler(&self) -> Result<String> {
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

    /// Link object files into executable.
    ///
    /// **Cross-compile gate**: when the configured target triple
    /// differs from the host, `clang` / `gcc` on the host cannot
    /// produce binaries for the target.  Pre-fix the linker step
    /// blindly invoked the host's `clang` regardless of target,
    /// producing `ld: unknown options: --gc-sections -z` errors when
    /// macOS-host's ld saw Linux-target's flags.
    ///
    /// Post-fix: detect the cross-compile case and skip linking with
    /// a clear status message.  The user gets the target-correct
    /// `.o` files in `target/build/` and can finish linking with the
    /// target's native toolchain (e.g. `aarch64-linux-gnu-gcc` on
    /// the target machine, or via Docker).
    ///
    /// True cross-linking (lld with `-target` flag, or vendored
    /// per-target ld) is a follow-up under #82 cross-compile matrix.
    pub(super) fn link_executable(
        &self,
        object_files: &[PathBuf],
        output_path: &PathBuf,
        needs_metal: bool,
    ) -> Result<()> {
        let linker = self.detect_c_compiler()?;

        debug!("Linking with {}: {}", linker, output_path.display());

        // Detect cross-compile case via target-triple comparison.
        let target_triple = self
            .session
            .options()
            .target_triple
            .clone()
            .map(|t| t.as_str().to_string());
        if let Some(tt) = &target_triple {
            use verum_codegen::llvm::target_triple::triple_str_os_family;
            use verum_codegen::llvm::verum_llvm::targets::TargetMachine;
            let host_triple_owned = TargetMachine::get_default_triple()
                .as_str()
                .to_string_lossy()
                .into_owned();
            let host_os = triple_str_os_family(&host_triple_owned);
            let target_os = triple_str_os_family(tt);
            // Architecture comparison via leading triple component.
            // `x86_64-apple-darwin` arch = "x86_64".
            let host_arch = host_triple_owned.split('-').next().unwrap_or("");
            let target_arch = tt.split('-').next().unwrap_or("");
            // Cross-compile is detected on EITHER OS or arch mismatch.
            // arm64-darwin host → x86_64-darwin target needs cross-link
            // even though OS matches (host's `-arch arm64` ld can't
            // consume x86_64 .o without an `-arch x86_64` invocation).
            let cross_os = host_os != target_os;
            let cross_arch = !target_arch.is_empty() && target_arch != host_arch;
            if cross_os || cross_arch {
                info!(
                    "  Cross-compile: target {} != host {} (cross_os={}, cross_arch={}) — skipping link step",
                    tt, host_triple_owned, cross_os, cross_arch
                );
                info!(
                    "  Object file(s) produced for target.  Link with target-native toolchain:"
                );
                for obj in object_files {
                    info!("    {}", obj.display());
                }
                // Don't error — the build successfully produced
                // target-correct object files, which is the meaningful
                // unit of work for cross-compile.  Skip linking.
                return Ok(());
            }
        }

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

        // **Target-aware linker flags** — driven by the session's
        // configured target triple, NOT host `#[cfg(target_os)]`.
        // Cross-compile correctness: a binary built on Linux for a
        // macOS target must get Darwin linker flags (-framework
        // Metal, etc.), not Linux flags (-Wl,--gc-sections).
        //
        // Session::options().target_triple is the configured target;
        // when None, falls back to the build host's triple via
        // `TargetMachine::get_default_triple()`.  We read it as a
        // string and dispatch via substring match — same pattern as
        // `target_triple::target_is_*(module)` in verum_codegen.
        // **Target-triple dispatch** via canonical helpers from
        // `verum_codegen::llvm::target_triple` — the single source of
        // truth for "is this triple X?" checks across the codebase.
        // (Avoids ad-hoc substring duplication; see #80 CI guard.)
        let target_triple_string: String = match self.session.options().target_triple.clone() {
            Some(t) => t.as_str().to_string(),
            None => {
                use verum_codegen::llvm::verum_llvm::targets::TargetMachine;
                TargetMachine::get_default_triple()
                    .as_str()
                    .to_string_lossy()
                    .into_owned()
            }
        };
        use verum_codegen::llvm::target_triple::{triple_str_is_darwin, triple_str_is_linux};
        let target_is_darwin = triple_str_is_darwin(&target_triple_string);
        let target_is_linux = triple_str_is_linux(&target_triple_string);

        if target_is_darwin {
            cmd.arg("-Wl,-dead_strip");
            cmd.arg("-Wl,-undefined,dynamic_lookup");
            // 16MB stack for recursive algorithms (default 8MB causes SIGSEGV in deep recursion)
            cmd.arg("-Wl,-stack_size,0x1000000");
            // Link Metal + Foundation frameworks ONLY when the program
            // actually uses GPU (post-globaldce probe — see #100).
            // Pre-fix every macOS Verum binary unconditionally pulled
            // these in even when the compiled module had zero Metal
            // symbols, bloating `LC_LOAD_DYLIB` for hello-world.
            if needs_metal {
                cmd.arg("-framework").arg("Metal");
                cmd.arg("-framework").arg("Foundation");
                cmd.arg("-lobjc");
            }
        }

        if target_is_linux {
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
    pub(super) fn load_linker_config(
        &self,
        project_root: &Path,
        profile: &str,
    ) -> Result<LinkingConfig> {
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
    ///  for LTO support and faster linking on Linux
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
    pub(super) fn link_with_config(
        &self,
        object_files: &[PathBuf],
        output_path: &PathBuf,
        config: &LinkingConfig,
        needs_metal: bool,
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
            // Use FinalLinker with LLD for AOT compilation.  Metal
            // framework links flow through `link_config.extra_flags`
            // (gated upstream by the same `needs_metal` probe), so
            // the LLD path doesn't need an explicit `needs_metal`
            // parameter here.
            self.link_with_lld(object_files, &link_config)
        } else {
            // Fall back to system linker — Metal framework gating is
            // explicit since this path doesn't consume `extra_flags`.
            self.link_executable(object_files, output_path, needs_metal)
        }
    }

    /// Link object files using LLD via FinalLinker
    ///

    /// This method uses the FinalLinker from phases/linking.rs which provides:
    /// - LTO support (Thin/Full)
    /// - CBGR runtime integration
    /// - Multi-platform support (ELF, MachO, COFF, Wasm)
    pub(super) fn link_with_lld(
        &self,
        object_files: &[PathBuf],
        config: &LinkingConfig,
    ) -> Result<()> {
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
}

/// Bridge `CompilerOptions.strip_symbols` / `.strip_debug` into the
/// linker's `LinkingConfig`. Manifest-set keys are OR'd with CLI
/// flags so a CLI override stacks on top of (rather than replaces)
/// the manifest's setting — matches the documented "CLI augments
/// manifest" semantic. `strip` is strictly stronger than
/// `strip_debug_only` (drops function names too); when both are
/// set, the linker honours `strip` and ignores `strip_debug_only`
/// at the cmd-line emission level.
fn apply_strip_options_to_linker_config(
    options: &crate::options::CompilerOptions,
    linker_config: &mut LinkingConfig,
) {
    linker_config.strip = linker_config.strip || options.strip_symbols;
    linker_config.strip_debug_only =
        linker_config.strip_debug_only || options.strip_debug;
}

#[cfg(test)]
mod strip_wiring_tests {
    use super::*;
    use crate::options::CompilerOptions;

    fn default_opts() -> CompilerOptions {
        CompilerOptions::default()
    }

    /// Pin: CLI `strip_symbols` flag flows into `LinkingConfig.strip`
    /// even when the manifest didn't set it. Closes the inert-defense
    /// pattern around the field.
    #[test]
    fn cli_strip_symbols_sets_linker_strip() {
        let mut linker = LinkingConfig::default();
        assert!(!linker.strip);
        let mut opts = default_opts();
        opts.strip_symbols = true;
        apply_strip_options_to_linker_config(&opts, &mut linker);
        assert!(linker.strip, "CLI strip_symbols=true must set linker.strip");
    }

    /// Pin: CLI `strip_debug` flag flows into
    /// `LinkingConfig.strip_debug_only`.
    #[test]
    fn cli_strip_debug_sets_linker_strip_debug_only() {
        let mut linker = LinkingConfig::default();
        assert!(!linker.strip_debug_only);
        let mut opts = default_opts();
        opts.strip_debug = true;
        apply_strip_options_to_linker_config(&opts, &mut linker);
        assert!(
            linker.strip_debug_only,
            "CLI strip_debug=true must set linker.strip_debug_only"
        );
    }

    /// Pin: CLI flag OR's with manifest setting — neither overrides
    /// the other. Mirrors the LTO wire-up's "CLI augments manifest"
    /// semantic.
    #[test]
    fn cli_or_with_manifest_strip_settings() {
        // Manifest already set strip; CLI doesn't override to false.
        let mut linker = LinkingConfig::default();
        linker.strip = true;
        let opts = default_opts(); // strip_symbols defaults to false
        apply_strip_options_to_linker_config(&opts, &mut linker);
        assert!(
            linker.strip,
            "manifest strip=true must persist when CLI doesn't override"
        );

        // Manifest didn't set strip; CLI doesn't either — both stay false.
        let mut linker = LinkingConfig::default();
        let opts = default_opts();
        apply_strip_options_to_linker_config(&opts, &mut linker);
        assert!(
            !linker.strip,
            "neither CLI nor manifest set strip — must remain false"
        );
    }
}

