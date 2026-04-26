//! # VVA extension governance gating (B12, #211)
//!
//! Closes red-team gap B12. The typed attribute
//! [`verum_ast::attr::typed::ExtensionRequirementAttr`] has shipped
//! since the VVA V0 layer (commit f53ae27b), but no verification
//! pass consumed it: kernel rules from VVA-N extensions ran
//! unconditionally in every module. This violates the VVA §0.0
//! rollout governance:
//!
//!   * **Year 0–2**: VVA rules are *opt-in only* via
//!     `@require_extension(vfe_N)`.
//!   * **Year 2–4**: rules become default-on; opt-out via
//!     `@disable_extension(vfe_N)`.
//!   * **Year 4+**: opt-out is removed; rule is a hard requirement.
//!
//! V8 ships the gating *infrastructure* — the policy enum, the
//! attribute scanner, and the active-extension predicate — without
//! flipping any production-pass default. Each VVA-aware pass
//! (currently `KernelRecheckPass`) gains a builder
//! (`with_extension_policy`) that drives gating from a configured
//! `ExtensionPolicy`. The default policy stays `AllRulesActive` so the
//! existing test corpus (which doesn't carry
//! `@require_extension`) continues to pass; flipping the default
//! to `OptInOnly` is a follow-up bump on the rollout calendar
//! (tracked alongside the next VVA minor bump per
//! `verum_kernel::VVA_VERSION`).
//!
//! ## Module-level vs item-level scope
//!
//! `@require_extension` may appear at module level (`Module.attributes`)
//! or on individual items. V8 reads from both; the module-level
//! scope establishes the baseline, and an item-level annotation
//! is an additive override (an item can require an extension the
//! module doesn't, but cannot disable an extension the module
//! requires — see [`EnabledExtensions::resolve_for_item`]).

use std::collections::HashSet;

use verum_ast::Item;
use verum_ast::Module;
use verum_ast::attr::Attribute;
use verum_ast::attr::{ExtensionRequirementAttr, ExtensionToggleKind};
use verum_common::{Maybe, Text};

/// V8 (#211, B12) — VVA rollout policy. Drives whether a given
/// extension is "active" for a given scope. `AllRulesActive` is
/// the V8 default and matches the pre-V8 always-on behaviour so
/// no existing tests regress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionPolicy {
    /// Year 0–2: VVA rules are off unless the scope explicitly
    /// `@require_extension(vfe_N)`.
    OptInOnly,
    /// Year 2–4: VVA rules are on unless the scope explicitly
    /// `@disable_extension(vfe_N)`.
    OptOutOnly,
    /// Year 4+: VVA rules are mandatory; `@disable_extension` is
    /// rejected with a hygiene-style error (V8 doesn't yet emit
    /// that error — wired alongside future Mandatory-tier flip).
    Mandatory,
    /// V8 default — runs every VVA rule regardless of opt-in.
    /// Equivalent to pre-V8 behaviour. Used while the rollout
    /// calendar is uncertain and to keep the existing VCS suite
    /// green.
    AllRulesActive,
}

impl ExtensionPolicy {
    /// Decide whether the named extension is active in a scope
    /// whose [`EnabledExtensions`] view is `set`.
    pub fn is_active(self, set: &EnabledExtensions, ext: &str) -> bool {
        match self {
            ExtensionPolicy::AllRulesActive => true,
            ExtensionPolicy::Mandatory => true,
            ExtensionPolicy::OptInOnly => set.requires(ext),
            ExtensionPolicy::OptOutOnly => !set.disables(ext),
        }
    }
}

/// View over the set of `@require_extension` / `@disable_extension`
/// annotations attached to an AST scope. Construct from a [`Module`]
/// via [`EnabledExtensions::from_module`] for the module-level set,
/// then narrow to a specific item via
/// [`EnabledExtensions::resolve_for_item`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnabledExtensions {
    /// Canonical (lowercased) extension identifiers required by
    /// this scope.
    explicitly_required: HashSet<Text>,
    /// Canonical (lowercased) extension identifiers disabled by
    /// this scope.
    explicitly_disabled: HashSet<Text>,
}

impl EnabledExtensions {
    /// An empty extension set — neither requires nor disables any
    /// extension. Suitable as the default for tests that don't
    /// exercise gating logic.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Walk a [`Module`]'s top-level attribute list for
    /// `@require_extension` / `@disable_extension` annotations.
    /// Item-level annotations are NOT included here; use
    /// [`Self::resolve_for_item`] to layer an item's annotations
    /// on top of the module baseline.
    pub fn from_module(module: &Module) -> Self {
        let mut me = Self::default();
        me.absorb_attrs(&module.attributes);
        me
    }

    /// Build an extension set from a raw attribute list. Useful for
    /// callers that already have the `List<Attribute>` slice in hand
    /// (e.g., when validating a synthetic decl or testing with a
    /// hand-crafted attribute set).
    pub fn from_attributes(attrs: &verum_common::List<Attribute>) -> Self {
        let mut me = Self::default();
        me.absorb_attrs(attrs);
        me
    }

    /// Layer the [`ItemKind`]-resident attributes (if any) on top
    /// of `self`. Returns a NEW [`EnabledExtensions`] — the
    /// original is unchanged. Item-level requires are unioned
    /// with the module-level set; item-level disables are also
    /// unioned (a function can opt out of an extension its
    /// module didn't take a position on).
    ///
    /// Conflict resolution: `@require_extension(vfe_X)` always
    /// wins over `@disable_extension(vfe_X)` for the same X
    /// in the same scope (the item is being explicit that it
    /// needs the rule). Cross-scope conflicts (module disables,
    /// item requires) are resolved in favour of the item.
    pub fn resolve_for_item(&self, item: &Item) -> Self {
        let mut me = self.clone();
        if let Some(attrs) = item_attrs(item) {
            me.absorb_attrs(attrs);
        }
        // Item-level explicit-require overrides any inherited disable.
        // `absorb_attrs` already keeps both sets, so we do a final
        // pass: anything in `explicitly_required` is removed from
        // `explicitly_disabled` to reflect the precedence above.
        me.explicitly_disabled = me
            .explicitly_disabled
            .difference(&me.explicitly_required)
            .cloned()
            .collect();
        me
    }

    /// True iff this scope explicitly requires `ext` via
    /// `@require_extension(<ext>)`.
    pub fn requires(&self, ext: &str) -> bool {
        self.explicitly_required.iter().any(|t| t.as_str() == ext)
    }

    /// True iff this scope explicitly disables `ext` via
    /// `@disable_extension(<ext>)`.
    pub fn disables(&self, ext: &str) -> bool {
        self.explicitly_disabled.iter().any(|t| t.as_str() == ext)
    }

    /// Sorted list of explicitly-required extensions (for stable
    /// diagnostic / audit output).
    pub fn required(&self) -> Vec<Text> {
        let mut v: Vec<Text> = self.explicitly_required.iter().cloned().collect();
        v.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        v
    }

    /// Sorted list of explicitly-disabled extensions (for stable
    /// diagnostic / audit output).
    pub fn disabled(&self) -> Vec<Text> {
        let mut v: Vec<Text> = self.explicitly_disabled.iter().cloned().collect();
        v.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        v
    }

    fn absorb_attrs(&mut self, attrs: &verum_common::List<Attribute>) {
        for attr in attrs.iter() {
            match ExtensionRequirementAttr::from_attribute(attr) {
                Maybe::Some(parsed) => {
                    match parsed.kind {
                        ExtensionToggleKind::Require => {
                            self.explicitly_required.insert(parsed.extension);
                        }
                        ExtensionToggleKind::Disable => {
                            self.explicitly_disabled.insert(parsed.extension);
                        }
                    }
                }
                Maybe::None => {}
            }
        }
    }
}

/// Attribute-list accessor for any [`Item`] kind that can carry
/// `@require_extension` / `@disable_extension`. Shapes that don't
/// host attributes (e.g., `Mount`, `FFIBoundary`) return `None`.
fn item_attrs(item: &Item) -> Option<&verum_common::List<Attribute>> {
    use verum_ast::decl::ItemKind;
    match &item.kind {
        ItemKind::Function(f) => Some(&f.attributes),
        ItemKind::Type(t) => Some(&t.attributes),
        ItemKind::Theorem(d) | ItemKind::Lemma(d) | ItemKind::Corollary(d) => {
            Some(&d.attributes)
        }
        ItemKind::Axiom(a) => Some(&a.attributes),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::{Ident, Span};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::ty::PathSegment;
    use verum_common::List;

    fn span() -> Span { Span::default() }

    fn ident(name: &str) -> Ident {
        Ident { name: Text::from(name), span: span() }
    }

    fn path_arg(name: &str) -> Expr {
        let mut segs: List<PathSegment> = List::new();
        segs.push(PathSegment::Name(ident(name)));
        Expr::new(
            ExprKind::Path(verum_ast::ty::Path::new(segs, span())),
            span(),
        )
    }

    fn make_attr(name: &str, ext: &str) -> Attribute {
        let mut args: List<Expr> = List::new();
        args.push(path_arg(ext));
        Attribute {
            name: Text::from(name),
            args: Maybe::Some(args),
            span: span(),
        }
    }

    fn attr_list(attrs: Vec<Attribute>) -> List<Attribute> {
        let mut l: List<Attribute> = List::new();
        for a in attrs {
            l.push(a);
        }
        l
    }

    #[test]
    fn empty_set_requires_nothing() {
        let s = EnabledExtensions::empty();
        assert!(!s.requires("vfe_1"));
        assert!(!s.disables("vfe_1"));
    }

    #[test]
    fn require_attribute_populates_required_set() {
        let attrs = attr_list(vec![make_attr("require_extension", "vfe_1")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(s.requires("vfe_1"));
        assert!(!s.disables("vfe_1"));
        assert_eq!(s.required(), vec![Text::from("vfe_1")]);
    }

    #[test]
    fn disable_attribute_populates_disabled_set() {
        let attrs = attr_list(vec![make_attr("disable_extension", "vfe_3")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(s.disables("vfe_3"));
        assert!(!s.requires("vfe_3"));
    }

    #[test]
    fn unrelated_attribute_ignored() {
        let attrs = attr_list(vec![make_attr("derive", "Clone")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(s.required().is_empty());
        assert!(s.disabled().is_empty());
    }

    #[test]
    fn policy_all_rules_active_always_on() {
        let s = EnabledExtensions::empty();
        assert!(ExtensionPolicy::AllRulesActive.is_active(&s, "vfe_7"));
    }

    #[test]
    fn policy_opt_in_only_off_without_require() {
        let s = EnabledExtensions::empty();
        assert!(!ExtensionPolicy::OptInOnly.is_active(&s, "vfe_7"));
    }

    #[test]
    fn policy_opt_in_only_on_with_require() {
        let attrs = attr_list(vec![make_attr("require_extension", "vfe_7")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(ExtensionPolicy::OptInOnly.is_active(&s, "vfe_7"));
        assert!(!ExtensionPolicy::OptInOnly.is_active(&s, "vfe_1"));
    }

    #[test]
    fn policy_opt_out_only_on_by_default() {
        let s = EnabledExtensions::empty();
        assert!(ExtensionPolicy::OptOutOnly.is_active(&s, "vfe_3"));
    }

    #[test]
    fn policy_opt_out_only_off_when_disabled() {
        let attrs = attr_list(vec![make_attr("disable_extension", "vfe_3")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(!ExtensionPolicy::OptOutOnly.is_active(&s, "vfe_3"));
        assert!(ExtensionPolicy::OptOutOnly.is_active(&s, "vfe_1"));
    }

    #[test]
    fn policy_mandatory_always_on() {
        let attrs = attr_list(vec![make_attr("disable_extension", "vfe_3")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        // Mandatory ignores @disable_extension.
        assert!(ExtensionPolicy::Mandatory.is_active(&s, "vfe_3"));
    }

    #[test]
    fn require_with_text_literal_extension_id_canonicalises() {
        // `@require_extension("vfe_2")` must canonicalise to "vfe_2".
        use verum_ast::literal::{Literal, LiteralKind, StringLit};
        let mut args: List<Expr> = List::new();
        args.push(Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Text(StringLit::Regular(Text::from("vfe_2"))),
                span: span(),
            }),
            span(),
        ));
        let attr = Attribute {
            name: Text::from("require_extension"),
            args: Maybe::Some(args),
            span: span(),
        };
        let attrs = attr_list(vec![attr]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(s.requires("vfe_2"));
    }

    #[test]
    fn malformed_extension_id_silently_dropped() {
        // `@require_extension(not_a_vfe)` is structurally invalid;
        // the typed-attribute parser returns Maybe::None and the
        // scanner silently drops it. (This is symmetric with how
        // other typed-attribute scanners handle parse failures —
        // a hygiene-pass diagnostic for malformed extension ids
        // is a separate follow-up, tracked alongside the rollout
        // policy flip.)
        let attrs = attr_list(vec![make_attr("require_extension", "not_a_vfe")]);
        let s = EnabledExtensions::from_attributes(&attrs);
        assert!(!s.requires("not_a_vfe"));
        assert!(s.required().is_empty());
    }
}
