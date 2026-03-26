use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{Data, DeriveInput, Fields};

use crate::utils::error_stream;

pub fn expand_service_error(mut input: DeriveInput) -> TokenStream {
    let span = input.ident.span();
    let enum_ident = &input.ident;

    let data_enum = match &input.data {
        Data::Enum(e) => e.clone(),
        _ => {
            return error_stream(
                span,
                "#[service_error] can only be applied to enums",
                &input,
            );
        }
    };

    let mut variant_arms = Vec::new();

    for variant in &data_enum.variants {
        let v_ident = &variant.ident;
        let v_name = v_ident.to_string();

        let is_internal = variant
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("internal"));

        let code: u16 = variant
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("code"))
            .and_then(|attr| attr.parse_args::<syn::LitInt>().ok())
            .and_then(|lit| lit.base10_parse().ok())
            .unwrap_or(if is_internal { 500 } else { 400 });

        let message = if is_internal {
            quote! { "an internal error occurred".to_string() }
        } else {
            quote! { __message }
        };

        let pattern = match &variant.fields {
            Fields::Unit => quote! { #enum_ident::#v_ident },
            Fields::Named(_) => quote! { #enum_ident::#v_ident { .. } },
            Fields::Unnamed(_) => quote! { #enum_ident::#v_ident(..) },
        };

        variant_arms.push(quote! {
            #pattern => ::nats_micro::NatsErrorResponse::new(
                #code,
                #v_name,
                #message,
                request_id,
            ),
        });
    }

    if let Data::Enum(ref mut data_enum) = input.data {
        for variant in data_enum.variants.iter_mut() {
            variant.attrs.retain(|attr| {
                !attr.path().is_ident("internal") && !attr.path().is_ident("code")
            });
        }
    }

    let enum_impl = quote! {
        impl ::nats_micro::IntoNatsError for #enum_ident {
            fn into_nats_error(self, request_id: String) -> ::nats_micro::NatsErrorResponse {
                let __message = ::std::string::ToString::to_string(&self);
                match self {
                    #(#variant_arms)*
                }
            }
        }
    };

    let mut out = TokenStream::new();
    out.extend(input.into_token_stream());
    out.extend(enum_impl);
    out
}
