use super::{build_aot_permission_policy, build_script_permission_policy, hash_grant_target};
use crate::script::{
    context::{ScriptContext, ScriptContextOptions},
    frontmatter::Frontmatter,
    permission_flags::PermissionFlags,
};
use std::path::Path;
use verum_vbc::interpreter::permission::{PermissionDecision, PermissionScope};

fn context(body: Option<&str>, flags: PermissionFlags) -> ScriptContext {
    let mut source = String::new();
    if let Some(body) = body {
        source.push_str("// /// script\n");
        for line in body.lines() {
            source.push_str("// ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("// ///\n");
    }
    source.push_str("fn main() {}\n");
    ScriptContext::from_source(
        Path::new("task.vr"),
        source.into_bytes(),
        &ScriptContextOptions {
            flags,
            ..Default::default()
        },
    )
    .unwrap()
}

fn assert_decision(ctx: &ScriptContext, scope: PermissionScope, target: u64, allowed: bool) {
    let interpreter = build_script_permission_policy(ctx).expect("explicit interpreter policy");
    assert_eq!(
        (interpreter.0)(scope, target) == PermissionDecision::Allow,
        allowed
    );
    let native = build_aot_permission_policy(ctx).expect("explicit AOT policy");
    let tag = scope.to_wire_tag();
    assert_eq!(
        native.is_scope_unconditionally_allowed(tag) || native.specific.contains(&(tag, target)),
        allowed
    );
}

#[test]
fn absent_permission_declarations_preserve_legacy_mode() {
    for body in [None, Some(""), Some("edition = \"2026\""), Some("[run]")] {
        let ctx = context(body, PermissionFlags::default());
        assert!(ctx.permissions.is_empty());
        assert!(ctx.enforced_permissions().is_none());
        assert!(build_script_permission_policy(&ctx).is_none());
        assert!(build_aot_permission_policy(&ctx).is_none());
    }
}

#[test]
fn explicit_empty_top_level_or_run_permissions_deny_in_both_tiers() {
    for body in ["permissions = []", "[run]\ndefault-permissions = []"] {
        let ctx = context(Some(body), PermissionFlags::default());
        assert!(ctx.enforced_permissions().unwrap().is_empty());
        for scope in [
            PermissionScope::Process,
            PermissionScope::FileSystem,
            PermissionScope::Network,
            PermissionScope::Syscall,
        ] {
            assert_decision(&ctx, scope, 0, false);
            assert_decision(&ctx, scope, hash_grant_target("ungranted"), false);
        }
    }
}

#[test]
fn deny_all_remains_explicit_with_or_without_frontmatter() {
    for body in [None, Some("permissions = [\"run\", \"net\", \"ffi\"]")] {
        let ctx = context(
            body,
            PermissionFlags {
                deny_all: true,
                allow: vec!["fs:read".into()],
                ..Default::default()
            },
        );
        assert!(ctx.enforced_permissions().unwrap().is_empty());
        assert_decision(&ctx, PermissionScope::Process, 0, false);
        assert_decision(&ctx, PermissionScope::Network, 0, false);
        assert_decision(&ctx, PermissionScope::FileSystem, 0, false);
    }
}

#[test]
fn allow_all_overrides_an_explicit_empty_declaration() {
    let ctx = context(
        Some("permissions = []"),
        PermissionFlags {
            allow_all: true,
            ..Default::default()
        },
    );
    for scope in [
        PermissionScope::Process,
        PermissionScope::FileSystem,
        PermissionScope::Network,
        PermissionScope::Syscall,
        PermissionScope::Time,
    ] {
        assert_decision(&ctx, scope, hash_grant_target("arbitrary-target"), true);
    }
}

#[test]
fn target_grants_have_matching_positive_and_negative_decisions() {
    for body in [
        Some("permissions = [\"run=verum\"]"),
        Some("[run]\ndefault-permissions = [\"run=verum\"]"),
        None,
    ] {
        let flags = if body.is_none() {
            PermissionFlags {
                allow: vec!["run=verum".into()],
                ..Default::default()
            }
        } else {
            PermissionFlags::default()
        };
        let ctx = context(body, flags);
        assert_decision(
            &ctx,
            PermissionScope::Process,
            hash_grant_target("verum"),
            true,
        );
        assert_decision(
            &ctx,
            PermissionScope::Process,
            hash_grant_target("other"),
            false,
        );
        assert_decision(&ctx, PermissionScope::Network, 0, false);
    }
}

#[test]
fn cached_binaries_cannot_cross_permission_modes_or_grants() {
    let legacy = context(None, PermissionFlags::default());
    let denied = context(
        None,
        PermissionFlags {
            deny_all: true,
            ..Default::default()
        },
    );
    let all = context(
        None,
        PermissionFlags {
            allow_all: true,
            ..Default::default()
        },
    );
    let limited = context(
        None,
        PermissionFlags {
            allow: vec!["run=verum".into()],
            ..Default::default()
        },
    );
    let keys = [
        legacy.cache_key,
        denied.cache_key,
        all.cache_key,
        limited.cache_key,
    ];
    for i in 0..keys.len() {
        for j in i + 1..keys.len() {
            assert_ne!(keys[i], keys[j]);
        }
    }
    assert_eq!(
        denied.cache_key,
        context(
            None,
            PermissionFlags {
                deny_all: true,
                ..Default::default()
            }
        )
        .cache_key
    );
}

#[test]
fn frontmatter_serialization_preserves_absent_and_explicit_empty_fields() {
    for text in [
        "",
        "permissions = []",
        "[run]",
        "[run]\ndefault-permissions = []",
    ] {
        let first: Frontmatter = toml::from_str(text).unwrap();
        let second: Frontmatter = toml::from_str(&toml::to_string(&first).unwrap()).unwrap();
        assert_eq!(first.permissions, second.permissions);
        assert_eq!(
            first.run.as_ref().map(|r| &r.default_permissions),
            second.run.as_ref().map(|r| &r.default_permissions)
        );
    }
}

#[test]
fn conflicting_policy_overrides_still_fail_before_execution() {
    assert!(
        ScriptContext::from_source(
            Path::new("task.vr"),
            b"fn main() {}".to_vec(),
            &ScriptContextOptions {
                flags: PermissionFlags {
                    allow_all: true,
                    deny_all: true,
                    ..Default::default()
                },
                ..Default::default()
            }
        )
        .is_err()
    );
}
