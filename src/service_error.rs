use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Data, DeriveInput, Fields};

use crate::utils::{error_stream, nats_micro_path};

pub fn expand_service_error(mut input: DeriveInput) -> TokenStream {
    let nats_micro = nats_micro_path();
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
    let mut from_response_arms = Vec::new();

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

        // The generated encoder and decoder are derived from the same field
        // shape so the wire format stays simple and mechanically reversible.
        // When we cannot prove that reversal is lossless, we preserve the
        // original NATS error instead of synthesizing a misleading typed value.
        let (pattern, details, from_response) = match &variant.fields {
            Fields::Unit => (
                quote! { #enum_ident::#v_ident },
                quote! { None },
                quote! {
                    (#code, #v_name) => #nats_micro::ServiceErrorMatch::Typed(#enum_ident::#v_ident),
                },
            ),
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let field = fields.unnamed.first().expect("single field");
                match &field.ty {
                    syn::Type::Path(type_path) if type_path.path.is_ident("String") => (
                        quote! { #enum_ident::#v_ident(__value) },
                        if is_internal {
                            quote! { None }
                        } else {
                            quote! { Some(#nats_micro::serde_json::Value::String(__value.clone())) }
                        },
                        quote! {
                            (#code, #v_name) => #nats_micro::ServiceErrorMatch::Typed(
                                #enum_ident::#v_ident(
                                    response
                                        .details
                                        .as_ref()
                                        .and_then(|details| details.as_str())
                                        .unwrap_or(response.message.as_str())
                                        .to_string(),
                                )
                            ),
                        },
                    ),
                    _ => build_unnamed_variant_tokens(
                        enum_ident,
                        v_ident,
                        code,
                        &v_name,
                        fields,
                        is_internal,
                    ),
                }
            }
            Fields::Unnamed(fields) => build_unnamed_variant_tokens(
                enum_ident,
                v_ident,
                code,
                &v_name,
                fields,
                is_internal,
            ),
            Fields::Named(fields) => {
                build_named_variant_tokens(enum_ident, v_ident, code, &v_name, fields, is_internal)
            }
        };

        variant_arms.push(quote! {
            #pattern => {
                let __error = #nats_micro::NatsErrorResponse::new(
                    #code,
                    #v_name,
                    #message,
                    request_id,
                );
                match #details {
                    Some(details) => __error.with_details(details),
                    None => __error,
                }
            },
        });

        from_response_arms.push(from_response);
    }

    if let Data::Enum(ref mut data_enum) = input.data {
        for variant in data_enum.variants.iter_mut() {
            variant
                .attrs
                .retain(|attr| !attr.path().is_ident("internal") && !attr.path().is_ident("code"));
        }
    }

    let enum_impl = quote! {
        impl #nats_micro::IntoNatsError for #enum_ident {
            fn into_nats_error(self, request_id: String) -> #nats_micro::NatsErrorResponse {
                let __message = ::std::string::ToString::to_string(&self);
                match self {
                    #(#variant_arms)*
                }
            }
        }

        impl #nats_micro::FromNatsErrorResponse for #enum_ident {
            fn from_nats_error_response(
                response: #nats_micro::NatsErrorResponse,
            ) -> #nats_micro::ServiceErrorMatch<Self> {
                match (response.code, response.error.as_str()) {
                    #(#from_response_arms)*
                    _ => #nats_micro::ServiceErrorMatch::Untyped(response),
                }
            }
        }
    };

    let mut out = TokenStream::new();
    out.extend(input.into_token_stream());
    out.extend(enum_impl);
    out
}

fn build_unnamed_variant_tokens(
    enum_ident: &syn::Ident,
    variant_ident: &syn::Ident,
    code: u16,
    variant_name: &str,
    fields: &syn::FieldsUnnamed,
    is_internal: bool,
) -> (TokenStream, TokenStream, TokenStream) {
    let nats_micro = nats_micro_path();
    // Public tuple variants are serialized as positional tuples rather than
    // object maps so both tuple and named variants can share one stable,
    // order-based round-trip format.
    let bindings: Vec<syn::Ident> = (0..fields.unnamed.len())
        .map(|index| syn::Ident::new(&format!("__field_{index}"), proc_macro2::Span::call_site()))
        .collect();
    let types: Vec<_> = fields.unnamed.iter().map(|field| &field.ty).collect();
    let details = if is_internal {
        quote! { None }
    } else {
        let serialize_value = tuple_value(&bindings);
        quote! { #nats_micro::serde_json::to_value(#serialize_value).ok() }
    };
    let tuple_type = tuple_type(&types);
    let deserialize_pattern = tuple_pattern(&bindings);

    (
        quote! { #enum_ident::#variant_ident(#(#bindings),*) },
        details,
        quote! {
            (#code, #variant_name) => match response
                .details
                .clone()
                .and_then(|details| #nats_micro::serde_json::from_value::<#tuple_type>(details).ok())
            {
                Some(#deserialize_pattern) => {
                    #nats_micro::ServiceErrorMatch::Typed(#enum_ident::#variant_ident(#(#bindings),*))
                }
                None => #nats_micro::ServiceErrorMatch::Untyped(response),
            },
        },
    )
}

fn build_named_variant_tokens(
    enum_ident: &syn::Ident,
    variant_ident: &syn::Ident,
    code: u16,
    variant_name: &str,
    fields: &syn::FieldsNamed,
    is_internal: bool,
) -> (TokenStream, TokenStream, TokenStream) {
    let nats_micro = nats_micro_path();
    // Named variants deliberately use the same tuple-shaped details payload as
    // tuple variants. That keeps the reconstruction logic uniform and avoids a
    // second wire format whose field names would become part of the protocol.
    let field_idents: Vec<_> = fields
        .named
        .iter()
        .map(|field| field.ident.clone().expect("named field"))
        .collect();
    let bindings: Vec<syn::Ident> = field_idents
        .iter()
        .map(|ident| syn::Ident::new(&format!("__{ident}"), ident.span()))
        .collect();
    let types: Vec<_> = fields.named.iter().map(|field| &field.ty).collect();
    let details = if is_internal {
        quote! { None }
    } else {
        let serialize_value = tuple_value(&bindings);
        quote! { #nats_micro::serde_json::to_value(#serialize_value).ok() }
    };
    let tuple_type = tuple_type(&types);
    let deserialize_pattern = tuple_pattern(&bindings);

    (
        quote! { #enum_ident::#variant_ident { #(#field_idents: #bindings),* } },
        details,
        quote! {
            (#code, #variant_name) => match response
                .details
                .clone()
                .and_then(|details| #nats_micro::serde_json::from_value::<#tuple_type>(details).ok())
            {
                Some(#deserialize_pattern) => {
                    #nats_micro::ServiceErrorMatch::Typed(#enum_ident::#variant_ident { #(#field_idents: #bindings),* })
                }
                None => #nats_micro::ServiceErrorMatch::Untyped(response),
            },
        },
    )
}

fn tuple_type(types: &[&syn::Type]) -> TokenStream {
    if types.len() == 1 {
        let ty = types[0];
        quote! { (#ty,) }
    } else {
        quote! { (#(#types),*) }
    }
}

fn tuple_value(idents: &[syn::Ident]) -> TokenStream {
    if idents.len() == 1 {
        let ident = &idents[0];
        quote! { (#ident,) }
    } else {
        quote! { (#(#idents),*) }
    }
}

fn tuple_pattern(idents: &[syn::Ident]) -> TokenStream {
    if idents.len() == 1 {
        let ident = &idents[0];
        quote! { (#ident,) }
    } else {
        quote! { (#(#idents),*) }
    }
}
