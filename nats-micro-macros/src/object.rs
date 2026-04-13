use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Fields, ItemStruct, Visibility, spanned::Spanned};

use crate::utils::nats_micro_path;

pub(crate) fn expand_object(item: &ItemStruct) -> TokenStream {
    if !item.generics.params.is_empty() {
        let span = item.generics.span();
        return quote_spanned! { span =>
            compile_error!("#[nats_micro::object] does not support generic structs");
        };
    }

    let Fields::Named(named_fields) = &item.fields else {
        let span = item.span();
        return quote_spanned! { span =>
            compile_error!("#[nats_micro::object] only supports structs with named fields");
        };
    };

    if let Some(field) = named_fields
        .named
        .iter()
        .find(|field| !matches!(field.vis, Visibility::Public(_)))
    {
        let span = field.span();
        return quote_spanned! { span =>
            compile_error!("#[nats_micro::object] requires all fields to be pub");
        };
    }

    let nats_micro = nats_micro_path();
    let ident = &item.ident;
    let vis = &item.vis;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();
    let module_name = ident.to_string();
    let module_ident = format_ident!(
        "__nats_micro_object_{}",
        heck::ToSnakeCase::to_snake_case(module_name.as_str())
    );

    quote! {
        #[doc(hidden)]
        mod #module_ident {
            use super::*;
            use #nats_micro::napi as napi;

            #[#nats_micro::napi_derive::napi(object)]
            #item
        }

        #vis use #module_ident::#ident;

        impl #impl_generics #nats_micro::__private::NapiObject for #ident #ty_generics #where_clause {}
    }
}
