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
// Comprehensive tests for the attr module.
//

// Tests all functionality in verum_ast::attr including:
// - Profile enum and its methods
// - ProfileAttr and profile compatibility
// - FeatureAttr and feature validation
// - Generic Attribute handling
//

// Tests for language profile attributes.

use verum_ast::attr::*;
use verum_ast::span::{FileId, Span, Spanned};
use verum_common::List;
use verum_common::{Heap, Maybe, Text};

// ============================================================================
// Profile Tests
// ============================================================================

#[test]
fn test_profile_from_str_valid_application() {
    let profile = Profile::from_str("application");
    assert_eq!(profile, Maybe::Some(Profile::Application));
}

#[test]
fn test_profile_from_str_valid_systems() {
    let profile = Profile::from_str("systems");
    assert_eq!(profile, Maybe::Some(Profile::Systems));
}

#[test]
fn test_profile_from_str_valid_research() {
    let profile = Profile::from_str("research");
    assert_eq!(profile, Maybe::Some(Profile::Research));
}

#[test]
fn test_profile_from_str_invalid() {
    let profile = Profile::from_str("invalid_profile");
    assert_eq!(profile, Maybe::None);
}

#[test]
fn test_profile_from_str_empty() {
    let profile = Profile::from_str("");
    assert_eq!(profile, Maybe::None);
}

#[test]
fn test_profile_from_str_case_sensitive() {
    // Profile names should be lowercase
    let profile = Profile::from_str("Application");
    assert_eq!(profile, Maybe::None);
}

#[test]
fn test_profile_as_str_application() {
    assert_eq!(Profile::Application.as_str(), "application");
}

#[test]
fn test_profile_as_str_systems() {
    assert_eq!(Profile::Systems.as_str(), "systems");
}

#[test]
fn test_profile_as_str_research() {
    assert_eq!(Profile::Research.as_str(), "research");
}

#[test]
fn test_profile_as_str_roundtrip() {
    // Test that from_str(as_str()) is identity
    for profile in [Profile::Application, Profile::Systems, Profile::Research] {
        let s = profile.as_str();
        let parsed = Profile::from_str(s);
        assert_eq!(parsed, Maybe::Some(profile));
    }
}

#[test]
fn test_profile_display() {
    assert_eq!(format!("{}", Profile::Application), "application");
    assert_eq!(format!("{}", Profile::Systems), "systems");
    assert_eq!(format!("{}", Profile::Research), "research");
}

// ============================================================================
// Profile Restriction Tests
// ============================================================================

#[test]
fn test_profile_is_more_restrictive_research_vs_application() {
    assert!(Profile::Research.is_more_restrictive_than(&Profile::Application));
}

#[test]
fn test_profile_is_more_restrictive_research_vs_systems() {
    assert!(Profile::Research.is_more_restrictive_than(&Profile::Systems));
}

#[test]
fn test_profile_is_more_restrictive_systems_vs_application() {
    assert!(Profile::Systems.is_more_restrictive_than(&Profile::Application));
}

#[test]
fn test_profile_is_more_restrictive_same_profile() {
    // Same profile is not more restrictive than itself
    assert!(!Profile::Application.is_more_restrictive_than(&Profile::Application));
    assert!(!Profile::Systems.is_more_restrictive_than(&Profile::Systems));
    assert!(!Profile::Research.is_more_restrictive_than(&Profile::Research));
}

#[test]
fn test_profile_is_more_restrictive_reverse() {
    // Application is NOT more restrictive than Systems
    assert!(!Profile::Application.is_more_restrictive_than(&Profile::Systems));
    // Application is NOT more restrictive than Research
    assert!(!Profile::Application.is_more_restrictive_than(&Profile::Research));
    // Systems is NOT more restrictive than Research
    assert!(!Profile::Systems.is_more_restrictive_than(&Profile::Research));
}

// ============================================================================
// Profile Capabilities Tests
// ============================================================================

#[test]
fn test_profile_allows_unsafe() {
    assert!(!Profile::Application.allows_unsafe());
    assert!(Profile::Systems.allows_unsafe());
    assert!(!Profile::Research.allows_unsafe());
}

#[test]
fn test_profile_requires_verification() {
    assert!(!Profile::Application.requires_verification());
    assert!(!Profile::Systems.requires_verification());
    assert!(Profile::Research.requires_verification());
}

// ============================================================================
// ProfileAttr Tests
// ============================================================================

#[test]
fn test_profile_attr_new() {
    let span = Span::default();
    let mut profiles = List::new();
    profiles.push(Profile::Application);

    let attr = ProfileAttr::new(profiles.clone(), span);
    assert_eq!(attr.profiles, profiles);
    assert_eq!(attr.span, span);
}

#[test]
fn test_profile_attr_single() {
    let span = Span::default();
    let attr = ProfileAttr::single(Profile::Systems, span);

    assert_eq!(attr.profiles.len(), 1);
    assert_eq!(attr.profiles[0], Profile::Systems);
}

#[test]
fn test_profile_attr_multiple_profiles() {
    let span = Span::default();
    let mut profiles = List::new();
    profiles.push(Profile::Systems);
    profiles.push(Profile::Research);

    let attr = ProfileAttr::new(profiles, span);
    assert_eq!(attr.profiles.len(), 2);
}

#[test]
fn test_profile_attr_contains() {
    let span = Span::default();
    let mut profiles = List::new();
    profiles.push(Profile::Application);
    profiles.push(Profile::Systems);

    let attr = ProfileAttr::new(profiles, span);

    assert!(attr.contains(Profile::Application));
    assert!(attr.contains(Profile::Systems));
    assert!(!attr.contains(Profile::Research));
}

#[test]
fn test_profile_attr_contains_single() {
    let span = Span::default();
    let attr = ProfileAttr::single(Profile::Research, span);

    assert!(attr.contains(Profile::Research));
    assert!(!attr.contains(Profile::Application));
    assert!(!attr.contains(Profile::Systems));
}

// ============================================================================
// ProfileAttr Compatibility Tests
// ============================================================================

#[test]
fn test_profile_attr_is_compatible_same_profile() {
    let span = Span::default();
    let parent = ProfileAttr::single(Profile::Application, span);
    let child = ProfileAttr::single(Profile::Application, span);

    assert!(child.is_compatible_with(&parent));
}

#[test]
fn test_profile_attr_is_compatible_child_more_restrictive() {
    let span = Span::default();
    let parent = ProfileAttr::single(Profile::Application, span);
    let child = ProfileAttr::single(Profile::Systems, span);

    // Systems is more restrictive than Application
    assert!(child.is_compatible_with(&parent));
}

#[test]
fn test_profile_attr_is_compatible_child_most_restrictive() {
    let span = Span::default();
    let parent = ProfileAttr::single(Profile::Application, span);
    let child = ProfileAttr::single(Profile::Research, span);

    // Research is most restrictive
    assert!(child.is_compatible_with(&parent));
}

#[test]
fn test_profile_attr_is_not_compatible_child_less_restrictive() {
    let span = Span::default();
    let parent = ProfileAttr::single(Profile::Systems, span);
    let child = ProfileAttr::single(Profile::Application, span);

    // Application is LESS restrictive than Systems - should be incompatible
    assert!(!child.is_compatible_with(&parent));
}

#[test]
fn test_profile_attr_is_compatible_multiple_profiles() {
    let span = Span::default();

    let mut parent_profiles = List::new();
    parent_profiles.push(Profile::Application);
    parent_profiles.push(Profile::Systems);
    let parent = ProfileAttr::new(parent_profiles, span);

    // Child supports Systems - compatible
    let child = ProfileAttr::single(Profile::Systems, span);
    assert!(child.is_compatible_with(&parent));
}

#[test]
fn test_profile_attr_is_compatible_overlapping_profiles() {
    let span = Span::default();

    let mut parent_profiles = List::new();
    parent_profiles.push(Profile::Application);
    let parent = ProfileAttr::new(parent_profiles, span);

    let mut child_profiles = List::new();
    child_profiles.push(Profile::Systems);
    child_profiles.push(Profile::Research);
    let child = ProfileAttr::new(child_profiles, span);

    // Child has at least one profile compatible with parent
    assert!(child.is_compatible_with(&parent));
}

// ============================================================================
// FeatureAttr Tests
// ============================================================================

#[test]
fn test_feature_attr_new() {
    let span = Span::default();
    let mut features = List::new();
    features.push(Text::from("unsafe"));

    let attr = FeatureAttr::new(features.clone(), span);
    assert_eq!(attr.features, features);
    assert_eq!(attr.span, span);
}

#[test]
fn test_feature_attr_has_feature() {
    let span = Span::default();
    let mut features = List::new();
    features.push(Text::from("unsafe"));
    features.push(Text::from("inline_asm"));

    let attr = FeatureAttr::new(features, span);

    assert!(attr.has_feature("unsafe"));
    assert!(attr.has_feature("inline_asm"));
    assert!(!attr.has_feature("custom_allocator"));
}

#[test]
fn test_feature_attr_has_feature_empty() {
    let span = Span::default();
    let features = List::new();
    let attr = FeatureAttr::new(features, span);

    assert!(!attr.has_feature("unsafe"));
}

#[test]
fn test_feature_attr_known_features() {
    let known = FeatureAttr::known_features();

    // Check that known features include expected ones
    assert!(known.contains(&"unsafe"));
    assert!(known.contains(&"inline_asm"));
    assert!(known.contains(&"custom_allocator"));
    assert!(known.contains(&"raw_pointers"));
    assert!(known.contains(&"manual_drop"));
    assert!(known.contains(&"volatile_access"));
}

#[test]
fn test_feature_attr_validate_valid_features() {
    let span = Span::default();
    let mut features = List::new();
    features.push(Text::from("unsafe"));
    features.push(Text::from("inline_asm"));

    let attr = FeatureAttr::new(features, span);

    assert!(attr.validate().is_ok());
}

#[test]
fn test_feature_attr_validate_unknown_feature() {
    let span = Span::default();
    let mut features = List::new();
    features.push(Text::from("unknown_feature"));

    let attr = FeatureAttr::new(features, span);

    let result = attr.validate();
    assert!(result.is_err());

    if let Err(err) = result {
        assert!(err.as_str().contains("unknown_feature"));
        assert!(err.as_str().contains("Unknown feature"));
    }
}

#[test]
fn test_feature_attr_validate_mixed_valid_invalid() {
    let span = Span::default();
    let mut features = List::new();
    features.push(Text::from("unsafe"));
    features.push(Text::from("invalid_feature"));

    let attr = FeatureAttr::new(features, span);

    let result = attr.validate();
    assert!(result.is_err());
}

#[test]
fn test_feature_attr_validate_empty() {
    let span = Span::default();
    let features = List::new();
    let attr = FeatureAttr::new(features, span);

    // Empty features list should be valid
    assert!(attr.validate().is_ok());
}

#[test]
fn test_feature_attr_validate_all_known_features() {
    let span = Span::default();
    let mut features = List::new();

    // Add all known features
    for feature in FeatureAttr::known_features() {
        features.push(Text::from(*feature));
    }

    let attr = FeatureAttr::new(features, span);

    // Should validate successfully
    assert!(attr.validate().is_ok());
}

// ============================================================================
// Generic Attribute Tests
// ============================================================================

#[test]
fn test_attribute_new() {
    let span = Span::default();
    let name = Text::from("inline");
    let args = Maybe::None;

    let attr = Attribute::new(name.clone(), args, span);
    assert_eq!(attr.name, name);
    assert_eq!(attr.args, Maybe::None);
    assert_eq!(attr.span, span);
}

#[test]
fn test_attribute_simple() {
    let span = Span::default();
    let name = Text::from("deprecated");

    let attr = Attribute::simple(name.clone(), span);
    assert_eq!(attr.name, name);
    assert_eq!(attr.args, Maybe::None);
}

#[test]
fn test_attribute_is_named() {
    let span = Span::default();
    let attr = Attribute::simple(Text::from("inline"), span);

    assert!(attr.is_named("inline"));
    assert!(!attr.is_named("deprecated"));
    assert!(!attr.is_named(""));
}

#[test]
fn test_attribute_is_named_case_sensitive() {
    let span = Span::default();
    let attr = Attribute::simple(Text::from("inline"), span);

    assert!(!attr.is_named("Inline"));
    assert!(!attr.is_named("INLINE"));
}

#[test]
fn test_attribute_with_args() {
    let span = Span::default();
    let name = Text::from("custom");

    // Create attribute with arguments
    let args = List::new();
    // Note: We can't easily create Expr without circular dependencies
    // This test verifies the structure accepts args
    let attr = Attribute::new(name, Maybe::Some(args), span);

    assert!(matches!(attr.args, Maybe::Some(_)));
}

// ============================================================================
// Edge Cases and Integration Tests
// ============================================================================

#[test]
fn test_profile_attr_span_preservation() {
    let file_id = FileId::new(1);
    let span = Span::new(10, 20, file_id);
    let attr = ProfileAttr::single(Profile::Application, span);

    assert_eq!(attr.span().start, 10);
    assert_eq!(attr.span().end, 20);
    assert_eq!(attr.span().file_id, file_id);
}

#[test]
fn test_feature_attr_span_preservation() {
    let file_id = FileId::new(2);
    let span = Span::new(30, 40, file_id);
    let features = List::new();
    let attr = FeatureAttr::new(features, span);

    assert_eq!(attr.span().start, 30);
    assert_eq!(attr.span().end, 40);
    assert_eq!(attr.span().file_id, file_id);
}

#[test]
fn test_attribute_span_preservation() {
    let file_id = FileId::new(3);
    let span = Span::new(50, 60, file_id);
    let attr = Attribute::simple(Text::from("test"), span);

    assert_eq!(attr.span().start, 50);
    assert_eq!(attr.span().end, 60);
    assert_eq!(attr.span().file_id, file_id);
}

#[test]
fn test_profile_serialization_roundtrip() {
    use serde_json;

    let profile = Profile::Research;
    let serialized = serde_json::to_string(&profile).unwrap();
    let deserialized: Profile = serde_json::from_str(&serialized).unwrap();

    assert_eq!(profile, deserialized);
}

#[test]
fn test_profile_attr_clone() {
    let span = Span::default();
    let attr = ProfileAttr::single(Profile::Systems, span);
    let cloned = attr.clone();

    assert_eq!(attr, cloned);
}

#[test]
fn test_feature_attr_clone() {
    let span = Span::default();
    let mut features = List::new();
    features.push(Text::from("unsafe"));
    let attr = FeatureAttr::new(features, span);
    let cloned = attr.clone();

    assert_eq!(attr, cloned);
}

#[test]
fn test_attribute_clone() {
    let span = Span::default();
    let attr = Attribute::simple(Text::from("test"), span);
    let cloned = attr.clone();

    assert_eq!(attr, cloned);
}

// ============================================================================
// Specification Compliance Tests
// ============================================================================

/// Test profile hierarchy per spec: Application < Systems < Research
/// Tests for language profile attributes..1
#[test]
fn test_profile_hierarchy_specification() {
    // Research is most restrictive
    assert!(Profile::Research.is_more_restrictive_than(&Profile::Systems));
    assert!(Profile::Research.is_more_restrictive_than(&Profile::Application));

    // Systems is middle restrictiveness
    assert!(Profile::Systems.is_more_restrictive_than(&Profile::Application));
    assert!(!Profile::Systems.is_more_restrictive_than(&Profile::Research));

    // Application is least restrictive
    assert!(!Profile::Application.is_more_restrictive_than(&Profile::Systems));
    assert!(!Profile::Application.is_more_restrictive_than(&Profile::Research));
}

/// Test that only Systems profile allows unsafe
/// Tests for language profile attributes..1
#[test]
fn test_unsafe_allowed_only_in_systems() {
    assert!(
        !Profile::Application.allows_unsafe(),
        "Application should not allow unsafe"
    );
    assert!(
        Profile::Systems.allows_unsafe(),
        "Systems should allow unsafe"
    );
    assert!(
        !Profile::Research.allows_unsafe(),
        "Research should not allow unsafe"
    );
}

/// Test that only Research profile requires verification
/// Tests for language profile attributes..1
#[test]
fn test_verification_required_only_in_research() {
    assert!(
        !Profile::Application.requires_verification(),
        "Application should not require verification"
    );
    assert!(
        !Profile::Systems.requires_verification(),
        "Systems should not require verification"
    );
    assert!(
        Profile::Research.requires_verification(),
        "Research should require verification"
    );
}

// ============================================================================
// TaggedLiteralAttr Tests - meta-system tagged literal handlers
// ============================================================================

#[test]
fn test_tagged_literal_attr_new() {
    let span = Span::default();
    let attr = TaggedLiteralAttr::new(Text::from("json"), span);

    assert_eq!(attr.tag, Text::from("json"));
    assert_eq!(attr.span, span);
}

#[test]
fn test_tagged_literal_attr_common_tags() {
    let span = Span::default();

    // Test various common tagged literal types
    let json = TaggedLiteralAttr::new(Text::from("json"), span);
    assert_eq!(json.tag.as_str(), "json");

    let sql = TaggedLiteralAttr::new(Text::from("sql"), span);
    assert_eq!(sql.tag.as_str(), "sql");

    let regex = TaggedLiteralAttr::new(Text::from("regex"), span);
    assert_eq!(regex.tag.as_str(), "regex");

    let img = TaggedLiteralAttr::new(Text::from("img"), span);
    assert_eq!(img.tag.as_str(), "img");

    let font = TaggedLiteralAttr::new(Text::from("font"), span);
    assert_eq!(font.tag.as_str(), "font");
}

#[test]
fn test_tagged_literal_attr_spanned() {
    let file_id = FileId::new(6);
    let span = Span::new(300, 400, file_id);
    let attr = TaggedLiteralAttr::new(Text::from("test"), span);

    assert_eq!(attr.span().start, 300);
    assert_eq!(attr.span().end, 400);
    assert_eq!(attr.span().file_id, file_id);
}

#[test]
fn test_tagged_literal_attr_clone_eq() {
    let span = Span::default();
    let attr = TaggedLiteralAttr::new(Text::from("data"), span);
    let cloned = attr.clone();

    assert_eq!(attr, cloned);
}

// ============================================================================
// FrameworkAttr Tests — `@framework(name, "citation")`
// ============================================================================

use verum_ast::Ident;
use verum_ast::expr::Expr;
use verum_ast::literal::{Literal, LiteralKind, StringLit};

/// Build `@framework(<name_ident>, "<citation>")` as a generic `Attribute`.
fn build_framework_attr(name: &str, citation: &str) -> Attribute {
    let span = Span::default();
    let name_expr = Expr::ident(Ident::new(Text::from(name), span));
    let citation_lit = Literal::new(
        LiteralKind::Text(StringLit::Regular(Text::from(citation))),
        span,
    );
    let citation_expr = Expr::literal(citation_lit);
    let mut args = List::new();
    args.push(name_expr);
    args.push(citation_expr);
    Attribute::new(Text::from("framework"), Maybe::Some(args), span)
}

#[test]
fn test_framework_attr_extracts_from_generic_attribute() {
    let raw = build_framework_attr("lurie_htt", "HTT 6.2.2.7");
    let typed = FrameworkAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(fw) => {
            assert_eq!(fw.name.as_str(), "lurie_htt");
            assert_eq!(fw.citation.as_str(), "HTT 6.2.2.7");
        }
        Maybe::None => panic!("FrameworkAttr::from_attribute returned None on valid input"),
    }
}

#[test]
fn test_framework_attr_rejects_wrong_name() {
    let raw = Attribute::new(Text::from("inline"), Maybe::None, Span::default());
    assert!(matches!(FrameworkAttr::from_attribute(&raw), Maybe::None));
}

#[test]
fn test_framework_attr_rejects_missing_args() {
    let raw = Attribute::new(Text::from("framework"), Maybe::None, Span::default());
    assert!(matches!(FrameworkAttr::from_attribute(&raw), Maybe::None));
}

#[test]
fn test_framework_attr_display_roundtrip() {
    let raw = build_framework_attr("schreiber_dcct", "DCCT §3.9");
    let typed = FrameworkAttr::from_attribute(&raw);
    let fw = match typed {
        Maybe::Some(fw) => fw,
        Maybe::None => panic!("extraction failed"),
    };
    assert_eq!(
        format!("{}", fw),
        r#"@framework(schreiber_dcct, "DCCT §3.9")"#
    );
}

// ============================================================================
// ExtractAttr Tests — `@extract` / `@extract(<target>)`
// ============================================================================

#[test]
fn test_extract_bare_defaults_to_verum() {
    let raw = Attribute::new(Text::from("extract"), Maybe::None, Span::default());
    match ExtractAttr::from_attribute(&raw) {
        Maybe::Some(a) => assert_eq!(a.target, ExtractTarget::Verum),
        Maybe::None => panic!("bare @extract must default to Verum"),
    }
}

#[test]
fn test_extract_with_target_lean() {
    let span = Span::default();
    let target_expr = Expr::ident(Ident::new(Text::from("lean"), span));
    let mut args = List::new();
    args.push(target_expr);
    let raw = Attribute::new(Text::from("extract"), Maybe::Some(args), span);
    match ExtractAttr::from_attribute(&raw) {
        Maybe::Some(a) => assert_eq!(a.target, ExtractTarget::Lean),
        Maybe::None => panic!("@extract(lean) must parse"),
    }
}

#[test]
fn test_extract_rejects_unknown_target() {
    let span = Span::default();
    let target_expr = Expr::ident(Ident::new(Text::from("haskell"), span));
    let mut args = List::new();
    args.push(target_expr);
    let raw = Attribute::new(Text::from("extract"), Maybe::Some(args), span);
    assert!(matches!(ExtractAttr::from_attribute(&raw), Maybe::None));
}

#[test]
fn test_extract_rejects_wrong_name() {
    let raw = Attribute::new(Text::from("inline"), Maybe::None, Span::default());
    assert!(matches!(ExtractAttr::from_attribute(&raw), Maybe::None));
}

#[test]
fn test_extract_witness_with_coq() {
    let span = Span::default();
    let target_expr = Expr::ident(Ident::new(Text::from("coq"), span));
    let mut args = List::new();
    args.push(target_expr);
    let raw = Attribute::new(Text::from("extract_witness"), Maybe::Some(args), span);
    match ExtractWitnessAttr::from_attribute(&raw) {
        Maybe::Some(a) => assert_eq!(a.target, ExtractTarget::Coq),
        Maybe::None => panic!("@extract_witness(coq) must parse"),
    }
}

#[test]
fn test_extract_contract_bare_defaults_to_verum() {
    let raw = Attribute::new(Text::from("extract_contract"), Maybe::None, Span::default());
    match ExtractContractAttr::from_attribute(&raw) {
        Maybe::Some(a) => assert_eq!(a.target, ExtractTarget::Verum),
        Maybe::None => panic!("bare @extract_contract must default to Verum"),
    }
}

#[test]
fn test_extract_target_from_ident_round_trip() {
    for (s, expected) in &[
        ("verum", ExtractTarget::Verum),
        ("VERUM", ExtractTarget::Verum),
        ("ocaml", ExtractTarget::OCaml),
        ("lean", ExtractTarget::Lean),
        ("Coq", ExtractTarget::Coq),
    ] {
        assert_eq!(ExtractTarget::from_ident(s), Some(*expected), "{}", s);
    }
    assert_eq!(ExtractTarget::from_ident("rust"), None);
    assert_eq!(ExtractTarget::from_ident(""), None);
}

#[test]
fn test_extract_attr_display_format() {
    let attr = ExtractAttr::new(ExtractTarget::Lean, Span::default());
    assert_eq!(format!("{}", attr), "@extract(lean)");
    let wattr = ExtractWitnessAttr::new(ExtractTarget::Coq, Span::default());
    assert_eq!(format!("{}", wattr), "@extract_witness(coq)");
    let cattr = ExtractContractAttr::new(ExtractTarget::Verum, Span::default());
    assert_eq!(format!("{}", cattr), "@extract_contract(verum)");
}
// ============================================================================
// FrameworkTranslateAttr Tests — `@framework_translate(source, target, "...")`
// ============================================================================

fn build_framework_translate_attr(source: &str, target: &str, citation: &str) -> Attribute {
    let span = Span::default();
    let source_expr = Expr::ident(Ident::new(Text::from(source), span));
    let target_expr = Expr::ident(Ident::new(Text::from(target), span));
    let citation_lit = Literal::new(
        LiteralKind::Text(StringLit::Regular(Text::from(citation))),
        span,
    );
    let citation_expr = Expr::literal(citation_lit);
    let mut args = List::new();
    args.push(source_expr);
    args.push(target_expr);
    args.push(citation_expr);
    Attribute::new(Text::from("framework_translate"), Maybe::Some(args), span)
}

#[test]
fn test_framework_translate_extracts_from_generic_attribute() {
    let raw = build_framework_translate_attr("owl2_fs", "lurie_htt", "Class → Presheaf");
    let typed = FrameworkTranslateAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(t) => {
            assert_eq!(t.source.as_str(), "owl2_fs");
            assert_eq!(t.target.as_str(), "lurie_htt");
            assert_eq!(t.citation.as_str(), "Class → Presheaf");
        }
        Maybe::None => {
            panic!("FrameworkTranslateAttr::from_attribute returned None on valid input")
        }
    }
}

#[test]
fn test_framework_translate_rejects_wrong_name() {
    let raw = build_framework_attr("lurie_htt", "HTT 6.2.2.7");
    assert!(matches!(
        FrameworkTranslateAttr::from_attribute(&raw),
        Maybe::None
    ));
}

#[test]
fn test_framework_translate_rejects_two_args() {
    // Two args (source, target) without citation must reject.
    let span = Span::default();
    let source_expr = Expr::ident(Ident::new(Text::from("a"), span));
    let target_expr = Expr::ident(Ident::new(Text::from("b"), span));
    let mut args = List::new();
    args.push(source_expr);
    args.push(target_expr);
    let raw = Attribute::new(Text::from("framework_translate"), Maybe::Some(args), span);
    assert!(matches!(
        FrameworkTranslateAttr::from_attribute(&raw),
        Maybe::None
    ));
}

#[test]
fn test_framework_translate_display_roundtrip() {
    let raw = build_framework_translate_attr("owl2_fs", "lurie_htt", "ObjectProperty → Functor");
    let typed = FrameworkTranslateAttr::from_attribute(&raw);
    let t = match typed {
        Maybe::Some(t) => t,
        Maybe::None => panic!("extraction failed"),
    };
    assert_eq!(
        format!("{}", t),
        r#"@framework_translate(owl2_fs, lurie_htt, "ObjectProperty → Functor")"#
    );
}

// ============================================================================
// OWL 2 ATTRIBUTION FAMILY — Phase 3 C8
// ============================================================================
//

// Round-trip tests for every Owl2*Attr from typed.rs. Builds a generic
// `Attribute` representing the surface form, then asserts the typed
// extractor produces the expected struct (or returns None for malformed
// shapes — silent acceptance of a typo would be a bug).

use verum_ast::expr::{ArrayExpr, BinOp, ExprKind};

/// Build a `name = "value"` Binary-Assign expression — the lowering
/// the parser produces for both `name: "value"` (colon) and `name =
/// "value"` (equals) named-arg forms.
fn named_string(name: &str, value: &str) -> Expr {
    let span = Span::default();
    let key = Expr::ident(Ident::new(Text::from(name), span));
    let lit = Literal::new(
        LiteralKind::Text(StringLit::Regular(Text::from(value))),
        span,
    );
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Assign,
            left: Heap::new(key),
            right: Heap::new(Expr::literal(lit)),
        },
        span,
    )
}

/// Build a `name = ClassPath` Binary-Assign expression for typed-class
/// references (`@owl2_property(domain = Animal)`).
fn named_class(name: &str, class: &str) -> Expr {
    let span = Span::default();
    let key = Expr::ident(Ident::new(Text::from(name), span));
    let class_expr = Expr::ident(Ident::new(Text::from(class), span));
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Assign,
            left: Heap::new(key),
            right: Heap::new(class_expr),
        },
        span,
    )
}

/// Build a `name = [Class1, Class2, ...]` Binary-Assign expression for
/// the `characteristic = [Transitive, Symmetric]` argument shape.
fn named_class_list(name: &str, classes: &[&str]) -> Expr {
    let span = Span::default();
    let key = Expr::ident(Ident::new(Text::from(name), span));
    let mut elems: List<Expr> = List::new();
    for c in classes {
        elems.push(Expr::ident(Ident::new(Text::from(*c), span)));
    }
    let arr = Expr::new(ExprKind::Array(ArrayExpr::List(elems)), span);
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Assign,
            left: Heap::new(key),
            right: Heap::new(arr),
        },
        span,
    )
}

fn class_ident(name: &str) -> Expr {
    Expr::ident(Ident::new(Text::from(name), Span::default()))
}

fn class_array(names: &[&str]) -> Expr {
    let span = Span::default();
    let mut elems: List<Expr> = List::new();
    for n in names {
        elems.push(class_ident(n));
    }
    Expr::new(ExprKind::Array(ArrayExpr::List(elems)), span)
}

// ----- @owl2_class -----
//
// OWL 2 Direct Semantics is open-world per W3C §5.6 — `@owl2_class`
// admits no `semantics = ...` argument. The earlier ClosedWorld
// opt-in surface has been removed; CWA-style closed-domain
// reasoning is expressed through Verum's refinement-type system or
// `count_o`'s explicit-witness contract, neither of which lives at
// this attribute layer.

#[test]
fn owl2_class_attr_no_args_accepts() {
    let raw = Attribute::new(Text::from("owl2_class"), Maybe::None, Span::default());
    let typed = Owl2ClassAttr::from_attribute(&raw);
    assert!(
        matches!(typed, Maybe::Some(_)),
        "@owl2_class without args must parse"
    );
}

#[test]
fn owl2_class_attr_empty_args_accepts() {
    // `@owl2_class()` is parser-uniform with `@owl2_class`.
    let args: List<Expr> = List::new();
    let raw = Attribute::new(Text::from("owl2_class"), Maybe::Some(args), Span::default());
    let typed = Owl2ClassAttr::from_attribute(&raw);
    assert!(
        matches!(typed, Maybe::Some(_)),
        "@owl2_class() with empty args must parse"
    );
}

#[test]
fn owl2_class_attr_rejects_legacy_semantics_arg() {
    // Holdover `semantics = "OpenWorld"` / `"ClosedWorld"` from
    // the legacy surface MUST be rejected — OWL 2 DS is OWA-only
    // per W3C §5.6, no per-class semantics flag is admitted.
    let mut args: List<Expr> = List::new();
    args.push(named_string("semantics", "OpenWorld"));
    let raw = Attribute::new(Text::from("owl2_class"), Maybe::Some(args), Span::default());
    assert!(
        matches!(Owl2ClassAttr::from_attribute(&raw), Maybe::None),
        "@owl2_class(semantics: OpenWorld) must NOT parse — surface is OWA-only"
    );
}

#[test]
fn owl2_class_attr_rejects_any_args() {
    // Defensive: arbitrary args should not be silently accepted.
    let mut args: List<Expr> = List::new();
    args.push(named_string("semantics", "TimeShared"));
    let raw = Attribute::new(Text::from("owl2_class"), Maybe::Some(args), Span::default());
    assert!(matches!(Owl2ClassAttr::from_attribute(&raw), Maybe::None));
}

// ----- @owl2_subclass_of -----

#[test]
fn owl2_subclass_of_accepts_path_form() {
    let mut args: List<Expr> = List::new();
    args.push(class_ident("Animal"));
    let raw = Attribute::new(
        Text::from("owl2_subclass_of"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2SubClassOfAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(s) => assert_eq!(s.parent.as_str(), "Animal"),
        Maybe::None => panic!("@owl2_subclass_of(Animal) must parse"),
    }
}

// ----- @owl2_disjoint_with -----

#[test]
fn owl2_disjoint_with_accepts_bracketed_list() {
    let mut args: List<Expr> = List::new();
    args.push(class_array(&["Foo", "Bar", "Baz"]));
    let raw = Attribute::new(
        Text::from("owl2_disjoint_with"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2DisjointWithAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(d) => {
            assert_eq!(d.disjoint_classes.len(), 3);
            assert_eq!(d.disjoint_classes[0].as_str(), "Foo");
            assert_eq!(d.disjoint_classes[2].as_str(), "Baz");
        }
        Maybe::None => panic!("@owl2_disjoint_with([Foo, Bar, Baz]) must parse"),
    }
}

#[test]
fn owl2_disjoint_with_accepts_positional_form() {
    let mut args: List<Expr> = List::new();
    args.push(class_ident("Pizza"));
    args.push(class_ident("IceCream"));
    let raw = Attribute::new(
        Text::from("owl2_disjoint_with"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2DisjointWithAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(d) => {
            assert_eq!(d.disjoint_classes.len(), 2);
            assert_eq!(d.disjoint_classes[1].as_str(), "IceCream");
        }
        Maybe::None => panic!("@owl2_disjoint_with(Pizza, IceCream) must parse"),
    }
}

// ----- @owl2_characteristic -----

#[test]
fn owl2_characteristic_parses_seven_canonical_names() {
    for &name in &[
        "Transitive",
        "Symmetric",
        "Asymmetric",
        "Reflexive",
        "Irreflexive",
        "Functional",
        "InverseFunctional",
    ] {
        let mut args: List<Expr> = List::new();
        args.push(class_ident(name));
        let raw = Attribute::new(
            Text::from("owl2_characteristic"),
            Maybe::Some(args),
            Span::default(),
        );
        let typed = Owl2CharacteristicAttr::from_attribute(&raw);
        match typed {
            Maybe::Some(c) => assert_eq!(c.characteristic.as_str(), name),
            Maybe::None => panic!("@owl2_characteristic({name}) must parse"),
        }
    }
}

#[test]
fn owl2_characteristic_rejects_unknown_flag() {
    let mut args: List<Expr> = List::new();
    args.push(class_ident("Idempotent"));
    let raw = Attribute::new(
        Text::from("owl2_characteristic"),
        Maybe::Some(args),
        Span::default(),
    );
    assert!(matches!(
        Owl2CharacteristicAttr::from_attribute(&raw),
        Maybe::None
    ));
}

// ----- @owl2_property -----

#[test]
fn owl2_property_full_form_with_inverse_and_characteristics() {
    let mut args: List<Expr> = List::new();
    args.push(named_class("domain", "Person"));
    args.push(named_class("range", "Person"));
    args.push(named_class_list(
        "characteristic",
        &["Symmetric", "Transitive"],
    ));
    args.push(named_class("inverse_of", "knownBy"));
    let raw = Attribute::new(
        Text::from("owl2_property"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2PropertyAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(p) => {
            assert!(matches!(&p.domain, Maybe::Some(d) if d.as_str() == "Person"));
            assert!(matches!(&p.range, Maybe::Some(r) if r.as_str() == "Person"));
            assert_eq!(p.characteristics.len(), 2);
            assert!(matches!(
                p.characteristics[0],
                Owl2Characteristic::Symmetric
            ));
            assert!(matches!(
                p.characteristics[1],
                Owl2Characteristic::Transitive
            ));
            assert!(matches!(&p.inverse_of, Maybe::Some(i) if i.as_str() == "knownBy"));
        }
        Maybe::None => panic!("full @owl2_property form must parse"),
    }
}

#[test]
fn owl2_property_requires_domain_and_range() {
    // domain only — should fail.
    let mut args: List<Expr> = List::new();
    args.push(named_class("domain", "Person"));
    let raw = Attribute::new(
        Text::from("owl2_property"),
        Maybe::Some(args),
        Span::default(),
    );
    assert!(matches!(
        Owl2PropertyAttr::from_attribute(&raw),
        Maybe::None
    ));
}

#[test]
fn owl2_property_rejects_unknown_named_arg() {
    let mut args: List<Expr> = List::new();
    args.push(named_class("domain", "Person"));
    args.push(named_class("range", "Person"));
    args.push(named_string("typo_key", "something")); // unknown key
    let raw = Attribute::new(
        Text::from("owl2_property"),
        Maybe::Some(args),
        Span::default(),
    );
    assert!(matches!(
        Owl2PropertyAttr::from_attribute(&raw),
        Maybe::None
    ));
}

// ----- @owl2_equivalent_class -----

#[test]
fn owl2_equivalent_class_parses_class_path() {
    let mut args: List<Expr> = List::new();
    args.push(class_ident("HumanBeing"));
    let raw = Attribute::new(
        Text::from("owl2_equivalent_class"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2EquivalentClassAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(eq) => assert_eq!(eq.equivalent_to.as_str(), "HumanBeing"),
        Maybe::None => panic!("@owl2_equivalent_class(HumanBeing) must parse"),
    }
}

// ----- @owl2_has_key -----

#[test]
fn owl2_has_key_accepts_property_list() {
    let mut args: List<Expr> = List::new();
    args.push(class_ident("ssn"));
    args.push(class_ident("birth_date"));
    let raw = Attribute::new(
        Text::from("owl2_has_key"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2HasKeyAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(k) => {
            assert_eq!(k.key_properties.len(), 2);
            assert_eq!(k.key_properties[0].as_str(), "ssn");
            assert_eq!(k.key_properties[1].as_str(), "birth_date");
        }
        Maybe::None => panic!("@owl2_has_key(ssn, birth_date) must parse"),
    }
}

#[test]
fn owl2_has_key_accepts_bracketed_form() {
    let mut args: List<Expr> = List::new();
    args.push(class_array(&["isbn", "edition"]));
    let raw = Attribute::new(
        Text::from("owl2_has_key"),
        Maybe::Some(args),
        Span::default(),
    );
    let typed = Owl2HasKeyAttr::from_attribute(&raw);
    match typed {
        Maybe::Some(k) => assert_eq!(k.key_properties.len(), 2),
        Maybe::None => panic!("@owl2_has_key([isbn, edition]) must parse"),
    }
}

// ----- Reject wrong attribute name across the family -----

#[test]
fn owl2_attrs_reject_wrong_attribute_name() {
    let raw = Attribute::new(Text::from("inline"), Maybe::None, Span::default());
    assert!(matches!(Owl2ClassAttr::from_attribute(&raw), Maybe::None));
    assert!(matches!(
        Owl2SubClassOfAttr::from_attribute(&raw),
        Maybe::None
    ));
    assert!(matches!(
        Owl2DisjointWithAttr::from_attribute(&raw),
        Maybe::None
    ));
    assert!(matches!(
        Owl2CharacteristicAttr::from_attribute(&raw),
        Maybe::None
    ));
    assert!(matches!(
        Owl2PropertyAttr::from_attribute(&raw),
        Maybe::None
    ));
    assert!(matches!(
        Owl2EquivalentClassAttr::from_attribute(&raw),
        Maybe::None
    ));
    assert!(matches!(Owl2HasKeyAttr::from_attribute(&raw), Maybe::None));
}

// ============================================================================
// QUANTITATIVE TYPE THEORY (Atkey QTT) — Phase 3 C5 V1
// ============================================================================
//

// Round-trip tests for QuantityAttr. The attribute accepts five
// equivalent surface shapes for each of the three quantities (`0`,
// `1`, `omega`), all canonicalised to the `Quantity` enum. Rejection
// paths are covered explicitly — silent acceptance of a typo would
// let a faulty linearity declaration compile.

fn build_int_quantity_attr(n: i128) -> Attribute {
    let span = Span::default();
    let lit = Literal::new(LiteralKind::Int(verum_ast::literal::IntLit::new(n)), span);
    let mut args: List<Expr> = List::new();
    args.push(Expr::literal(lit));
    Attribute::new(Text::from("quantity"), Maybe::Some(args), span)
}

fn build_ident_quantity_attr(name: &str) -> Attribute {
    let span = Span::default();
    let mut args: List<Expr> = List::new();
    args.push(Expr::ident(Ident::new(Text::from(name), span)));
    Attribute::new(Text::from("quantity"), Maybe::Some(args), span)
}

#[test]
fn quantity_attr_accepts_zero_via_int_literal() {
    let raw = build_int_quantity_attr(0);
    match QuantityAttr::from_attribute(&raw) {
        Maybe::Some(q) => assert_eq!(q.quantity, Quantity::Zero),
        Maybe::None => panic!("@quantity(0) must parse"),
    }
}

#[test]
fn quantity_attr_accepts_one_via_int_literal() {
    let raw = build_int_quantity_attr(1);
    match QuantityAttr::from_attribute(&raw) {
        Maybe::Some(q) => assert_eq!(q.quantity, Quantity::One),
        Maybe::None => panic!("@quantity(1) must parse"),
    }
}

#[test]
fn quantity_attr_accepts_omega_via_path() {
    let raw = build_ident_quantity_attr("omega");
    match QuantityAttr::from_attribute(&raw) {
        Maybe::Some(q) => assert_eq!(q.quantity, Quantity::Many),
        Maybe::None => panic!("@quantity(omega) must parse"),
    }
}

#[test]
fn quantity_attr_accepts_keyword_aliases() {
    for &name in &["zero", "linear", "many", "unrestricted", "erased"] {
        let raw = build_ident_quantity_attr(name);
        let parsed = QuantityAttr::from_attribute(&raw);
        assert!(
            matches!(parsed, Maybe::Some(_)),
            "alias '{name}' must parse"
        );
    }
}

#[test]
fn quantity_attr_rejects_invalid_quantity() {
    let raw = build_int_quantity_attr(2);
    assert!(matches!(QuantityAttr::from_attribute(&raw), Maybe::None));

    let bad_name = build_ident_quantity_attr("affine");
    assert!(matches!(
        QuantityAttr::from_attribute(&bad_name),
        Maybe::None
    ));
}

#[test]
fn quantity_attr_rejects_wrong_attribute_name() {
    let raw = Attribute::new(Text::from("inline"), Maybe::None, Span::default());
    assert!(matches!(QuantityAttr::from_attribute(&raw), Maybe::None));
}

#[test]
fn quantity_attr_rejects_missing_args() {
    let raw = Attribute::new(Text::from("quantity"), Maybe::None, Span::default());
    assert!(matches!(QuantityAttr::from_attribute(&raw), Maybe::None));
}

#[test]
fn quantity_predicates_partition_correctly() {
    assert!(Quantity::Zero.is_finite() && Quantity::Zero.is_erased());
    assert!(Quantity::One.is_finite() && Quantity::One.is_linear());
    assert!(!Quantity::Many.is_finite() && !Quantity::Many.is_linear());

    assert_eq!(Quantity::default(), Quantity::Many);

    assert_eq!(Quantity::Zero.surface_glyph(), "0");
    assert_eq!(Quantity::One.surface_glyph(), "1");
    assert_eq!(Quantity::Many.surface_glyph(), "ω");
}

#[test]
fn quantity_attr_display_round_trip() {
    let q = QuantityAttr::new(Quantity::One, Span::default());
    assert_eq!(format!("{}", q), "@quantity(1)");

    let q = QuantityAttr::new(Quantity::Many, Span::default());
    assert_eq!(format!("{}", q), "@quantity(omega)");
}

// =============================================================================
// @accessibility(λ) typed attribute (item 4)
// =============================================================================

mod accessibility_tests {
    use super::*;
    use verum_ast::attr::AccessibilityAttr;
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::{Literal, LiteralKind, StringLit};
    use verum_ast::ty::PathSegment;

    fn make_path_arg(name: &str) -> verum_ast::expr::Expr {
        let mut segs: List<PathSegment> = List::new();
        segs.push(PathSegment::Name(verum_ast::Ident {
            name: Text::from(name),
            span: Span::default(),
        }));
        verum_ast::expr::Expr::new(
            ExprKind::Path(verum_ast::ty::Path::new(segs, Span::default())),
            Span::default(),
        )
    }

    fn make_text_arg(s: &str) -> verum_ast::expr::Expr {
        verum_ast::expr::Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Text(StringLit::Regular(Text::from(s))),
                span: Span::default(),
            }),
            Span::default(),
        )
    }

    fn make_int_arg(n: i64) -> verum_ast::expr::Expr {
        verum_ast::expr::Expr::new(
            ExprKind::Literal(Literal::int(n as i128, Span::default())),
            Span::default(),
        )
    }

    fn build_attr(args: Vec<verum_ast::expr::Expr>) -> Attribute {
        let mut arg_list: List<verum_ast::expr::Expr> = List::new();
        for a in args {
            arg_list.push(a);
        }
        Attribute {
            name: Text::from("accessibility"),
            args: Maybe::Some(arg_list),
            span: Span::default(),
        }
    }

    #[test]
    fn canonicalise_omega() {
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("omega").as_deref(),
            Some("omega"),
        );
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("OMEGA").as_deref(),
            Some("omega"),
        );
    }

    #[test]
    fn canonicalise_subscripted_omega() {
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("omega_1").as_deref(),
            Some("omega_1"),
        );
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("omega_42").as_deref(),
            Some("omega_42"),
        );
    }

    #[test]
    fn canonicalise_omega_plus_n() {
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("omega+1").as_deref(),
            Some("omega+1"),
        );
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("omega+5").as_deref(),
            Some("omega+5"),
        );
    }

    #[test]
    fn canonicalise_subscripted_omega_plus_n() {
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("omega_1+1").as_deref(),
            Some("omega_1+1"),
        );
    }

    #[test]
    fn canonicalise_finite_cardinal() {
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("0").as_deref(),
            Some("0"),
        );
        assert_eq!(
            AccessibilityAttr::canonicalise_lambda("42").as_deref(),
            Some("42"),
        );
    }

    #[test]
    fn canonicalise_garbage_rejected() {
        assert!(AccessibilityAttr::canonicalise_lambda("not_an_ordinal").is_none());
        assert!(AccessibilityAttr::canonicalise_lambda("omega+").is_none());
        assert!(AccessibilityAttr::canonicalise_lambda("omega_").is_none());
        assert!(AccessibilityAttr::canonicalise_lambda("").is_none());
    }

    #[test]
    fn from_attribute_path_form_omega() {
        let attr = build_attr(vec![make_path_arg("omega")]);
        match AccessibilityAttr::from_attribute(&attr) {
            Maybe::Some(a) => assert_eq!(a.lambda.as_str(), "omega"),
            Maybe::None => panic!("expected Some"),
        }
    }

    #[test]
    fn from_attribute_text_form_omega_plus_one() {
        let attr = build_attr(vec![make_text_arg("omega+1")]);
        match AccessibilityAttr::from_attribute(&attr) {
            Maybe::Some(a) => assert_eq!(a.lambda.as_str(), "omega+1"),
            Maybe::None => panic!("expected Some"),
        }
    }

    #[test]
    fn from_attribute_int_form_finite() {
        let attr = build_attr(vec![make_int_arg(42)]);
        match AccessibilityAttr::from_attribute(&attr) {
            Maybe::Some(a) => assert_eq!(a.lambda.as_str(), "42"),
            Maybe::None => panic!("expected Some"),
        }
    }

    #[test]
    fn from_attribute_wrong_name_rejected() {
        let mut arg_list: List<verum_ast::expr::Expr> = List::new();
        arg_list.push(make_path_arg("omega"));
        let attr = Attribute {
            name: Text::from("accessibility_typo"),
            args: Maybe::Some(arg_list),
            span: Span::default(),
        };
        assert!(matches!(
            AccessibilityAttr::from_attribute(&attr),
            Maybe::None
        ));
    }

    #[test]
    fn from_attribute_no_args_rejected() {
        let attr = Attribute {
            name: Text::from("accessibility"),
            args: Maybe::Some(List::new()),
            span: Span::default(),
        };
        assert!(matches!(
            AccessibilityAttr::from_attribute(&attr),
            Maybe::None
        ));
    }

    #[test]
    fn from_attribute_two_args_rejected() {
        let attr = build_attr(vec![make_path_arg("omega"), make_path_arg("extra")]);
        assert!(matches!(
            AccessibilityAttr::from_attribute(&attr),
            Maybe::None
        ));
    }

    #[test]
    fn from_attribute_garbage_path_rejected() {
        let attr = build_attr(vec![make_path_arg("not_an_ordinal")]);
        assert!(matches!(
            AccessibilityAttr::from_attribute(&attr),
            Maybe::None
        ));
    }

    #[test]
    fn display_round_trip_path_form() {
        let attr = AccessibilityAttr::new(Text::from("omega"), Span::default());
        assert_eq!(format!("{}", attr), "@accessibility(omega)");
    }

    #[test]
    fn display_round_trip_subscripted() {
        let attr = AccessibilityAttr::new(Text::from("omega_1"), Span::default());
        assert_eq!(format!("{}", attr), "@accessibility(omega_1)");
    }

    #[test]
    fn spanned_returns_attribute_span() {
        let span = Span::default();
        let attr = AccessibilityAttr::new(Text::from("omega"), span);
        assert_eq!(Spanned::span(&attr), span);
    }
}

/// Drift-pin tests for the small attribute enums consolidated under the
/// shared `meta() -> XMeta` pattern. Each pin closes one of these
/// failure modes:
///
///   * Round-trip drift: `from_str(x.as_str()) != Some(x)` — exposed
///     by the `InlineMode::from_str("suggest")` defect (returned `None`
///     even though `as_str(Suggest) == "suggest"`).
///   * Variant coverage drift: a new variant is added without the
///     parallel `from_str` / `as_str` arm — caught by `ALL` having to
///     stay consistent with the `meta()` match.
///   * Name uniqueness drift: two variants picking up the same canonical
///     string would silently make the parser non-injective.
///
/// All pins are pure `const fn` projections plus byte-equality checks,
/// so they cost nothing at runtime and protect the surface from
/// hand-edit drift between sibling match arms.
#[cfg(test)]
mod meta_consolidation_pins {
    use verum_ast::attr::*;
    use verum_common::{List, Maybe};

    /// Generic round-trip + uniqueness pin used by every consolidated
    /// enum. We keep the helper inline (rather than a macro) so each
    /// pin is greppable by enum name.
    fn assert_round_trip_unique<const N: usize>(
        name: &'static str,
        all: [&'static str; N],
        parse: impl Fn(&str) -> Maybe<&'static str>,
    ) {
        // Round trip: every canonical name parses back to itself.
        for s in all.iter() {
            match parse(s) {
                Maybe::Some(round) => assert_eq!(
                    round, *s,
                    "{}: round-trip drift on '{}'",
                    name, s
                ),
                Maybe::None => panic!("{}: from_str dropped canonical '{}'", name, s),
            }
        }
        // Uniqueness: the canonical-name list has no duplicates.
        let mut seen: List<&'static str> = List::new();
        for s in all.iter() {
            assert!(
                !seen.iter().any(|prev| prev == s),
                "{}: duplicate canonical name '{}'",
                name,
                s
            );
            seen.push(*s);
        }
        // Negative: an obviously-not-a-variant string returns None.
        assert!(
            matches!(parse("__not_a_variant_string__"), Maybe::None),
            "{}: from_str accepted a bogus name",
            name
        );
    }

    #[test]
    fn optimization_level_round_trip_unique() {
        let all = [
            OptimizationLevel::None.as_str(),
            OptimizationLevel::Size.as_str(),
            OptimizationLevel::Speed.as_str(),
            OptimizationLevel::Balanced.as_str(),
        ];
        assert_eq!(OptimizationLevel::ALL.len(), all.len());
        assert_round_trip_unique("OptimizationLevel", all, |s| {
            match OptimizationLevel::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
    }

    #[test]
    fn vectorize_mode_round_trip_unique_and_empty_alias_preserved() {
        let all = [
            VectorizeMode::Auto.as_str(),
            VectorizeMode::Force.as_str(),
            VectorizeMode::Prefer.as_str(),
            VectorizeMode::Never.as_str(),
        ];
        assert_eq!(VectorizeMode::ALL.len(), all.len());
        assert_round_trip_unique("VectorizeMode", all, |s| {
            match VectorizeMode::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
        // `""` is a legacy bare-attribute alias for Auto. Pin it so the
        // consolidation doesn't silently drop it.
        assert_eq!(VectorizeMode::from_str(""), Maybe::Some(VectorizeMode::Auto));
    }

    #[test]
    fn prefetch_access_round_trip_unique() {
        let all = [
            PrefetchAccess::Read.as_str(),
            PrefetchAccess::Write.as_str(),
        ];
        assert_eq!(PrefetchAccess::ALL.len(), all.len());
        assert_round_trip_unique("PrefetchAccess", all, |s| {
            match PrefetchAccess::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
    }

    #[test]
    fn deterministic_fp_strictness_round_trip_unique_and_is_strict() {
        let all = [
            DeterministicFpStrictness::Warn.as_str(),
            DeterministicFpStrictness::Strict.as_str(),
        ];
        assert_eq!(DeterministicFpStrictness::ALL.len(), all.len());
        assert_round_trip_unique("DeterministicFpStrictness", all, |s| {
            match DeterministicFpStrictness::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
        // is_strict agrees with the dedicated variant.
        for v in DeterministicFpStrictness::ALL {
            assert_eq!(v.is_strict(), *v == DeterministicFpStrictness::Strict);
        }
    }

    #[test]
    fn const_eval_mode_round_trip_unique() {
        let all = [
            ConstEvalMode::Eval.as_str(),
            ConstEvalMode::Fold.as_str(),
            ConstEvalMode::Propagate.as_str(),
        ];
        assert_eq!(ConstEvalMode::ALL.len(), all.len());
        assert_round_trip_unique("ConstEvalMode", all, |s| {
            match ConstEvalMode::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
    }

    #[test]
    fn lto_mode_round_trip_unique() {
        let all = [
            LtoMode::None.as_str(),
            LtoMode::Always.as_str(),
            LtoMode::Thin.as_str(),
        ];
        assert_eq!(LtoMode::ALL.len(), all.len());
        assert_round_trip_unique("LtoMode", all, |s| match LtoMode::from_str(s) {
            Maybe::Some(v) => Maybe::Some(v.as_str()),
            Maybe::None => Maybe::None,
        });
    }

    #[test]
    fn symbol_visibility_round_trip_unique() {
        let all = [
            SymbolVisibility::Hidden.as_str(),
            SymbolVisibility::Default.as_str(),
            SymbolVisibility::Protected.as_str(),
        ];
        assert_eq!(SymbolVisibility::ALL.len(), all.len());
        assert_round_trip_unique("SymbolVisibility", all, |s| {
            match SymbolVisibility::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
    }

    #[test]
    fn linkage_kind_round_trip_unique_and_multidef_classification() {
        let all = [
            LinkageKind::External.as_str(),
            LinkageKind::Internal.as_str(),
            LinkageKind::Private.as_str(),
            LinkageKind::Weak.as_str(),
            LinkageKind::Linkonce.as_str(),
            LinkageKind::LinkonceOdr.as_str(),
            LinkageKind::Common.as_str(),
            LinkageKind::AvailableExternally.as_str(),
        ];
        assert_eq!(LinkageKind::ALL.len(), all.len());
        assert_round_trip_unique("LinkageKind", all, |s| {
            match LinkageKind::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
        // Multi-definition classification — pinned so adding a new
        // linkage kind forces a deliberate decision in `meta()`.
        for v in LinkageKind::ALL {
            let expected = matches!(
                v,
                LinkageKind::Weak
                    | LinkageKind::Linkonce
                    | LinkageKind::LinkonceOdr
                    | LinkageKind::Common
            );
            assert_eq!(
                v.allows_multiple_definitions(),
                expected,
                "LinkageKind::{:?}: meta-derived allows_multiple_definitions disagrees with reference matches!",
                v
            );
        }
    }

    #[test]
    fn access_pattern_round_trip_unique() {
        let all = [
            AccessPattern::Sequential.as_str(),
            AccessPattern::Random.as_str(),
            AccessPattern::Streaming.as_str(),
        ];
        assert_eq!(AccessPattern::ALL.len(), all.len());
        assert_round_trip_unique("AccessPattern", all, |s| {
            match AccessPattern::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
    }

    #[test]
    fn reduction_op_round_trip_unique_and_kind_partition() {
        let all = [
            ReductionOp::Add.as_str(),
            ReductionOp::Multiply.as_str(),
            ReductionOp::Min.as_str(),
            ReductionOp::Max.as_str(),
            ReductionOp::BitAnd.as_str(),
            ReductionOp::BitOr.as_str(),
            ReductionOp::BitXor.as_str(),
            ReductionOp::LogicAnd.as_str(),
            ReductionOp::LogicOr.as_str(),
        ];
        assert_eq!(ReductionOp::ALL.len(), 9);
        assert_round_trip_unique("ReductionOp", all, |s| match ReductionOp::from_str(s) {
            Maybe::Some(v) => Maybe::Some(v.as_str()),
            Maybe::None => Maybe::None,
        });

        // Operator forms also round-trip.
        for v in ReductionOp::ALL {
            let op = v.operator();
            assert_eq!(
                ReductionOp::from_str(op),
                Maybe::Some(*v),
                "ReductionOp::{:?}: operator '{}' must parse back",
                v,
                op
            );
        }
        // Legacy `mul` alias preserved.
        assert_eq!(ReductionOp::from_str("mul"), Maybe::Some(ReductionOp::Multiply));

        // Kind partition: {Arithmetic, Bitwise, Logical} cover ALL.
        let mut arithmetic = 0;
        let mut bitwise = 0;
        let mut logical = 0;
        for v in ReductionOp::ALL {
            match v.kind() {
                ReductionOpKind::Arithmetic => arithmetic += 1,
                ReductionOpKind::Bitwise => bitwise += 1,
                ReductionOpKind::Logical => logical += 1,
            }
        }
        assert_eq!(arithmetic, 4, "Arithmetic: Add/Multiply/Min/Max");
        assert_eq!(bitwise, 3, "Bitwise: BitAnd/BitOr/BitXor");
        assert_eq!(logical, 2, "Logical: LogicAnd/LogicOr");

        // Spot-pin a few canonical names — multiply not "mul".
        assert_eq!(ReductionOp::Multiply.as_str(), "multiply");
        assert_eq!(ReductionOp::BitXor.as_str(), "bitxor");
        assert_eq!(ReductionOp::LogicAnd.as_str(), "and");
    }

    #[test]
    fn injection_scope_round_trip_unique_and_lifetime_hierarchy() {
        let all = [
            InjectionScope::Singleton.as_str(),
            InjectionScope::Request.as_str(),
            InjectionScope::Transient.as_str(),
        ];
        assert_eq!(InjectionScope::ALL.len(), 3);
        assert_round_trip_unique("InjectionScope", all, |s| {
            match InjectionScope::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });
        // Lowercase aliases parse to the same variants.
        assert_eq!(
            InjectionScope::from_str("singleton"),
            Maybe::Some(InjectionScope::Singleton)
        );
        assert_eq!(
            InjectionScope::from_str("request"),
            Maybe::Some(InjectionScope::Request)
        );
        assert_eq!(
            InjectionScope::from_str("transient"),
            Maybe::Some(InjectionScope::Transient)
        );

        // Lifetime ranks are dense, strictly monotone in declaration
        // order: Singleton=0 (longest), Transient=2 (shortest).
        for (i, v) in InjectionScope::ALL.iter().enumerate() {
            assert_eq!(
                v.lifetime_rank() as usize,
                i,
                "InjectionScope::{:?}: rank drift at {}",
                v,
                i
            );
        }

        // can_depend_on agrees with the legacy 9-arm match exactly.
        // Reference table: rows = self, cols = other.
        //              Singleton  Request  Transient
        //   Singleton  true       false    false
        //   Request    true       true     false
        //   Transient  true       true     true
        let table: [[bool; 3]; 3] = [
            [true, false, false],
            [true, true, false],
            [true, true, true],
        ];
        for (i, a) in InjectionScope::ALL.iter().enumerate() {
            for (j, b) in InjectionScope::ALL.iter().enumerate() {
                assert_eq!(
                    a.can_depend_on(b),
                    table[i][j],
                    "can_depend_on drift: {:?} -> {:?}",
                    a,
                    b
                );
            }
        }

        // Self-dependency is always allowed (every scope can depend
        // on the same scope).
        for v in InjectionScope::ALL {
            assert!(
                v.can_depend_on(v),
                "scope {:?} must be self-compatible",
                v
            );
        }
    }

    #[test]
    fn access_mode_round_trip_unique_and_capability_classification() {
        let all = [
            AccessMode::ReadOnly.as_str(),
            AccessMode::WriteOnly.as_str(),
            AccessMode::ReadWrite.as_str(),
            AccessMode::WriteOneToClear.as_str(),
            AccessMode::WriteOneToSet.as_str(),
            AccessMode::Reserved.as_str(),
        ];
        assert_eq!(AccessMode::ALL.len(), 6);
        assert_round_trip_unique("AccessMode", all, |s| match AccessMode::from_str(s) {
            Maybe::Some(v) => Maybe::Some(v.as_str()),
            Maybe::None => Maybe::None,
        });
        // snake_case alias parses to the same variant.
        for v in AccessMode::ALL {
            let snake = v.meta().snake_name;
            assert_eq!(
                AccessMode::from_str(snake),
                Maybe::Some(*v),
                "AccessMode::{:?}: snake alias '{}' must parse",
                v,
                snake
            );
        }

        // Capability classification — meta-derived projections agree
        // with hand-written reference matches!.
        for v in AccessMode::ALL {
            let expected_read = matches!(
                v,
                AccessMode::ReadOnly
                    | AccessMode::ReadWrite
                    | AccessMode::WriteOneToClear
                    | AccessMode::WriteOneToSet
            );
            let expected_write = matches!(
                v,
                AccessMode::WriteOnly
                    | AccessMode::ReadWrite
                    | AccessMode::WriteOneToClear
                    | AccessMode::WriteOneToSet
            );
            let expected_modify = matches!(
                v,
                AccessMode::WriteOneToClear | AccessMode::WriteOneToSet
            );
            assert_eq!(v.can_read(), expected_read, "AccessMode::{:?}: can_read", v);
            assert_eq!(
                v.can_write(),
                expected_write,
                "AccessMode::{:?}: can_write",
                v
            );
            assert_eq!(
                v.is_write_modify(),
                expected_modify,
                "AccessMode::{:?}: is_write_modify",
                v
            );
            // Cross-cutting: write-modify ⇒ both can_read and can_write.
            if v.is_write_modify() {
                assert!(
                    v.can_read() && v.can_write(),
                    "AccessMode::{:?}: write-modify must allow both read and write",
                    v
                );
            }
        }
        // Reserved is the only no-access variant.
        for v in AccessMode::ALL {
            let no_access = !v.can_read() && !v.can_write();
            assert_eq!(
                no_access,
                *v == AccessMode::Reserved,
                "Reserved is the only no-access variant"
            );
        }
    }

    #[test]
    fn interrupt_kind_round_trip_unique_and_classification_and_aliases() {
        let all = [
            InterruptKind::Regular.as_str(),
            InterruptKind::NMI.as_str(),
            InterruptKind::Fast.as_str(),
            InterruptKind::Exception.as_str(),
            InterruptKind::Trap.as_str(),
            InterruptKind::Reset.as_str(),
        ];
        assert_eq!(InterruptKind::ALL.len(), 6);
        assert_round_trip_unique("InterruptKind", all, |s| {
            match InterruptKind::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });

        // Architecture-specific aliases all parse to the right variant.
        let alias_pins: &[(&str, InterruptKind)] = &[
            ("Regular", InterruptKind::Regular),
            ("irq", InterruptKind::Regular),
            ("IRQ", InterruptKind::Regular),
            ("NMI", InterruptKind::NMI),
            ("Fast", InterruptKind::Fast),
            ("fiq", InterruptKind::Fast),
            ("FIQ", InterruptKind::Fast),
            ("Exception", InterruptKind::Exception),
            ("Trap", InterruptKind::Trap),
            ("syscall", InterruptKind::Trap),
            ("svc", InterruptKind::Trap),
            ("SVC", InterruptKind::Trap),
            ("Reset", InterruptKind::Reset),
        ];
        for (alias, expected) in alias_pins {
            assert_eq!(
                InterruptKind::from_str(alias),
                Maybe::Some(*expected),
                "InterruptKind alias '{}' must parse to {:?}",
                alias,
                expected
            );
        }

        // Classification — meta-derived projections agree with
        // hand-written reference matches!.
        for v in InterruptKind::ALL {
            let expected_maskable =
                matches!(v, InterruptKind::Regular | InterruptKind::Fast);
            let expected_special_stack = matches!(
                v,
                InterruptKind::NMI | InterruptKind::Reset | InterruptKind::Exception
            );
            assert_eq!(
                v.is_maskable(),
                expected_maskable,
                "InterruptKind::{:?}: is_maskable",
                v
            );
            assert_eq!(
                v.needs_special_stack(),
                expected_special_stack,
                "InterruptKind::{:?}: needs_special_stack",
                v
            );
            // Cross-cutting: needs_special_stack ⇒ ¬is_maskable
            // (every special-stack variant is non-maskable).
            if v.needs_special_stack() {
                assert!(
                    !v.is_maskable(),
                    "InterruptKind::{:?}: special-stack variants must be non-maskable",
                    v
                );
            }
        }
    }

    #[test]
    fn verification_mode_round_trip_unique_and_rank_strict_monotone() {
        let all = [
            VerificationMode::Runtime.as_str(),
            VerificationMode::Static.as_str(),
            VerificationMode::Fast.as_str(),
            VerificationMode::ComplexityTyped.as_str(),
            VerificationMode::Formal.as_str(),
            VerificationMode::Proof.as_str(),
            VerificationMode::Thorough.as_str(),
            VerificationMode::Reliable.as_str(),
            VerificationMode::Certified.as_str(),
            VerificationMode::CoherentStatic.as_str(),
            VerificationMode::CoherentRuntime.as_str(),
            VerificationMode::Coherent.as_str(),
            VerificationMode::Synthesize.as_str(),
            VerificationMode::Assume.as_str(),
        ];
        assert_eq!(VerificationMode::ALL.len(), 14);
        assert_eq!(VerificationMode::ALL.len(), all.len());
        assert_round_trip_unique("VerificationMode", all, |s| {
            match VerificationMode::from_str(s) {
                Maybe::Some(v) => Maybe::Some(v.as_str()),
                Maybe::None => Maybe::None,
            }
        });

        // ν-ladder: ranks are exactly 0..=13 and strictly monotone in
        // declaration order. Adding a new variant without picking a
        // rank slot fails this pin.
        for (i, v) in VerificationMode::ALL.iter().enumerate() {
            assert_eq!(
                v.rank() as usize,
                i,
                "VerificationMode::{:?}: rank drift at position {}",
                v,
                i
            );
        }
        // Strict monotonicity (redundant with the dense check above
        // but pins the ordering contract on its own).
        for w in VerificationMode::ALL.windows(2) {
            assert!(
                w[0].rank() < w[1].rank(),
                "VerificationMode rank not strictly monotone at {:?} -> {:?}",
                w[0],
                w[1]
            );
        }

        // Classification pins — every variant's meta-derived
        // projection must agree with its hand-written reference set.
        // Adding a new variant forces deliberate classification in
        // `meta()` instead of silent drift.
        for v in VerificationMode::ALL {
            let expected_coherent = matches!(
                v,
                VerificationMode::CoherentStatic
                    | VerificationMode::CoherentRuntime
                    | VerificationMode::Coherent
            );
            assert_eq!(
                v.is_coherent(),
                expected_coherent,
                "VerificationMode::{:?}: is_coherent drift",
                v
            );

            let expected_smt = matches!(
                v,
                VerificationMode::Fast
                    | VerificationMode::Formal
                    | VerificationMode::Thorough
                    | VerificationMode::Reliable
                    | VerificationMode::Certified
                    | VerificationMode::CoherentStatic
                    | VerificationMode::CoherentRuntime
                    | VerificationMode::Coherent
                    | VerificationMode::Synthesize
            );
            assert_eq!(
                v.requires_smt(),
                expected_smt,
                "VerificationMode::{:?}: requires_smt drift",
                v
            );

            let expected_runtime = matches!(
                v,
                VerificationMode::Runtime | VerificationMode::CoherentRuntime
            );
            assert_eq!(
                v.has_runtime_component(),
                expected_runtime,
                "VerificationMode::{:?}: has_runtime_component drift",
                v
            );

            assert_eq!(
                v.is_assume(),
                *v == VerificationMode::Assume,
                "VerificationMode::{:?}: is_assume drift",
                v
            );
        }

        // Cross-cutting invariants:
        //
        //   1. Coherent ⇒ requires_smt (every Coherent* mode runs an
        //      SMT-backed α-check, even the runtime-monitor variant).
        //   2. Assume ⇒ ¬requires_smt ∧ ¬is_coherent (the escape
        //      hatch can never imply verification work).
        //   3. Synthesize is the highest non-Assume rank — pin so
        //      future ladder additions either bump Assume's sentinel
        //      slot or land below Synthesize, never silently above.
        for v in VerificationMode::ALL {
            if v.is_coherent() {
                assert!(
                    v.requires_smt(),
                    "VerificationMode::{:?}: is_coherent ⇒ requires_smt invariant violated",
                    v
                );
            }
            if v.is_assume() {
                assert!(
                    !v.requires_smt() && !v.is_coherent(),
                    "VerificationMode::Assume must not require SMT or be coherent"
                );
            }
        }
        assert_eq!(
            VerificationMode::Synthesize.rank() + 1,
            VerificationMode::Assume.rank(),
            "Assume must immediately follow Synthesize in the ladder; \
             new ladder variants must land before Synthesize"
        );

        // Spot-pin the canonical wire form on a tricky multi-word
        // variant — exposes any underscore drift early.
        assert_eq!(VerificationMode::ComplexityTyped.as_str(), "complexity_typed");
        assert_eq!(VerificationMode::CoherentStatic.as_str(), "coherent_static");
        assert_eq!(
            VerificationMode::CoherentRuntime.as_str(),
            "coherent_runtime"
        );
    }

    #[test]
    fn repr_round_trip_unique_and_simd_classification_and_c_alias() {
        let all = [
            Repr::Packed.as_str(),
            Repr::C.as_str(),
            Repr::CacheOptimal.as_str(),
            Repr::Transparent.as_str(),
            Repr::Simd.as_str(),
            Repr::SimdMask.as_str(),
        ];
        assert_eq!(Repr::ALL.len(), all.len());
        assert_round_trip_unique("Repr", all, |s| match Repr::from_str(s) {
            Maybe::Some(v) => Maybe::Some(v.as_str()),
            Maybe::None => Maybe::None,
        });
        // Tolerant `"c"` alias is preserved (canonical is `"C"`).
        assert_eq!(Repr::from_str("c"), Maybe::Some(Repr::C));
        // SIMD classification — meta-derived projections agree with
        // hand-written reference matches!.
        for v in Repr::ALL {
            let expected_simd = matches!(v, Repr::Simd | Repr::SimdMask);
            let expected_simd_mask = matches!(v, Repr::SimdMask);
            assert_eq!(v.is_simd(), expected_simd, "Repr::{:?}: is_simd drift", v);
            assert_eq!(
                v.is_simd_mask(),
                expected_simd_mask,
                "Repr::{:?}: is_simd_mask drift",
                v
            );
        }
    }
}
