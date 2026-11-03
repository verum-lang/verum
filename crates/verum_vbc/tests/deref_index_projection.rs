//! Indexing must use the same declared Deref target for types and storage.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

const ENV: &str = r#"
module index_projection;
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type DerefMut is protocol { fn deref_mut(&mut self) -> &mut Self.Target; };
type Guard<T> is { marker: Int, value: T };
implement<T> Deref for Guard<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.value }
}
implement<T> DerefMut for Guard<T> {
    fn deref_mut(&mut self) -> &mut T { &mut self.value }
}
type Cell is { padding: Int, value: Int };
"#;

fn compile(body: &str) -> Result<verum_vbc::VbcModule, verum_vbc::codegen::CodegenError> {
    let source = format!("{ENV}\n{body}");
    let ast = Parser::new(&source).parse_module().expect("source grammar");
    VbcCodegen::with_config(CodegenConfig::new("index_projection")).compile_module(&ast)
}

fn value(body: &str, expected: i64) {
    execute(
        compile(body).expect("declared index projection compiles"),
        expected,
    );
}

fn execute(module: verum_vbc::VbcModule, expected: i64) {
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("deserialize");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let result = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert!(result.is_int(), "expected Int, got {result:?}");
    assert_eq!(result.as_i64(), expected);
}

#[test]
fn read_follows_the_declared_target_before_projecting_the_element_field() {
    value(
        r#"
fn probe() -> Int {
    let g: Guard<List<Cell>> = Guard { marker: 99, value: [Cell { padding: 3, value: 7 }] };
    g[0].value
}
"#,
        7,
    );
}

#[test]
fn borrowed_element_keeps_the_element_declaration() {
    value(
        r#"
fn probe() -> Int {
    let g: Guard<List<Cell>> = Guard { marker: 99, value: [Cell { padding: 3, value: 11 }] };
    let element = &g[0];
    element.value
}
"#,
        11,
    );
}

#[test]
fn writes_and_compound_writes_reach_the_same_collection() {
    value(
        r#"
fn probe() -> Int {
    let mut g: Guard<List<Int>> = Guard { marker: 99, value: [3, 5] };
    g[0] = 13;
    g[0] += 4;
    g.value[0] + g.marker
}
"#,
        116,
    );
}

#[test]
fn mutable_element_borrows_alias_the_projected_collection() {
    value(
        r#"
fn set(value: &mut Int) { *value = 19; }
fn probe() -> Int {
    let mut g: Guard<List<Int>> = Guard { marker: 99, value: [3] };
    set(&mut g[0]);
    g.value[0]
}
"#,
        19,
    );
}

#[test]
fn target_parameter_position_and_nested_wrappers_are_preserved() {
    value(
        r#"
type Pair<Noise, Item> is { noise: Noise, target: Item };
implement<Noise, Item> Deref for Pair<Noise, Item> {
    type Target = Item;
    fn deref(&self) -> &Item { &self.target }
}
fn probe() -> Int {
    let g: Pair<Bool, Guard<List<Cell>>> = Pair {
        noise: false,
        target: Guard { marker: 99, value: [Cell { padding: 3, value: 23 }] },
    };
    g[0].value
}
"#,
        23,
    );
}

#[test]
fn an_unrelated_generic_record_is_not_an_index_carrier() {
    let error = compile(
        r#"
type Heap<T> is { marker: Int, value: T };
fn probe() -> Int { let g: Heap<List<Int>> = Heap { marker: 99, value: [1] }; g[0] }
"#,
    )
    .err()
    .expect("foreign Heap spelling cannot supply an indexing declaration");
    assert!(error.to_string().contains("index"), "{error}");
}

#[test]
fn mutable_indexing_requires_a_mutable_projection() {
    let error = compile(
        r#"
type ReadOnly<T> is { value: T };
implement<T> Deref for ReadOnly<T> { type Target = T; fn deref(&self) -> &T { &self.value } }
fn probe() -> Int { let mut g: ReadOnly<List<Int>> = ReadOnly { value: [1] }; g[0] = 2; 0 }
"#,
    )
    .err()
    .expect("Deref does not authorize DerefMut");
    assert!(error.to_string().contains("index"), "{error}");
}

fn refuses(body: &str) {
    let error = compile(body).err().expect("invalid index must be rejected");
    assert!(error.to_string().contains("index"), "{error}");
}

#[test]
fn immutable_collection_reference_cannot_authorize_a_write() {
    refuses("fn probe(g: &List<Int>) { g[0] = 1; }");
}

#[test]
fn immutable_wrapper_reference_cannot_authorize_a_mutable_borrow() {
    refuses("fn probe(g: &Guard<List<Int>>) { let place = &mut g[0]; }");
}

#[test]
fn shared_ownership_does_not_authorize_mutable_deref() {
    refuses("fn probe(g: &mut Shared<List<Int>>) { g[0] = 1; }");
}

#[test]
fn an_index_method_requires_its_declared_concrete_key_type() {
    refuses(
        r#"
type Store is { value: Int };
implement Store { fn index(&self, key: Int) -> &Int { &self.value } }
fn probe() -> Int { let s: Store = Store { value: 1 }; s[true] }
"#,
    );
}

#[test]
fn borrowed_wrapper_reads_and_writes_preserve_the_receiver() {
    value(
        r#"
fn read(g: &Guard<List<Cell>>) -> Int { g[0].value }
fn write(g: &mut Guard<List<Cell>>) { g[0] = Cell { padding: 1, value: 31 }; }
fn probe() -> Int {
    let mut g: Guard<List<Cell>> = Guard { marker: 99, value: [Cell { padding: 3, value: 7 }] };
    write(&mut g);
    read(&g)
}
"#,
        31,
    );
}

#[test]
fn an_unavailable_parameter_declaration_is_not_reconstructed_from_an_erased_hint() {
    refuses(
        r#"
fn read(g: &Heap<List<Cell>>) -> Int { g[0].value }
fn probe() -> Int {
    let g: Heap<List<Cell>> = Heap.new([Cell { padding: 3, value: 37 }]);
    read(&g)
}
"#,
    );
}

#[test]
fn custom_index_methods_are_selected_before_deref() {
    value(
        r#"
type Store<T> is { marker: Int, value: T };
implement<T> Store<T> {
    fn index(&self, key: Int) -> &T { &self.value }
    fn index_mut(&mut self, key: Int) -> &mut T { &mut self.value }
}
fn probe() -> Int {
    let mut s: Store<Int> = Store { marker: 99, value: 3 };
    s[0] = 8;
    s[0] += 5;
    s[0] + s.marker
}
"#,
        112,
    );
}

#[test]
fn range_borrow_keeps_the_projected_collection_element_type() {
    value(
        r#"
fn probe() -> Int {
    let g: Guard<List<Cell>> = Guard { marker: 99, value: [Cell { padding: 3, value: 41 }] };
    let slice = &g[..];
    slice[0].value
}
"#,
        41,
    );
}

#[test]
fn inferred_collection_literals_keep_their_source_element_type() {
    value("fn probe() -> Int { let values = [3, 7]; values[1] }", 7);
}

fn with_memory_declarations(
    body: &str,
) -> Result<verum_vbc::VbcModule, verum_vbc::codegen::CodegenError> {
    let memory = Parser::new(include_str!("../../../core/base/memory.vr"))
        .parse_module()
        .expect("actual memory source grammar");
    let source = format!("module core.base.memory; {body}");
    let mut ast = Parser::new(&source)
        .parse_module()
        .expect("control grammar");
    // Use the actual nominal source declarations. The fixture exercises native
    // carrier operations without replacing their layout or inventing an alias.
    for item in memory.items {
        if matches!(&item.kind, verum_ast::ItemKind::Type(decl)
            if matches!(decl.name.name.as_str(), "Heap" | "Shared" | "SharedInner"))
        {
            ast.items.insert(0, item);
        }
    }
    VbcCodegen::with_config(CodegenConfig::new("core.base.memory")).compile_module(&ast)
}

#[test]
fn native_owned_carrier_uses_its_actual_stdlib_declaration() {
    execute(
        with_memory_declarations(
            r#"
type Cell is { padding: Int, value: Int };
fn read(g: &core.base.memory.Heap<List<Cell>>) -> Int { g[0].value }
fn probe() -> Int {
    let g: Heap<List<Cell>> = Heap.new([Cell { padding: 3, value: 37 }]);
    read(&g)
}
"#,
        )
        .expect("canonical Heap"),
        37,
    );
}

#[test]
fn actual_shared_declaration_does_not_authorize_mutable_deref() {
    let error = with_memory_declarations("fn probe(g: &mut Shared<List<Int>>) { g[0] = 1; }")
        .err()
        .expect("shared ownership grants no mutable projection");
    assert!(error.to_string().contains("index"), "{error}");
}

#[test]
fn range_index_uses_its_declared_cell_output() {
    let source = r#"
module core.base.iterator;
type Cell is { padding: Int, value: Int };
type Store is { cell: Cell };
implement Store {
    fn index(&self, range: Range<Int>) -> &Cell { &self.cell }
}
fn probe() -> Int {
    let store = Store { cell: Cell { padding: 3, value: 47 } };
    let value = store[0..1];
    value.value
}
"#;
    let mut ast = Parser::new(source).parse_module().unwrap();
    let library = include_str!("../../../core/base/iterator.vr");
    for item in Parser::new(&library).parse_module().unwrap().items {
        if matches!(&item.kind, verum_ast::ItemKind::Type(decl) if decl.name.name.as_str() == "Range")
        {
            ast.items.insert(0, item);
        }
    }
    let module =
        VbcCodegen::with_config(CodegenConfig::new("core.base.iterator").with_validation())
            .compile_module(&ast)
            .expect("custom Range index declares Cell output");
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".probe"))
        })
        .unwrap()
        .id;
    let result = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert!(result.is_int(), "{result:?}");
    assert_eq!(result.as_i64(), 47);
}

#[test]
fn range_index_value_is_not_the_reference_returned_by_its_method() {
    let source = r#"
module core.base.iterator;
type Store is { value: Int };
implement Store {
    fn index(&self, range: Range<Int>) -> &Int { &self.value }
}
fn probe() -> Int { let store = Store { value: 47 }; store[0..1] }
"#;
    let mut ast = Parser::new(source).parse_module().unwrap();
    let library = Parser::new(include_str!("../../../core/base/iterator.vr"))
        .parse_module()
        .unwrap();
    for item in library.items {
        if matches!(&item.kind, verum_ast::ItemKind::Type(decl) if decl.name.name.as_str() == "Range")
        {
            ast.items.insert(0, item);
        }
    }
    let module = VbcCodegen::with_config(CodegenConfig::new("core.base.iterator"))
        .compile_module(&ast)
        .expect("custom range output");
    execute(module, 47);
}

#[test]
fn native_field_projection_keeps_a_borrowed_carrier() {
    execute(
        with_memory_declarations(
            r#"
type Cell is { padding: Int, value: Int };
fn read(g: &core.base.memory.Heap<Cell>) -> Int { g.value }
fn probe() -> Int {
    let g: Heap<Cell> = Heap.new(Cell { padding: 3, value: 53 });
    read(&g)
}
"#,
        )
        .expect("canonical borrowed field"),
        53,
    );
}

#[test]
fn native_carrier_aliases_preserve_index_and_field_projections() {
    execute(
        with_memory_declarations(
            r#"
type Cell is { padding: Int, value: Int };
type Owner<T> is Heap<T>;
fn read(g: &Owner<List<Owner<Cell>>>) -> Int { g[0].value }
fn probe() -> Int {
    let cell: Owner<Cell> = Heap.new(Cell { padding: 3, value: 59 });
    let g: Owner<List<Owner<Cell>>> = Heap.new([cell]);
    read(&g)
}
"#,
        )
        .expect("canonical carrier alias"),
        59,
    );
}

#[test]
fn a_mutable_slice_borrow_has_one_reference_layer() {
    value(
        r#"
fn probe() -> Int {
    let mut g: Guard<List<Int>> = Guard { marker: 99, value: [3] };
    let slice = &mut g[..];
    slice[0] = 61;
    g.value[0]
}
"#,
        61,
    );
}

#[test]
fn a_local_binding_inside_a_match_owns_its_initializer_type() {
    value(
        r#"
type Presence<T> is Absent | Present(T);
fn probe() -> Int {
    let source: Presence<Guard<List<Int>>> = Presence.Present(Guard { marker: 99, value: [67] });
    match &source {
        Presence.Present(node) => { let items = &node.value; items[0] },
        Presence.Absent => 0,
    }
}
"#,
        67,
    );
}

#[test]
fn wildcard_array_parameters_keep_element_identity_without_an_extent() {
    value(
        r#"
fn element(values: [Int; _]) -> Int { values[1] }
fn probe() -> Int { element([3, 71]) }
"#,
        71,
    );
}

#[test]
fn declared_constant_extents_are_evaluated_in_their_owner_scope() {
    value(
        r#"
const COUNT: Int = 2;
fn element(values: [Int; COUNT]) -> Int { values[1] }
fn probe() -> Int { element([3, 73]) }
"#,
        73,
    );
}

#[test]
fn an_unknown_extent_is_not_replaced_by_zero() {
    refuses("fn probe(values: [Int; MISSING]) -> Int { values[0] }");
}

#[test]
fn wildcard_and_fixed_array_metadata_remain_distinct_after_serialization() {
    use verum_vbc::types::{TypeId, TypeRef};
    let module = compile(
        r#"
fn wildcard(values: [Int; _]) {}
fn three(values: [Int; 3]) {}
fn four(values: [Int; 4]) {}
"#,
    )
    .expect("source declarations");
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let parameter = |name| {
        module
            .functions
            .iter()
            .find(|f| module.get_string(f.name).is_some_and(|n| n.ends_with(name)))
            .unwrap()
            .semantic_parameter_types()
            .unwrap()[0]
            .clone()
            .unwrap()
    };
    let wildcard = parameter(".wildcard");
    let three = parameter(".three");
    let four = parameter(".four");
    assert_eq!(
        wildcard,
        TypeRef::Instantiated {
            base: TypeId::ARRAY,
            args: vec![TypeRef::Concrete(TypeId::INT)]
        }
    );
    assert_eq!(
        three,
        TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::INT)),
            length: 3
        }
    );
    assert_eq!(
        four,
        TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId::INT)),
            length: 4
        }
    );
    assert_ne!(wildcard, three);
    assert_ne!(three, four);
    assert_eq!(
        verum_vbc::type_layout::query(
            &module,
            &wildcard,
            verum_vbc::instruction::LayoutProperty::Size
        ),
        None
    );
}

#[test]
fn packed_array_storage_does_not_grant_write_through_an_immutable_borrow() {
    refuses("fn probe(items: &[Byte; 2]) { items[0] = 1; }");
    refuses("fn probe(items: &[Byte; 2]) { let item = &mut items[0]; }");
}

#[test]
fn packed_fields_do_not_grant_write_through_an_immutable_owner() {
    refuses("type Bytes is { data: [Byte; 2] }; fn probe(items: &Bytes) { items.data[0] = 1; }");
    refuses(
        "type Bytes is { data: [Byte; 2] }; fn probe(items: &Bytes) { let item = &mut items.data[0]; }",
    );
}

#[test]
fn packed_fields_preserve_mutable_reference_permission() {
    value(
        r#"
type Bytes is { data: [Byte; 2] };
fn update(items: &mut Bytes) { items.data[0] = 79; }
fn probe() -> Int {
    let mut items = Bytes { data: [0, 0] };
    update(&mut items);
    items.data[0] as Int
}
"#,
        79,
    );
}

#[test]
fn declaration_interfaces_do_not_borrow_a_shadowed_module_extent() {
    let source = r#"
module shadow_count;
const N: Int = 2;
public fn generic<N: meta Int>(items: [Int; N]) -> Int { 0 }
public fn runtime(N: Int, items: [Int; N]) -> Int { 0 }
public fn output<N: meta Int>() -> [Int; N] { [] }
"#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("shadow_count"))
        .collect_declaration_interface(&[&ast])
        .expect("declaration interface");
    for (name, slot) in [("generic", 0), ("runtime", 1)] {
        let function = module
            .functions
            .iter()
            .find(|f| {
                module
                    .get_string(f.name)
                    .is_some_and(|name_| name_.rsplit('.').next() == Some(name))
            })
            .unwrap();
        assert!(
            function.semantic_parameter_types().unwrap()[slot].is_none(),
            "module N cannot supply a function-owned extent: {name}"
        );
    }
    let output = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.rsplit('.').next() == Some("output"))
        })
        .unwrap();
    assert!(output.semantic_return.is_none());
}
