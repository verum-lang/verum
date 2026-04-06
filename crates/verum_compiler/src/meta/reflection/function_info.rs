//! Function information for compile-time reflection
//!
//! Provides function signature metadata matching core/meta/reflection.vr FunctionInfo.

use verum_ast::MetaValue;
use verum_common::{List, Maybe, Text};

use super::generic_param::GenericParam;
use super::param_info::ParamInfo;
use super::type_kind::{TypeKind, Visibility};

/// Information about a function signature
///
/// Matches: core/meta/reflection.vr FunctionInfo
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionInfo {
    /// Function name
    pub name: Text,
    /// Full path
    pub path: Text,
    /// Generic parameters
    pub generics: List<GenericParam>,
    /// Function parameters
    pub params: List<ParamInfo>,
    /// Return type name
    pub return_type: Text,
    /// Return type kind
    pub return_kind: TypeKind,
    /// Whether function is async
    pub is_async: bool,
    /// Whether function is const
    pub is_const: bool,
    /// Whether function is unsafe
    pub is_unsafe: bool,
    /// Whether function is pure
    pub is_pure: bool,
    /// Whether function is meta
    pub is_meta: bool,
    /// Required contexts
    pub contexts: List<Text>,
    /// Function attributes
    pub attributes: List<Text>,
    /// Function documentation
    pub doc: Maybe<Text>,
    /// Visibility
    pub visibility: Visibility,
}

impl FunctionInfo {
    /// Create a new function info
    pub fn new(name: Text, return_type: Text) -> Self {
        Self {
            name: name.clone(),
            path: name,
            generics: List::new(),
            params: List::new(),
            return_type,
            return_kind: TypeKind::Unknown,
            is_async: false,
            is_const: false,
            is_unsafe: false,
            is_pure: false,
            is_meta: false,
            contexts: List::new(),
            attributes: List::new(),
            doc: Maybe::None,
            visibility: Visibility::Public,
        }
    }

    /// Get self parameter if present
    pub fn self_param(&self) -> Option<&ParamInfo> {
        self.params.first().filter(|p| p.is_self())
    }

    /// Check if this is a method (has self parameter)
    #[inline]
    pub fn is_method(&self) -> bool {
        self.self_param().is_some()
    }

    /// Check if this is a static function (no self)
    #[inline]
    pub fn is_static(&self) -> bool {
        !self.is_method()
    }

    /// Check if function requires specific context
    pub fn requires_context(&self, ctx: &str) -> bool {
        self.contexts.iter().any(|c| c.as_str() == ctx)
    }

    /// Mark as async
    #[inline]
    pub fn async_fn(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Mark as static (no self)
    pub fn static_fn(mut self) -> Self {
        if self.params.first().map(|p| p.is_self()).unwrap_or(false) {
            self.params = self.params.into_iter().skip(1).collect();
        }
        self
    }

    /// Get function signature as string
    pub fn signature(&self) -> Text {
        let params: Vec<String> = self.params.iter().map(|p| p.declaration().to_string()).collect();

        let mut sig = format!("fn {}", self.name.as_str());

        if !self.generics.is_empty() {
            let gens: Vec<String> = self
                .generics
                .iter()
                .map(|g| g.declaration().to_string())
                .collect();
            sig = format!("{}<{}>", sig, gens.join(", "));
        }

        sig = format!("{}({})", sig, params.join(", "));

        if self.return_type.as_str() != "()" {
            sig = format!("{} -> {}", sig, self.return_type.as_str());
        }

        if !self.contexts.is_empty() {
            let ctxs: Vec<&str> = self.contexts.iter().map(|c| c.as_str()).collect();
            sig = format!("{} using [{}]", sig, ctxs.join(", "));
        }

        Text::from(sig)
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.params.len() as i128),
                MetaValue::Text(self.return_type.clone()),
                MetaValue::Bool(self.is_async),
                MetaValue::Bool(self.is_method()),
            ]
            .into_iter()
            .collect(),
        )
    }

    /// Alias for to_meta_value for backward compatibility
    #[inline]
    pub fn to_const_value(&self) -> MetaValue {
        self.to_meta_value()
    }
}
