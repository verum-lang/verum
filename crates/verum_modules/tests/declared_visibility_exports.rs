use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_modules::{ModuleId, ModulePath, Visibility, extract_exports_from_module};

#[test]
fn restricted_declarations_keep_their_policy_at_the_module_boundary() {
    for (modifier, visibility, caller, allowed) in [
        ("public(cog)", Visibility::PublicCrate, "realm.peer", true),
        ("public(cog)", Visibility::PublicCrate, "foreign.peer", false),
        // These modifiers retain the kernel's conservative same-module policy;
        // exporting the declaration does not make them aliases for public(cog).
        ("internal", Visibility::Internal, "realm.vault", true),
        ("internal", Visibility::Internal, "realm.peer", false),
        ("internal", Visibility::Internal, "foreign.peer", false),
        ("protected", Visibility::Protected, "realm.vault", true),
        ("protected", Visibility::Protected, "realm.vault.child", false),
        ("protected", Visibility::Protected, "realm.peer", false),
        ("public(super)", Visibility::PublicSuper, "realm", true),
        ("public(super)", Visibility::PublicSuper, "foreign", false),
    ] {
        let source = format!("module realm.vault;
            {modifier} fn inspect() -> Int {{ 7 }}
            {modifier} type Token is (Int);
            {modifier} const LIMIT: Int = 7;
            fn hidden() -> Int {{ 9 }}
            public fn open() -> Int {{ 1 }}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let table = extract_exports_from_module(&ast, ModuleId::new(0), &ModulePath::from_str("realm.vault")).unwrap();
        let caller = ModulePath::from_str(caller);
        let visible: List<Text> = table.visible_exports(&caller).map(|item| item.name.clone()).collect();
        for name in ["inspect", "Token", "LIMIT"] {
            let name = Text::from(name);
            assert_eq!(table.get(&name).unwrap().visibility, visibility);
            assert_eq!(table.is_visible_from_path(&name, &caller), allowed, "{modifier} {caller} {name}");
            assert_eq!(visible.contains(&name), allowed);
        }
        assert!(!table.contains("hidden"));
        assert_eq!(table.public_exports().map(|item| item.name.as_str()).collect::<List<_>>().as_slice(), &["open"]);
    }
}
