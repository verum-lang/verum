//! ShellRender derive macro implementation.
//!
//! Stub-level implementation: registers the derive in the registry so
//! `@derive(ShellRender)` doesn't fail with "unknown derive", but the
//! generated body is a single placeholder method that returns the empty
//! string.  Full body synthesis (`@flag`, `@positional`, type-aware
//! List/Maybe handling) lives behind a typed-AST pass that depends on
//! AST builder helpers that aren't yet stabilised — tracked separately.
//!
//! Until that lands, callers can hand-write `render()` for their DSL types
//! (this is what `core/shell/dsl/{git,docker,kubectl}.vr` already do); the
//! attribute is still useful as a compile-time marker that survives the
//! attribute validator.

use super::common::DeriveContext;
use super::{DeriveMacro, DeriveResult};
use verum_ast::Span;
use verum_ast::decl::{Item, ItemKind};

pub struct DeriveShellRender;

impl DeriveMacro for DeriveShellRender {
    fn name(&self) -> &'static str { "ShellRender" }
    fn protocol_name(&self) -> &'static str { "ShellRender" }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        // Emit a no-op item that downstream passes safely ignore.  The
        // type-decl itself is not modified; this just returns a marker
        // so the derive registry pipeline succeeds.
        let span: Span = ctx.span;
        let _ = ctx;
        // Verum uses `mount` (not `use`); emit an empty Nested
        // mount as the no-op marker. The pipeline ignores empty
        // mounts.
        Ok(Item::new(
            ItemKind::Mount(verum_ast::decl::MountDecl {
                visibility: verum_ast::decl::Visibility::Private,
                tree: verum_ast::decl::MountTree {
                    kind: verum_ast::decl::MountTreeKind::Nested {
                        prefix: verum_ast::ty::Path::new(
                            verum_common::List::new(),
                            span,
                        ),
                        trees: verum_common::List::new(),
                    },
                    alias: verum_common::Maybe::None,
                    span,
                },
                alias: verum_common::Maybe::None,
                span,
            }),
            span,
        ))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated render() for typed shell command DSLs"
    }
}
