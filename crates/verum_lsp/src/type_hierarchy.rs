//! Type hierarchy support
//!
//! Provides supertype and subtype navigation for Verum types and protocols.
//! - Supertypes: for a type, finds implemented protocols; for a protocol, finds bounds
//! - Subtypes: for a protocol, finds all types implementing it

use crate::document::DocumentState;
use crate::position_utils::ast_span_to_range;
use tower_lsp::lsp_types::*;
use verum_ast::{ItemKind, Module};

/// Prepare type hierarchy at the given position
pub fn prepare_type_hierarchy(
    document: &DocumentState,
    position: Position,
    uri: &Url,
) -> Option<Vec<TypeHierarchyItem>> {
    let word = document.word_at_position(position)?;
    let module = document.module.as_ref()?;

    // Find the type or protocol at this position
    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Type(type_decl) if type_decl.name.as_str() == word => {
                let range = ast_span_to_range(&type_decl.span, &document.text);
                let selection_range =
                    ast_span_to_range(&type_decl.name.span, &document.text);
                return Some(vec![TypeHierarchyItem {
                    name: word.clone(),
                    kind: SymbolKind::STRUCT,
                    tags: None,
                    detail: Some("type".to_string()),
                    uri: uri.clone(),
                    range,
                    selection_range,
                    data: None,
                }]);
            }
            ItemKind::Protocol(protocol) if protocol.name.as_str() == word => {
                let range = ast_span_to_range(&protocol.span, &document.text);
                let selection_range =
                    ast_span_to_range(&protocol.name.span, &document.text);
                return Some(vec![TypeHierarchyItem {
                    name: word.clone(),
                    kind: SymbolKind::INTERFACE,
                    tags: None,
                    detail: Some("protocol".to_string()),
                    uri: uri.clone(),
                    range,
                    selection_range,
                    data: None,
                }]);
            }
            _ => {}
        }
    }

    None
}

/// Find supertypes for a type hierarchy item
///
/// For a type: returns protocols it implements
/// For a protocol: returns protocols it extends (bounds)
pub fn supertypes(
    document: &DocumentState,
    item: &TypeHierarchyItem,
    uri: &Url,
) -> Vec<TypeHierarchyItem> {
    let module = match &document.module {
        Some(m) => m,
        None => return Vec::new(),
    };

    let name = &item.name;
    let mut result = Vec::new();

    // If this is a type, find all protocol implementations for it
    for ast_item in module.items.iter() {
        if let ItemKind::Impl(impl_block) = &ast_item.kind {
            if let verum_ast::decl::ImplKind::Protocol {
                protocol,
                for_type,
                ..
            } = &impl_block.kind
            {
                let type_name = extract_ast_type_name(&for_type.kind);
                if type_name.as_deref() == Some(name.as_str()) {
                    // This impl block implements a protocol for our type
                    if let Some(protocol_name) = extract_path_name(protocol) {
                        // Find the protocol definition to get its range
                        if let Some(protocol_item) = find_protocol(module, &protocol_name) {
                            let range =
                                ast_span_to_range(&protocol_item.span, &document.text);
                            let selection_range =
                                ast_span_to_range(&protocol_item.name.span, &document.text);
                            result.push(TypeHierarchyItem {
                                name: protocol_name,
                                kind: SymbolKind::INTERFACE,
                                tags: None,
                                detail: Some("protocol".to_string()),
                                uri: uri.clone(),
                                range,
                                selection_range,
                                data: None,
                            });
                        }
                    }
                }
            }
        }
    }

    result
}

/// Find subtypes for a type hierarchy item
///
/// For a protocol: returns all types that implement it
/// For a type: returns types that extend it (if applicable)
pub fn subtypes(
    document: &DocumentState,
    item: &TypeHierarchyItem,
    uri: &Url,
) -> Vec<TypeHierarchyItem> {
    let module = match &document.module {
        Some(m) => m,
        None => return Vec::new(),
    };

    let name = &item.name;
    let mut result = Vec::new();

    // Find all types that implement this protocol
    for ast_item in module.items.iter() {
        if let ItemKind::Impl(impl_block) = &ast_item.kind {
            if let verum_ast::decl::ImplKind::Protocol {
                protocol,
                for_type,
                ..
            } = &impl_block.kind
            {
                let protocol_name = extract_path_name(protocol);
                if protocol_name.as_deref() == Some(name.as_str()) {
                    // This impl block implements our protocol
                    if let Some(type_name) = extract_ast_type_name(&for_type.kind) {
                        // Find the type definition to get its range
                        if let Some(type_decl) = find_type(module, &type_name) {
                            let range =
                                ast_span_to_range(&type_decl.span, &document.text);
                            let selection_range =
                                ast_span_to_range(&type_decl.name.span, &document.text);
                            result.push(TypeHierarchyItem {
                                name: type_name,
                                kind: SymbolKind::STRUCT,
                                tags: None,
                                detail: Some("type".to_string()),
                                uri: uri.clone(),
                                range,
                                selection_range,
                                data: None,
                            });
                        }
                    }
                }
            }
        }
    }

    result
}

// ==================== Helpers ====================

fn extract_ast_type_name(kind: &verum_ast::TypeKind) -> Option<String> {
    match kind {
        verum_ast::TypeKind::Path(path) => extract_path_name(path),
        _ => None,
    }
}

fn extract_path_name(path: &verum_ast::ty::Path) -> Option<String> {
    for seg in path.segments.iter().rev() {
        if let verum_ast::ty::PathSegment::Name(ident) = seg {
            return Some(ident.as_str().to_string());
        }
    }
    None
}

fn find_protocol<'a>(
    module: &'a Module,
    name: &str,
) -> Option<&'a verum_ast::decl::ProtocolDecl> {
    for item in module.items.iter() {
        if let ItemKind::Protocol(protocol) = &item.kind {
            if protocol.name.as_str() == name {
                return Some(protocol);
            }
        }
    }
    None
}

fn find_type<'a>(module: &'a Module, name: &str) -> Option<&'a verum_ast::decl::TypeDecl> {
    for item in module.items.iter() {
        if let ItemKind::Type(type_decl) = &item.kind {
            if type_decl.name.as_str() == name {
                return Some(type_decl);
            }
        }
    }
    None
}
