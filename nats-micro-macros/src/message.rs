use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Meta, Token, parse::Parser, punctuated::Punctuated};

use crate::util::{nats_micro_path, nats_micro_serde_path};

pub(crate) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    match expand_result(args, input) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand_result(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    if !cfg!(feature = "macros_json_feature") {
        return Err(syn::Error::new_spanned(
            &input,
            "#[message] requires the `json` feature",
        ));
    }
    let mut serde_options = Vec::new();
    let mut serde_crate = nats_micro_serde_path();
    let options = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(args)?;
    for option in options {
        if option.path().is_ident("schema") {
            let Meta::NameValue(value) = option else {
                return Err(syn::Error::new_spanned(
                    option,
                    "schema requires a boolean value",
                ));
            };
            let syn::Expr::Lit(expression) = value.value else {
                return Err(syn::Error::new_spanned(
                    value,
                    "schema requires a boolean value",
                ));
            };
            let syn::Lit::Bool(_value) = expression.lit else {
                return Err(syn::Error::new_spanned(
                    expression,
                    "schema requires a boolean value",
                ));
            };
        } else if option.path().is_ident("crate") {
            let Meta::NameValue(value) = option else {
                return Err(syn::Error::new_spanned(
                    option,
                    "crate requires a string path",
                ));
            };
            let syn::Expr::Lit(expression) = value.value else {
                return Err(syn::Error::new_spanned(
                    value,
                    "crate requires a string path",
                ));
            };
            let syn::Lit::Str(value) = expression.lit else {
                return Err(syn::Error::new_spanned(
                    expression,
                    "crate requires a string path",
                ));
            };
            serde_crate = value;
        } else {
            serde_options.push(option);
        }
    }

    let item: DeriveInput = syn::parse2(input)?;
    let ident = &item.ident;
    let generics = &item.generics;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let nats_micro = nats_micro_path();
    let name = ident.to_string();
    let schema_marker = quote! {
        impl #impl_generics #nats_micro::JsonMessage for #ident #type_generics #where_clause {
            const NAME: &'static str = #name;
        }
    };
    let serde_attr = if serde_options.is_empty() {
        quote!(#[serde(crate = #serde_crate)])
    } else {
        quote!(#[serde(crate = #serde_crate, #(#serde_options),*)])
    };

    Ok(quote! {
        #[derive(
            #nats_micro::serde::Serialize,
            #nats_micro::serde::Deserialize,
        )]
        #serde_attr
        #item

        #schema_marker
    })
}
