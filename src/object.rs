use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{Fields, ItemStruct, Visibility, spanned::Spanned};

use crate::utils::nats_micro_path;

pub(crate) fn expand_object(item: ItemStruct) -> TokenStream {
    if !item.generics.params.is_empty() {
        let span = item.generics.span();
        return quote_spanned! { span =>
            compile_error!("#[nats_micro::object] does not support generic structs");
        };
    }

    let named_fields = match &item.fields {
        Fields::Named(fields) => fields,
        _ => {
            let span = item.span();
            return quote_spanned! { span =>
                compile_error!("#[nats_micro::object] only supports structs with named fields");
            };
        }
    };

    for field in &named_fields.named {
        if !matches!(field.vis, Visibility::Public(_)) {
            let span = field.span();
            return quote_spanned! { span =>
                compile_error!("#[nats_micro::object] requires all fields to be pub");
            };
        }
    }

    let nats_micro = nats_micro_path();
    let ident = &item.ident;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    quote! {
        #[#nats_micro::napi_derive::napi(object)]
        #item

        impl #impl_generics #nats_micro::__private::NapiObject for #ident #ty_generics #where_clause {}
    }
}
