//! The public implementation query must not trust a claimed scalar carrier.
use verum_ast::{Span,ty::{Path,Ident}};
use verum_common::{List,Map,ConstValue};
use verum_types::{Type,context::GenericBinder,protocol::{ProtocolImpl,ProtocolChecker}};
fn protocol()->Path {Path::single(Ident::new("Evidence",Span::default()))}
fn cell(value:Type)->Type {Type::Named{path:Path::single(Ident::new("Cell",Span::default())),args:List::from_single(value)}}
#[test]
fn boolean_payload_cannot_claim_an_integer_carrier() {
    let parameter=Type::meta("N".into(),Type::Int,None);
    let implementation=ProtocolImpl {parameters:List::from_single(GenericBinder::Value(parameter.clone())),protocol:protocol(),protocol_args:List::new(),for_type:cell(parameter),where_clauses:List::new(),methods:Map::new(),associated_types:Map::new(),associated_consts:Map::new(),specialization:None,impl_crate:None,span:Span::default(),type_param_fn_bounds:Map::new()};
    let mut checker=ProtocolChecker::new_empty();checker.register_impl(implementation).unwrap();
    assert!(checker.find_impl(&cell(Type::meta_value(ConstValue::Int(7),Type::Int)),&protocol()).is_some());
    assert!(checker.find_impl(&cell(Type::meta_value(ConstValue::Bool(true),Type::Bool)),&protocol()).is_none());
    assert!(checker.find_impl(&cell(Type::meta_value(ConstValue::Bool(true),Type::Int)),&protocol()).is_none(),"forged payload/carrier was admitted");
}
