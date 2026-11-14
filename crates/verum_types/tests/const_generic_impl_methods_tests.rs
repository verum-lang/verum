//! Const-generic inherent methods are selected by their complete source declaration.
//! The removed slot-only matcher erased value domains and did not retain a
//! selected owner. These source calls preserve the actual language controls.
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

#[test]
fn const_receiver_rejects_the_original_foreign_text_argument() {
    let source = "type Buf<const SIZE:Int> is { top:Int }; implement<const SIZE:Int> Buf<SIZE> { public fn capacity(&self)->Int { SIZE } } fn probe(value:Buf<Text>)->Int { value.capacity() }";
    let module = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    let mut errors = checker.prepare_module_declarations(&module);
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(error); }
    }
    errors.extend(checker.take_deferred_errors());
    assert!(!errors.is_empty(), "foreign-base receiver arg must still be rejected");
    assert!(errors.iter().any(|error| format!("{error:?}").contains("Text")), "{errors:?}");
}

// ─── Behavioral: full typecheck of the defect's repro shapes ────────────────

fn typecheck_ok(code: &str, label: &str) {
    let mut parser = Parser::new(code);
    let module = parser.parse_module().expect("parse should succeed");
    let mut checker = TypeChecker::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(td) = &item.kind {
            let _ = checker.register_type_declaration(td);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Impl(impl_block) = &item.kind {
            let _ = checker.register_impl_block(impl_block);
        }
    }
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            let _ = checker.register_function_signature(f);
        }
    }
    let errs: Vec<String> = module
        .items
        .iter()
        .filter_map(|item| checker.check_item(item).err().map(|e| format!("{:?}", e)))
        .collect();
    assert!(errs.is_empty(), "{}: {:?}", label, errs);
}

/// Direct instantiation: methods of a `implement<const SIZE: Int>` block
/// must resolve on `Buf<256>` receivers.
#[test]
fn const_generic_impl_methods_resolve_on_direct_instantiation() {
    typecheck_ok(
        r#"
type Buf<const SIZE: Int> is {
    top: Int,
};

implement<const SIZE: Int> Buf<SIZE> {
    public fn new() -> Self {
        Self { top: 0 }
    }
    public fn capacity(&self) -> Int {
        SIZE
    }
    public fn used(&self) -> Int {
        self.top
    }
}

fn main() {
    let mut a = Buf<256>.new();
    let c = a.capacity();
    let u = a.used();
}
"#,
        "direct const-generic instantiation",
    );
}

/// Alias expansion: `type Tiny is Buf<1024>;` — the alias preserves the
/// const arg as `Meta{value: Some(1024)}`, the exact shape the pattern
/// gate rejected pre-fix (E400 `no method named `capacity``).
#[test]
fn const_generic_impl_methods_resolve_through_alias() {
    typecheck_ok(
        r#"
type Buf<const SIZE: Int> is {
    top: Int,
};

implement<const SIZE: Int> Buf<SIZE> {
    public fn new() -> Self {
        Self { top: 0 }
    }
    public fn capacity(&self) -> Int {
        SIZE
    }
}

type Tiny is Buf<1024>;

fn main() {
    let t = Tiny.new();
    let c = t.capacity();
}
"#,
        "alias to const-generic instantiation",
    );
}

/// Multi-const-param impls (the ArenaAllocator / PoolAllocator shape).
#[test]
fn const_generic_impl_methods_resolve_with_two_const_params() {
    typecheck_ok(
        r#"
type Pool<const BLOCK_SIZE: Int, const BLOCK_COUNT: Int> is {
    used: Int,
};

implement<const BLOCK_SIZE: Int, const BLOCK_COUNT: Int> Pool<BLOCK_SIZE, BLOCK_COUNT> {
    public fn new() -> Self {
        Self { used: 0 }
    }
    public fn block_count(&self) -> Int {
        BLOCK_COUNT
    }
}

fn main() {
    let p = Pool<64, 32>.new();
    let n = p.block_count();
}
"#,
        "two const params",
    );
}
