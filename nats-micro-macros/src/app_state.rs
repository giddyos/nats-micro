use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

use crate::util::nats_micro_path;

pub(crate) fn expand(input: DeriveInput) -> TokenStream {
    match expand_result(input) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand_result(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "AppState can only be derived for structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "AppState requires named fields",
        ));
    };
    let nats_micro = nats_micro_path();
    let projections = fields
        .named
        .iter()
        .filter(|field| field.attrs.iter().any(|attr| attr.path().is_ident("state")))
        .map(|field| {
            let field_ident = field.ident.as_ref().expect("named field");
            let ty = &field.ty;
            quote! {
                impl #nats_micro::FromAppState<#ident> for #ty {
                    fn from_state(state: &#ident) -> &Self {
                        &state.#field_ident
                    }
                }
            }
        });
    Ok(quote! {
        #(#projections)*
    })
}
