use verum_compiler::embedded_stdlib_metadata::{decode_metadata, encode_metadata};
use verum_types::core_metadata::{CORE_METADATA_VERSION, CoreMetadata};

#[test]
fn sidecar_schema_is_checked_before_payload_decoding() {
    let metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    let current = encode_metadata(&metadata).unwrap();
    assert_eq!(
        decode_metadata(&current).unwrap().version,
        CORE_METADATA_VERSION
    );
    for version in [0, 1, CORE_METADATA_VERSION + 1] {
        let mut wrong_header = current[..12].to_vec();
        wrong_header[8..12].copy_from_slice(&version.to_le_bytes());
        // Deliberately corrupt payload: version refusal must happen first.
        wrong_header.extend_from_slice(b"not a bincode payload");
        let error = decode_metadata(&wrong_header).unwrap_err();
        assert!(error.contains("unsupported metadata schema"), "{error}");
    }
    assert!(decode_metadata(&bincode::serialize(&metadata).unwrap()).is_err());
    assert!(encode_metadata(&CoreMetadata::default()).is_err());
}

#[test]
fn sidecar_rejects_trailing_and_mismatched_payloads() {
    let metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    let mut bytes = encode_metadata(&metadata).unwrap();
    bytes.push(0);
    assert!(decode_metadata(&bytes).is_err());
    let mut mismatched = encode_metadata(&metadata).unwrap()[..12].to_vec();
    mismatched.extend(bincode::serialize(&CoreMetadata::default()).unwrap());
    assert!(
        decode_metadata(&mismatched)
            .unwrap_err()
            .contains("header/payload schema mismatch")
    );
}

fn universe_metadata() -> (CoreMetadata, verum_ast::FunctionDecl) {
    use verum_types::core_metadata::{FunctionDescriptor, ParamDescriptor};
    let ast = verum_fast_parser::Parser::new(
        "fn retain<universe u>(value: Type(u)) -> Type(u) { value }",
    )
    .parse_module()
    .unwrap();
    let verum_ast::ItemKind::Function(function) = &ast.items[0].kind else {
        panic!("function")
    };
    let signature = verum_ast::source_signature::source_signature(function, false)
        .unwrap()
        .unwrap();
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    metadata.functions.insert(
        "test.retain".into(),
        FunctionDescriptor {
            source_signature: Some(signature.clone()),
            name: "retain".into(),
            module_path: "test".into(),
            origin_module_path: None,
            generic_params: Default::default(),
            params: [ParamDescriptor {
                name: "value".into(),
                ty: "Type(u)".into(),
                has_default: false,
                default_literal: None,
                declared_ty: "Type(u)".into(),
            }]
            .into_iter()
            .collect(),
            return_type: "Type(u)".into(),
            contexts: Default::default(),
            is_async: false,
            is_unsafe: false,
            intrinsic_id: None,
            parent_type: None,
            impl_generic_names: Default::default(),
            is_const: false,
            decl_span: None,
            is_public: true,
            explicit_type_param_ids: None,
        },
    );
    (metadata, signature)
}

fn replace_carrier(
    bytes: &mut Vec<u8>,
    original: &verum_ast::FunctionDecl,
    changed: &verum_ast::FunctionDecl,
) {
    let old = serde_json::to_vec(original).unwrap();
    let new = serde_json::to_vec(changed).unwrap();
    let start = bytes
        .windows(old.len())
        .position(|part| part == old)
        .expect("JSON carrier");
    assert_eq!(&bytes[start - 8..start], &(old.len() as u64).to_le_bytes());
    let replacement = (new.len() as u64).to_le_bytes().into_iter().chain(new);
    bytes.splice(start - 8..start + old.len(), replacement);
}

#[test]
fn sidecar_replays_binding_checks_on_tampered_carrier() {
    use verum_ast::ty::{TypeKind, UniverseLevelExpr};
    let (metadata, signature) = universe_metadata();
    let mut bytes = encode_metadata(&metadata).unwrap();
    assert!(
        decode_metadata(&bytes)
            .unwrap()
            .functions
            .values()
            .next()
            .unwrap()
            .source_signature
            .is_some()
    );
    let mut changed = signature.clone();
    let TypeKind::Universe {
        level: Some(UniverseLevelExpr::Variable(name)),
    } = &mut changed.return_type.as_mut().unwrap().kind
    else {
        panic!("universe")
    };
    name.name = "v".into();
    replace_carrier(&mut bytes, &signature, &changed);
    assert!(decode_metadata(&bytes).is_err());
}

#[test]
fn sidecar_refuses_excessive_json_depth_before_constructing_the_ast() {
    use verum_ast::ty::{TypeKind, UniverseLevelExpr};
    let (metadata, signature) = universe_metadata();
    let mut bytes = encode_metadata(&metadata).unwrap();
    let mut changed = signature.clone();
    let mut level = UniverseLevelExpr::Concrete(0);
    for _ in 0..180 {
        level = UniverseLevelExpr::Succ(Box::new(level));
    }
    changed.return_type.as_mut().unwrap().kind = TypeKind::Universe { level: Some(level) };
    replace_carrier(&mut bytes, &signature, &changed);
    let error = decode_metadata(&bytes).unwrap_err();
    assert!(error.contains("recursion limit"), "{error}");
}
