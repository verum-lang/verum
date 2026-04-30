//! `@device(gpu)` annotation detection.
//!
//! Extracted from `pipeline.rs` (#106 Phase 22). A pair of free
//! helpers that scan a parsed module for `@device(gpu)` /
//! `@device(GPU)` attributes on functions, enabling automatic
//! GPU-compilation routing without an explicit `--gpu` flag.
//!
//! Surface:
//!
//!   * `detect_gpu_kernels` — module-level scan; checks both
//!     item-level attributes and module-decl-level attributes.
//!   * `has_device_gpu_attr` — single-attribute predicate
//!     (matches `@device(gpu)` and `@device(GPU)` shapes).

use verum_ast::{decl::ItemKind, Module};
use verum_common::{List, Maybe};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Scan a parsed module for `@device(gpu)` or `@device(GPU)` attributes on
    /// functions. Returns true if any GPU kernel annotation is found, enabling
    /// automatic GPU compilation without an explicit `--gpu` flag.
    ///
    /// This runs after Phase 2 (parsing) and before type checking so that the
    /// backend selection (CPU-only vs CPU+GPU) can be informed early.
    pub(super) fn detect_gpu_kernels(module: &Module) -> bool {
        for item in module.items.iter() {
            // Check item-level attributes (outer attributes on the item)
            if Self::has_device_gpu_attr(&item.attributes) {
                return true;
            }
            // Check function-level attributes (on the FunctionDecl itself)
            if let ItemKind::Function(ref func) = item.kind {
                if Self::has_device_gpu_attr(&func.attributes) {
                    return true;
                }
            }
            // Check functions inside impl blocks
            if let ItemKind::Impl(ref impl_decl) = item.kind {
                for impl_item in impl_decl.items.iter() {
                    if Self::has_device_gpu_attr(&impl_item.attributes) {
                        return true;
                    }
                    if let verum_ast::decl::ImplItemKind::Function(ref func) = impl_item.kind {
                        if Self::has_device_gpu_attr(&func.attributes) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if a list of attributes contains `@device(gpu)` or `@device(GPU)`.
    pub(super) fn has_device_gpu_attr(attrs: &List<verum_ast::Attribute>) -> bool {
        use verum_ast::expr::ExprKind;
        use verum_ast::ty::PathSegment;

        for attr in attrs.iter() {
            if attr.name.as_str() != "device" {
                continue;
            }
            // Check the first argument for "gpu" or "GPU" identifier
            if let Maybe::Some(ref args) = attr.args {
                if let Some(first_arg) = args.first() {
                    match &first_arg.kind {
                        // @device(gpu) — parsed as a path with single segment
                        ExprKind::Path(path) => {
                            if let Some(seg) = path.segments.first() {
                                if let PathSegment::Name(ident) = seg {
                                    let name = ident.name.as_str();
                                    if name.eq_ignore_ascii_case("gpu") {
                                        return true;
                                    }
                                }
                            }
                        }
                        // @device("gpu") — parsed as a string literal
                        ExprKind::Literal(lit) => {
                            if let verum_ast::literal::LiteralKind::Text(s) = &lit.kind {
                                if s.as_str().eq_ignore_ascii_case("gpu") {
                                    return true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }
}
