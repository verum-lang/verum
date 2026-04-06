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
use verum_common::{Maybe, Text};

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
