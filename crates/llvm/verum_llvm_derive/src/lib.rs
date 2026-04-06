//! Procedural macros for verum_llvm.
//!
//! This crate provides derive macros for safe LLVM enum conversions.
//! Simplified from inkwell_internals for LLVM 21 only (no version conditionals).

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::fold::Fold;
use syn::parse::{Parse, ParseStream, Result};
use syn::parse_macro_input;
use syn::{parse_quote, Arm, Attribute, Ident, PatPath, Path, Variant};

// ============================================================================
// #[llvm_enum] macro
// ============================================================================

/// Decorate an enum to generate conversions to/from an LLVM C enum type.
///
/// The macro generates:
/// - `impl EnumName { fn new(src: LLVMEnumType) -> Self }`
/// - `impl From<LLVMEnumType> for EnumName`
/// - `impl Into<LLVMEnumType> for EnumName`
///
/// # Example
///
/// ```ignore
/// #[llvm_enum(LLVMOpcode)]
/// enum InstructionOpcode {
///     Call,
///     #[llvm_variant(LLVMRet)]
///     Return,
///     ...
/// }
/// ```
#[proc_macro_attribute]
pub fn llvm_enum(args: TokenStream, input: TokenStream) -> TokenStream {
    let llvm_ty = parse_macro_input!(args as Path);
    let llvm_enum_type = parse_macro_input!(input as LLVMEnumType);
    llvm_enum_impl(llvm_ty, llvm_enum_type).into()
}

/// Tracks an enum variant and its LLVM <-> Rust mappings.
struct EnumVariant {
    llvm_variant: Ident,
    rust_variant: Ident,
    attrs: Vec<Attribute>,
}

impl EnumVariant {
    fn new(variant: &Variant) -> Self {
        let rust_variant = variant.ident.clone();
        let llvm_variant = Ident::new(&format!("LLVM{}", rust_variant), variant.ident.span());
        let mut attrs = variant.attrs.clone();
        attrs.retain(|attr| !attr.path().is_ident("llvm_variant"));
        Self {
            llvm_variant,
            rust_variant,
            attrs,
        }
    }

    fn with_name(variant: &Variant, mut llvm_variant: Ident) -> Self {
        let rust_variant = variant.ident.clone();
        llvm_variant.set_span(rust_variant.span());
        let mut attrs = variant.attrs.clone();
        attrs.retain(|attr| !attr.path().is_ident("llvm_variant"));
        Self {
            llvm_variant,
            rust_variant,
            attrs,
        }
    }
}

/// Collects variants during folding.
#[derive(Default)]
struct EnumVariants {
    variants: Vec<EnumVariant>,
    error: Option<syn::Error>,
}

impl Fold for EnumVariants {
    fn fold_variant(&mut self, mut variant: Variant) -> Variant {
        use syn::Meta;

        if self.error.is_some() {
            return variant;
        }

        // Check for #[llvm_variant(NAME)]
        let Some(attr) = variant
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("llvm_variant"))
        else {
            self.variants.push(EnumVariant::new(&variant));
            return variant;
        };

        if let Meta::List(meta) = &attr.meta
            && let Ok(Meta::Path(name)) = meta.parse_args()
        {
            self.variants
                .push(EnumVariant::with_name(&variant, name.get_ident().unwrap().clone()));
            variant
                .attrs
                .retain(|attr| !attr.path().is_ident("llvm_variant"));
            return variant;
        }
        self.error = Some(syn::Error::new_spanned(
            attr,
            "expected #[llvm_variant(VARIANT_NAME)]",
        ));
        variant
    }
}

/// Parsed enum declaration with #[llvm_enum(...)].
struct LLVMEnumType {
    name: Ident,
    decl: syn::ItemEnum,
    variants: Vec<EnumVariant>,
}

impl Parse for LLVMEnumType {
    fn parse(input: ParseStream) -> Result<Self> {
        let decl = input.parse::<syn::ItemEnum>()?;
        let name = decl.ident.clone();

        let mut variants = EnumVariants::default();
        let decl = variants.fold_item_enum(decl);

        if let Some(err) = variants.error {
            return Err(err);
        }

        Ok(Self {
            name,
            decl,
            variants: variants.variants,
        })
    }
}

/// Helper attribute for specifying LLVM variant names.
/// Used with `#[llvm_enum]` to map Rust enum variants to LLVM enum variants.
///
/// This is a no-op macro - the actual processing is done by `#[llvm_enum]`.
#[proc_macro_attribute]
pub fn llvm_variant(_args: TokenStream, input: TokenStream) -> TokenStream {
    // This is just a marker attribute - llvm_enum handles the actual work
    input
}

fn llvm_enum_impl(llvm_ty: Path, llvm_enum_type: LLVMEnumType) -> TokenStream2 {
    // Build LLVM -> Rust conversion arms
    let mut from_arms = Vec::with_capacity(llvm_enum_type.variants.len());
    for variant in &llvm_enum_type.variants {
        let src_variant = &variant.llvm_variant;
        let src_attrs: Vec<_> = variant
            .attrs
            .iter()
            .filter(|&attr| !attr.meta.path().is_ident("doc"))
            .collect();
        let dst_variant = &variant.rust_variant;
        let dst_ty = &llvm_enum_type.name;

        let pat = PatPath {
            attrs: Vec::new(),
            qself: None,
            path: parse_quote!(#llvm_ty::#src_variant),
        };

        let arm: Arm = parse_quote! {
            #(#src_attrs)*
            #pat => { #dst_ty::#dst_variant }
        };
        from_arms.push(arm);
    }

    // Build Rust -> LLVM conversion arms
    let mut to_arms = Vec::with_capacity(llvm_enum_type.variants.len());
    for variant in &llvm_enum_type.variants {
        let src_variant = &variant.rust_variant;
        let src_attrs: Vec<_> = variant
            .attrs
            .iter()
            .filter(|&attr| !attr.meta.path().is_ident("doc"))
            .collect();
        let src_ty = &llvm_enum_type.name;
        let dst_variant = &variant.llvm_variant;

        let pat = PatPath {
            attrs: Vec::new(),
            qself: None,
            path: parse_quote!(#src_ty::#src_variant),
        };

        let arm: Arm = parse_quote! {
            #(#src_attrs)*
            #pat => { #llvm_ty::#dst_variant }
        };
        to_arms.push(arm);
    }

    let enum_ty = &llvm_enum_type.name;
    let enum_decl = &llvm_enum_type.decl;

    quote! {
        #enum_decl

        impl #enum_ty {
            fn new(src: #llvm_ty) -> Self {
                match src {
                    #(#from_arms)*
                }
            }
        }

        impl From<#llvm_ty> for #enum_ty {
            fn from(src: #llvm_ty) -> Self {
                Self::new(src)
            }
        }

        impl Into<#llvm_ty> for #enum_ty {
            fn into(self) -> #llvm_ty {
                match self {
                    #(#to_arms),*
                }
            }
        }
    }
}
