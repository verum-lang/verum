use verum_fast_parser::Parser;
use verum_types::TypeChecker;
use verum_vbc::{codegen::{CodegenConfig,VbcCodegen},serialize::serialize_module,deserialize::deserialize_module,interpreter::Interpreter};
const SOURCE:&str=r#"module query;
type Result<T,E> is Ok(T)|Err(E);
type Row is protocol {};
type DbError is { code:Int };
type Stream is protocol { type Item; };
type Connection is protocol {
 type Rows<R:Row>:Stream<Item=Result<R,DbError>>;
 fn query<R:Row>(&mut self,value:R)->Result<Self.Rows<R>,DbError>;
};
type Cursor<T> is { value:Result<T,DbError> };
implement<T> Stream for Cursor<T> { type Item=Result<T,DbError>; }
type Db is {};
implement Connection for Db {
 type Rows<R:Row> = Cursor<R>;
 fn query<R:Row>(&mut self,value:R)->Result<Self.Rows<R>,DbError> { Ok(Cursor {value:Ok(value)}) }
}
implement Row for Int {}
fn probe()->Int { let mut db=Db {}; let outer=db.query(42); match outer { Ok(rows)=>match rows.value { Ok(value)=>value, Err(_)=>-1 }, Err(_)=>-2 } }
"#;
#[test]
fn applied_family_through_source_wire_and_vm(){
 let ast=Parser::new(SOURCE).parse_module().unwrap();
 let mut checker=TypeChecker::new();checker.register_primitives();checker.set_current_module_path("query");
 let errors=checker.prepare_module_declarations(&ast);assert!(errors.is_empty(),"{errors:?}");
 for item in &ast.items { checker.check_item(item).unwrap(); }
 assert!(checker.diagnostics().is_empty(), "{:?}", checker.diagnostics());
 let module=VbcCodegen::with_config(CodegenConfig::new("query")).compile_module(&ast).unwrap();
 let wire=serialize_module(&module).unwrap(); let restored=deserialize_module(&wire).unwrap();
 for module in [module,restored] { let entry=module.find_function_by_name("query.probe").unwrap();let result=Interpreter::new(std::sync::Arc::new(module)).execute_function(entry).unwrap();assert_eq!(result.try_as_i64(),Some(42)); }
}

#[test]
fn applied_family_argument_must_survive_signature_substitution() {
 use verum_vbc::{types::{TypeRef,TypeId},mono::TypeSubstitution};
 let ast=Parser::new(SOURCE).parse_module().unwrap();
 let module=VbcCodegen::with_config(CodegenConfig::new("query")).compile_module(&ast).unwrap();
 let wire=serialize_module(&module).unwrap();let restored=deserialize_module(&wire).unwrap();
 for module in [module,restored] {
   let method=module.functions.iter().find(|function|module.get_string(function.name)==Some("Db.query")).unwrap();
   assert!(method.source_signature.is_some(), "applied family needs its source binder carrier");
   assert_eq!(method.type_params.len(),1,"one declaration-owned method binder");
   let int_result=TypeSubstitution::new(&method.type_params,&[TypeRef::Concrete(TypeId::INT)]).apply(&method.return_type).unwrap();
   let text_result=TypeSubstitution::new(&method.type_params,&[TypeRef::Concrete(TypeId::TEXT)]).apply(&method.return_type).unwrap();
   assert_ne!(int_result,text_result,"Self.Rows<R> must remain dependent on its declared method binder R");
 }
}

#[test]
fn typed_source_qualifier_cannot_be_published_as_an_unqualified_projection() {
 use verum_ast::{ItemKind, TypeKind, Path, Ident};
 use verum_vbc::codegen::CodegenErrorKind;
 let mut module = Parser::new("module qualifiers; type Relation is protocol {type Item;}; fn keep<T:Relation>(value:T.Item)->T.Item{value}").parse_module().unwrap();
 let function = module.items.iter_mut().find_map(|item| match &mut item.kind {
  ItemKind::Function(function) => Some(function), _ => None,
 }).unwrap();
 let result = function.return_type.as_mut().unwrap();
 let expected_span = result.span;
 let TypeKind::Qualified { trait_ref, .. } = &mut result.kind else {panic!("source projection")};
 // The current surface parser supplies an empty qualifier. The public AST
 // boundary must also reject a supplied qualifier it cannot represent exactly.
 *trait_ref = Path::single(Ident::new("Relation", expected_span));
 let error = VbcCodegen::with_config(CodegenConfig::new("qualifiers")).compile_module(&module).unwrap_err();
 assert!(matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(ref reason)
  if reason.contains("qualified associated projection")), "{error}");
 assert_eq!(error.span, Some(expected_span));
}
