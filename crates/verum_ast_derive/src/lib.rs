//! Derive a shallow structural step. Every declared field is enqueued by
//! reference; the AST encoder owns traversal order, budgets and byte encoding.
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, parse_macro_input, parse_quote};
use verum_common::List;

#[proc_macro_derive(DefinitionNode)]
pub fn definition_node(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let mut generics = input.generics.clone();
    for parameter in generics.type_params_mut() {
        parameter
            .bounds
            .push(parse_quote!(crate::source_definition::Node));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let body = match &input.data {
        Data::Struct(data) => {
            let count = data.fields.len();
            let fields: List<_> = data
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let member = field
                        .ident
                        .clone()
                        .map(syn::Member::Named)
                        .unwrap_or_else(|| syn::Member::Unnamed(syn::Index::from(index)));
                    let label = field
                        .ident
                        .as_ref()
                        .map_or_else(|| index.to_string(), ToString::to_string);
                    quote!(encoder.field(#label, &self.#member)?;)
                })
                .collect();
            let fields = fields.iter().rev();
            quote! {
                encoder.record(concat!(module_path!(), "::", stringify!(#name)), #count)?;
                #(#fields)*
            }
        }
        Data::Enum(data) => {
            let arms = data.variants.iter().map(|variant| {
                let variant_name = &variant.ident;
                let count = variant.fields.len();
                let bindings: List<_> = variant.fields.iter().enumerate()
                    .map(|(index, _)| format_ident!("definition_field_{index}")).collect();
                let pattern = match &variant.fields {
                    Fields::Named(fields) => {
                        let members = fields.named.iter().zip(&bindings).map(|(field, binding)| {
                            let member = &field.ident; quote!(#member: #binding)
                        });
                        quote!(Self::#variant_name { #(#members),* })
                    }
                    Fields::Unnamed(_) => quote!(Self::#variant_name(#(#bindings),*)),
                    Fields::Unit => quote!(Self::#variant_name),
                };
                let fields: List<_> = variant.fields.iter().zip(&bindings).enumerate()
                    .map(|(index, (field, binding))| {
                        let label = field.ident.as_ref().map_or_else(|| index.to_string(), ToString::to_string);
                        quote!(encoder.field(#label, #binding)?;)
                    }).collect();
                let fields = fields.iter().rev();
                quote! { #pattern => {
                    encoder.variant(concat!(module_path!(), "::", stringify!(#name)), stringify!(#variant_name), #count)?;
                    #(#fields)*
                } }
            });
            quote!(match self { #(#arms),* })
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                name,
                "definition identity cannot read an untagged union",
            ));
        }
    };
    Ok(quote! {
        impl #impl_generics crate::source_definition::Node for #name #ty_generics #where_clause {
            fn encode<'__definition_borrow>(&'__definition_borrow self, encoder: &mut crate::source_definition::Encoder<'__definition_borrow, '_>)
                -> crate::source_definition::Result<()> {
                #body
                Ok(())
            }
        }
    })
}
