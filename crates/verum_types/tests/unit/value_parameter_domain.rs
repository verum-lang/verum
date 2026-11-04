use crate::ty::{Type,ValueId,ValueTerm};
use verum_common::ConstValue;
use verum_ast::{ItemKind,ty::GenericParamKind};
use crate::refinement::{RefinementPredicate,RefinementBinding,PredicateProvenance};
fn parameter(domain:&str)->Type {
    let ast=verum_fast_parser::Parser::new(&format!("fn bounded<N: meta Int{domain}>(x:Int)->Int {{ x }}")).parse_module().unwrap();
    let ItemKind::Function(f)=&ast.items[0].kind else {panic!("function")};
    let GenericParamKind::Meta{name,ty,refinement}= &f.generics[0].kind else {panic!("meta")};
    let mut checker=crate::TypeChecker::new(); checker.register_primitives();
    let carrier=checker.ast_to_type(ty).expect("actual declared carrier");
    Type::meta(name.name.clone(),carrier,refinement.as_ref().map(|p|RefinementPredicate {predicate:(**p).clone(),binding:RefinementBinding::Sigma(name.name.clone()),span:ty.span,provenance:PredicateProvenance::Declared}))
}
fn admits(p:&Type,a:&Type)->bool {super::argument_satisfies_domain(p,a,Clone::clone)}
#[test] fn empty_domain_still_requires_exact_carrier_sort(){let p=parameter("");assert!(admits(&p,&Type::meta_value(ConstValue::Int(7),Type::Int)));assert!(!admits(&p,&Type::meta_value(ConstValue::Bool(true),Type::Bool)));assert!(!admits(&p,&Type::meta("B".into(),Type::Bool,None)));}
#[test] fn unresolved_inference_is_not_a_domain_witness(){let p=parameter("");let a=Type::Meta{name:"N".into(),ty:Box::new(Type::Int),refinement:None,value:ValueTerm::Inference(ValueId::fresh())};assert!(!admits(&p,&a));assert!(admits(&p,&parameter("")));}
#[test] fn constants_discharge_actual_predicate(){let p=parameter("{>=0}");assert!(admits(&p,&Type::meta_value(ConstValue::Int(7),Type::Int)));assert!(!admits(&p,&Type::meta_value(ConstValue::Int(-1),Type::Int)));}
#[test] fn symbolic_domain_uses_entailment_not_matching_names(){let p=parameter("{>=0}");assert!(admits(&p,&parameter("{>=7}")));assert!(!admits(&p,&parameter("")));assert!(!admits(&parameter("{>=7}"),&p));}
#[test] fn identical_unsupported_predicate_is_not_proof(){let p=parameter("{it>=M}");assert!(!admits(&p,&p));assert!(!admits(&p,&Type::meta_value(ConstValue::Int(7),Type::Int)));}
#[test] fn normalization_authority_is_explicit(){let p=parameter("");let a=Type::meta_value(ConstValue::Int(7),Type::Generic{name:"CarrierAlias".into(),args:Default::default()});assert!(!admits(&p,&a));assert!(super::argument_satisfies_domain(&p,&a,|ty|match ty {Type::Generic{name,args} if name.as_str()=="CarrierAlias"&&args.is_empty()=>Type::Int,_=>ty.clone()}));}
