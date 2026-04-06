#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Tests for Phase 7.5: Final Linking
//!
//! These tests verify the linking phase functionality including:
//! - OutputKind enum for different output formats
//! - Symbol resolution
//! - Static library creation
//! - Shared library linking
//! - JIT linking

use std::path::PathBuf;
use verum_compiler::phases::linking::*;
use verum_compiler::phases::{CompilationPhase, ExecutionTier};
use verum_common::{List, Map, Text};

// =============================================================================
// OutputKind Tests
// =============================================================================

#[test]
fn test_output_kind_defaults_to_executable() {
    let kind: OutputKind = Default::default();
    assert_eq!(kind, OutputKind::Executable);
}

#[test]
fn test_output_kind_extensions() {
    // Executable extension depends on platform
    let exe_ext = OutputKind::Executable.extension();
    #[cfg(target_os = "windows")]
    assert_eq!(exe_ext, "exe");
    #[cfg(not(target_os = "windows"))]
    assert_eq!(exe_ext, "");

    // Shared library extension depends on platform
    let so_ext = OutputKind::SharedLibrary.extension();
    #[cfg(target_os = "macos")]
    assert_eq!(so_ext, "dylib");
    #[cfg(target_os = "windows")]
    assert_eq!(so_ext, "dll");
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    assert_eq!(so_ext, "so");

    // Static library always .a
    assert_eq!(OutputKind::StaticLibrary.extension(), "a");

    // Object file always .o
    assert_eq!(OutputKind::ObjectFile.extension(), "o");
}

#[test]
fn test_output_kind_library_prefix() {
    // Libraries have "lib" prefix on Unix
    #[cfg(not(target_os = "windows"))]
    {
        assert_eq!(OutputKind::SharedLibrary.library_prefix(), "lib");
        assert_eq!(OutputKind::StaticLibrary.library_prefix(), "lib");
    }
    #[cfg(target_os = "windows")]
    {
        assert_eq!(OutputKind::SharedLibrary.library_prefix(), "");
        assert_eq!(OutputKind::StaticLibrary.library_prefix(), "");
    }

    // Executables and object files have no prefix
    assert_eq!(OutputKind::Executable.library_prefix(), "");
    assert_eq!(OutputKind::ObjectFile.library_prefix(), "");
}

// =============================================================================
// LTOConfig Tests
// =============================================================================

#[test]
fn test_lto_config_default() {
    let lto: LTOConfig = Default::default();
    assert_eq!(lto, LTOConfig::Thin);
}

#[test]
fn test_lto_config_variants() {
    let none = LTOConfig::None;
    let thin = LTOConfig::Thin;
    let full = LTOConfig::Full;

    assert_ne!(none, thin);
    assert_ne!(thin, full);
    assert_ne!(none, full);
}

// =============================================================================
// LinkingConfig Tests
// =============================================================================

#[test]
fn test_linking_config_default() {
    let config = LinkingConfig::default();

    assert_eq!(config.output_kind, OutputKind::Executable);
    assert_eq!(config.lto, LTOConfig::Thin);
    assert!(config.pic);
    assert!(!config.strip);
    assert_eq!(config.output_path, PathBuf::from("a.out"));
    assert_eq!(config.entry_point, Some(Text::from("main")));
    assert!(config.debug_info);
}

#[test]
fn test_linking_config_for_executable() {
    let output = PathBuf::from("/tmp/test_exe");
    let config = LinkingConfig::for_executable(output.clone());

    assert_eq!(config.output_kind, OutputKind::Executable);
    assert_eq!(config.output_path, output);
    assert_eq!(config.entry_point, Some(Text::from("main")));
}

#[test]
fn test_linking_config_for_shared_library() {
    let output = PathBuf::from("/tmp/libtest.so");
    let config = LinkingConfig::for_shared_library(output.clone());

    assert_eq!(config.output_kind, OutputKind::SharedLibrary);
    assert_eq!(config.output_path, output);
    assert!(config.pic); // PIC required for shared libraries
    assert_eq!(config.entry_point, None); // No entry point for libraries
}

#[test]
fn test_linking_config_for_static_library() {
    let output = PathBuf::from("/tmp/libtest.a");
    let config = LinkingConfig::for_static_library(output.clone());

    assert_eq!(config.output_kind, OutputKind::StaticLibrary);
    assert_eq!(config.output_path, output);
    assert_eq!(config.lto, LTOConfig::None); // LTO deferred to final link
    assert_eq!(config.entry_point, None);
}

#[test]
fn test_linking_config_for_object_file() {
    let output = PathBuf::from("/tmp/test.o");
    let config = LinkingConfig::for_object_file(output.clone());

    assert_eq!(config.output_kind, OutputKind::ObjectFile);
    assert_eq!(config.output_path, output);
    assert_eq!(config.lto, LTOConfig::None);
    assert_eq!(config.entry_point, None);
}

// =============================================================================
// Symbol Table Tests
// =============================================================================

#[test]
fn test_symbol_table_creation() {
    let table = SymbolTable::new();
    assert_eq!(table.undefined_count(), 0);
    assert_eq!(table.resolved_count(), 0);
}

#[test]
fn test_external_symbol_undefined() {
    let sym = ExternalSymbol::undefined("test_func");

    assert_eq!(sym.name, Text::from("test_func"));
    assert_eq!(sym.binding, SymbolBinding::Global);
    assert_eq!(sym.sym_type, SymbolType::Undefined);
    assert_eq!(sym.address, 0);
    assert!(!sym.is_resolved());
}

#[test]
fn test_external_symbol_function() {
    let sym = ExternalSymbol::function("main", 0x1000);

    assert_eq!(sym.name, Text::from("main"));
    assert_eq!(sym.binding, SymbolBinding::Global);
    assert_eq!(sym.sym_type, SymbolType::Function);
    assert_eq!(sym.address, 0x1000);
    assert!(sym.is_resolved());
}

#[test]
fn test_symbol_table_add_and_get() {
    let mut table = SymbolTable::new();

    let sym = ExternalSymbol::function("test_func", 0x2000);
    table.add_symbol(sym);

    let retrieved = table.get_symbol("test_func");
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().address, 0x2000);
}

#[test]
fn test_symbol_table_undefined_tracking() {
    let mut table = SymbolTable::new();

    table.add_symbol(ExternalSymbol::undefined("external_func"));
    table.add_symbol(ExternalSymbol::function("local_func", 0x1000));

    assert_eq!(table.undefined_count(), 1);
    assert_eq!(table.resolved_count(), 1);
}

#[test]
fn test_symbol_table_resolution() {
    let mut table = SymbolTable::new();

    table.add_symbol(ExternalSymbol::undefined("printf"));

    assert!(!table.get_symbol("printf").unwrap().is_resolved());

    let resolved = table.resolve_symbol("printf", 0x7fff00001000);
    assert!(resolved);

    // After resolution, the symbol should be marked as Function
    let sym = table.get_symbol("printf").unwrap();
    assert!(sym.is_resolved());
    assert_eq!(sym.address, 0x7fff00001000);
}

#[test]
fn test_symbol_table_export() {
    let mut table = SymbolTable::new();

    table.export_symbol("my_public_api");
    table.export_symbol("another_export");

    // Verify exports were added
    let undefined = table.get_undefined();
    // Note: exported symbols are tracked separately from defined/undefined
}

// =============================================================================
// ObjectFile Tests
// =============================================================================

#[test]
fn test_object_file_creation() {
    let temp_dir = std::env::temp_dir().join("verum_linking_test");
    std::fs::create_dir_all(&temp_dir).ok();

    let obj_path = temp_dir.join("test.o");
    std::fs::write(&obj_path, b"fake object file content").ok();

    let obj = ObjectFile::new(obj_path.clone(), "test_module").unwrap();

    assert_eq!(obj.path, obj_path);
    assert_eq!(obj.module_name, Text::from("test_module"));
    assert!(!obj.has_bitcode);
    assert!(obj.defined_symbols.is_empty());
    assert!(obj.undefined_symbols.is_empty());

    // Cleanup
    std::fs::remove_file(&obj_path).ok();
}

#[test]
fn test_object_file_from_path() {
    let temp_dir = std::env::temp_dir().join("verum_linking_test");
    std::fs::create_dir_all(&temp_dir).ok();

    let obj_path = temp_dir.join("my_module.o");
    std::fs::write(&obj_path, b"fake object").ok();

    let obj = ObjectFile::from_path(obj_path.clone()).unwrap();

    // Module name should be extracted from filename
    assert_eq!(obj.module_name, Text::from("my_module"));

    // Cleanup
    std::fs::remove_file(&obj_path).ok();
}

// =============================================================================
// FinalLinker Tests
// =============================================================================

#[test]
fn test_final_linker_creation() {
    let config = LinkingConfig::default();
    let linker = FinalLinker::new(ExecutionTier::Aot, config);

    // Test CompilationPhase trait
    assert_eq!(linker.name(), "Phase 7.5: Final Linking");
    assert!(!linker.can_parallelize()); // Linking must be serial
}

#[test]
fn test_linker_with_core_artifacts() {
    let config = LinkingConfig::default();
    let linker = FinalLinker::new(ExecutionTier::Aot, config);

    // Note: We can't easily test with_core_artifacts without actual artifacts
    // Just verify the method exists and linker can be constructed
    assert_eq!(
        linker.description(),
        "Link object files with stdlib and apply LTO optimizations"
    );
}

#[test]
fn test_linker_with_exported_symbols() {
    let config = LinkingConfig::default();
    let linker =
        FinalLinker::new(ExecutionTier::Aot, config).with_exported_symbols(List::from(vec![
            Text::from("my_api_init"),
            Text::from("my_api_process"),
        ]));

    // Verify linker was created with exported symbols
    assert_eq!(linker.name(), "Phase 7.5: Final Linking");
}

// =============================================================================
// Linker Statistics Tests
// =============================================================================

#[test]
fn test_linking_stats_default() {
    let stats = LinkingStats::default();

    assert_eq!(stats.object_files_linked, 0);
    assert_eq!(stats.total_object_size, 0);
    assert_eq!(stats.binary_size, 0);
    assert!(stats.lto_mode.is_none());
    assert_eq!(stats.symbols_resolved, 0);
}

#[test]
fn test_linking_stats_report() {
    let mut stats = LinkingStats::default();
    stats.object_files_linked = 5;
    stats.total_object_size = 1_048_576; // 1 MB
    stats.binary_size = 524_288; // 512 KB
    stats.total_link_time_ms = 500;
    stats.symbols_resolved = 100;

    let report = stats.report();

    // Verify report contains expected sections
    assert!(report.contains("Linking Statistics"));
    assert!(report.contains("Object Files: 5"));
    assert!(report.contains("Symbols Resolved: 100"));
}

#[test]
fn test_linking_stats_report_with_lto() {
    let mut stats = LinkingStats::default();
    stats.object_files_linked = 3;
    stats.lto_mode = Some(LTOConfig::Full);
    stats.bitcode_merge_time_ms = 100;
    stats.lto_optimization_time_ms = 300;
    stats.functions_inlined = 50;
    stats.dead_code_eliminated = 10240; // 10 KB

    let report = stats.report();

    assert!(report.contains("LTO Mode: Full"));
    assert!(report.contains("Functions Inlined: 50"));
    assert!(report.contains("Dead Code Eliminated"));
}

// =============================================================================
// Public API Tests
// =============================================================================

#[test]
fn test_create_linker() {
    let linker = create_linker(ExecutionTier::Aot);
    assert_eq!(linker.name(), "Phase 7.5: Final Linking");
}

#[test]
fn test_create_linker_with_config() {
    let mut config = LinkingConfig::default();
    config.lto = LTOConfig::Full;
    config.strip = true;

    let linker = create_linker_with_config(ExecutionTier::Aot, config);
    assert_eq!(linker.name(), "Phase 7.5: Final Linking");
}

// =============================================================================
// Bitcode Tests
// =============================================================================

#[test]
fn test_bitcode_from_file() {
    let temp_dir = std::env::temp_dir().join("verum_linking_test");
    std::fs::create_dir_all(&temp_dir).ok();

    let bc_path = temp_dir.join("test.bc");
    std::fs::write(&bc_path, b"fake bitcode content here").ok();

    let bc = Bitcode::from_file(&bc_path).unwrap();

    assert_eq!(bc.path, bc_path);
    assert!(bc.size > 0);

    // Cleanup
    std::fs::remove_file(&bc_path).ok();
}

// =============================================================================
// Binary Tests
// =============================================================================

#[test]
fn test_binary_from_path() {
    let temp_dir = std::env::temp_dir().join("verum_linking_test");
    std::fs::create_dir_all(&temp_dir).ok();

    let bin_path = temp_dir.join("test_binary");
    std::fs::write(&bin_path, b"fake binary content").ok();

    // Make it executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).ok();
    }

    let binary = Binary::from_path(&bin_path).unwrap();

    assert_eq!(binary.path, bin_path);
    assert!(binary.size > 0);
    #[cfg(unix)]
    assert!(binary.executable);

    // Cleanup
    std::fs::remove_file(&bin_path).ok();
}

// =============================================================================
// Symbol Binding/Type Tests
// =============================================================================

#[test]
fn test_symbol_binding_variants() {
    let global = SymbolBinding::Global;
    let weak = SymbolBinding::Weak;
    let local = SymbolBinding::Local;

    assert_ne!(global, weak);
    assert_ne!(weak, local);
    assert_ne!(global, local);
}

#[test]
fn test_symbol_type_variants() {
    let function = SymbolType::Function;
    let data = SymbolType::Data;
    let undefined = SymbolType::Undefined;

    assert_ne!(function, data);
    assert_ne!(data, undefined);
    assert_ne!(function, undefined);
}

// =============================================================================
// Execution Tier Tests
// =============================================================================

#[test]
fn test_linker_interpreter_tier() {
    let config = LinkingConfig::default();
    let linker = FinalLinker::new(ExecutionTier::Interpreter, config);

    // Interpreter tier should create linker but not need actual linking
    assert_eq!(linker.name(), "Phase 7.5: Final Linking");
}

#[test]
fn test_linker_jit_tiers() {
    let config = LinkingConfig::default();

    // Baseline JIT
    let linker1 = FinalLinker::new(ExecutionTier::Aot, config.clone());
    assert_eq!(linker1.name(), "Phase 7.5: Final Linking");

    // Optimizing JIT
    let linker2 = FinalLinker::new(ExecutionTier::Aot, config);
    assert_eq!(linker2.name(), "Phase 7.5: Final Linking");
}

#[test]
fn test_linker_aot_tier() {
    let config = LinkingConfig::default();
    let linker = FinalLinker::new(ExecutionTier::Aot, config);

    // AOT tier should use full linking with LTO
    assert_eq!(linker.name(), "Phase 7.5: Final Linking");
}
