use heck::{AsShoutySnakeCase, ToSnakeCase};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Data, DeriveInput, Fields, Path, Token, punctuated::Punctuated};

use crate::utils::{error_stream, nats_micro_path};

#[allow(clippy::too_many_lines)]
pub fn expand_service_error(mut input: DeriveInput) -> TokenStream {
    let nats_micro = nats_micro_path();
    let span = input.ident.span();
    let enum_ident = input.ident.clone();
    let enum_vis = input.vis.clone();
    let generics = input.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

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

    normalize_service_error_derives(&mut input);

    let mut display_arms = Vec::new();
    let mut variant_arms = Vec::new();
    let mut from_response_arms = Vec::new();
    let mut napi_js_variants = Vec::new();
    let mut napi_as_ref_arms = Vec::new();
    let mut napi_from_error_arms = Vec::new();
    let js_enum_ident = format_ident!("Js{}", enum_ident);

    for variant in &data_enum.variants {
        let v_ident = &variant.ident;
        let v_name = v_ident.to_string();
        let v_code = format!("{}", AsShoutySnakeCase(&v_name));
        let js_variant_ident = format_ident!("{}", v_code);
        let error_message = match variant_error_message(variant) {
            Ok(message) => message,
            Err(error) => return error.to_compile_error(),
        };

        let is_internal = variant_is_internal(variant);
        let code = variant_code(variant, is_internal);

        let message = if is_internal {
            quote! { "an internal error occurred".to_string() }
        } else {
            quote! { __message }
        };

        let napi_from_pattern = match &variant.fields {
            Fields::Unit => quote! { #enum_ident::#v_ident },
            Fields::Unnamed(_) => quote! { #enum_ident::#v_ident(..) },
            Fields::Named(_) => quote! { #enum_ident::#v_ident { .. } },
        };

        let display_arm = match display_arm_tokens(&enum_ident, variant, &error_message) {
            Ok(tokens) => tokens,
            Err(error) => return error.to_compile_error(),
        };

        display_arms.push(display_arm);
        napi_js_variants.push(quote! { #js_variant_ident });
        napi_as_ref_arms.push(quote! {
            Self::#js_variant_ident => #v_code,
        });
        napi_from_error_arms.push(quote! {
            #napi_from_pattern => #js_enum_ident::#js_variant_ident,
        });

        // The generated encoder and decoder are derived from the same field
        // shape so the wire format stays simple and mechanically reversible.
        // When we cannot prove that reversal is lossless, we preserve the
        // original NATS error instead of synthesizing a misleading typed value.
        let (pattern, details, from_response) = match &variant.fields {
            Fields::Unit => (
                quote! { #enum_ident::#v_ident },
                quote! { None },
                quote! {
                    (#code, #v_code) => #nats_micro::ServiceErrorMatch::Typed(#enum_ident::#v_ident),
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
                            (#code, #v_code) => #nats_micro::ServiceErrorMatch::Typed(
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
                        &enum_ident,
                        v_ident,
                        code,
                        &v_code,
                        fields,
                        is_internal,
                    ),
                }
            }
            Fields::Unnamed(fields) => build_unnamed_variant_tokens(
                &enum_ident,
                v_ident,
                code,
                &v_code,
                fields,
                is_internal,
            ),
            Fields::Named(fields) => {
                build_named_variant_tokens(&enum_ident, v_ident, code, &v_code, fields, is_internal)
            }
        };

        variant_arms.push(quote! {
            #pattern => {
                let __error = #nats_micro::NatsErrorResponse::new(
                    #code,
                    #v_code,
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
        for variant in &mut data_enum.variants {
            variant.attrs.retain(|attr| {
                !attr.path().is_ident("internal")
                    && !attr.path().is_ident("code")
                    && !attr.path().is_ident("error")
            });
        }
    }

    let display_impl = quote! {
        impl #impl_generics ::std::fmt::Display for #enum_ident #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self {
                    #(#display_arms)*
                }
            }
        }

        impl #impl_generics ::std::error::Error for #enum_ident #ty_generics #where_clause {}
    };

    let enum_impl = quote! {
        impl #impl_generics #nats_micro::IntoNatsError for #enum_ident #ty_generics #where_clause {
            fn into_nats_error(self, request_id: String) -> #nats_micro::NatsErrorResponse {
                let __message = ::std::string::ToString::to_string(&self);
                match self {
                    #(#variant_arms)*
                }
            }
        }

        impl #impl_generics #nats_micro::FromNatsErrorResponse for #enum_ident #ty_generics #where_clause {
            fn from_nats_error_response(
                response: #nats_micro::NatsErrorResponse,
            ) -> #nats_micro::ServiceErrorMatch<Self> {
                match (response.code, response.kind.as_str()) {
                    #(#from_response_arms)*
                    _ => #nats_micro::ServiceErrorMatch::Untyped(response),
                }
            }
        }
    };

    let napi_impl = if cfg!(feature = "macros_napi_feature") {
        let napi_module_ident = format_ident!(
            "__nats_micro_service_error_napi_{}",
            enum_ident.to_string().to_snake_case()
        );

        quote! {
            #[doc(hidden)]
            mod #napi_module_ident {
                use super::*;
                use #nats_micro::napi as napi;

                #[derive(Debug, Clone, Copy, PartialEq, Eq)]
                #[#nats_micro::napi_derive::napi(string_enum)]
                pub enum #js_enum_ident {
                    #(#napi_js_variants,)*
                }

                impl ::std::convert::AsRef<str> for #js_enum_ident {
                    fn as_ref(&self) -> &str {
                        match self {
                            #(#napi_as_ref_arms)*
                        }
                    }
                }

                impl #impl_generics ::std::convert::From<&#enum_ident #ty_generics> for #js_enum_ident #where_clause {
                    fn from(value: &#enum_ident #ty_generics) -> Self {
                        match value {
                            #(#napi_from_error_arms)*
                        }
                    }
                }
            }

            #enum_vis use #napi_module_ident::#js_enum_ident;
            impl #impl_generics #nats_micro::__private::NapiServiceError for #enum_ident #ty_generics #where_clause {}
        }
    } else {
        quote! {}
    };

    let mut out = TokenStream::new();
    out.extend(input.into_token_stream());
    out.extend(display_impl);
    out.extend(enum_impl);
    out.extend(napi_impl);
    out
}

fn normalize_service_error_derives(input: &mut DeriveInput) {
    let mut has_debug = false;
    let mut attrs = Vec::with_capacity(input.attrs.len() + 2);

    for attr in &input.attrs {
        if !attr.path().is_ident("derive") {
            attrs.push(attr.clone());
            continue;
        }

        let Ok(paths) = attr.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated)
        else {
            attrs.push(attr.clone());
            continue;
        };

        let mut rewritten = Punctuated::<Path, Token![,]>::new();
        for path in paths {
            let Some(segment) = path.segments.last() else {
                rewritten.push(path);
                continue;
            };

            if segment.ident == "Debug" {
                has_debug = true;
                rewritten.push(syn::parse_quote!(Debug));
                continue;
            }

            if segment.ident == "Error" {
                continue;
            }

            rewritten.push(path);
        }

        if !rewritten.is_empty() {
            attrs.push(syn::parse_quote!(#[derive(#rewritten)]));
        }
    }

    if !has_debug {
        attrs.push(syn::parse_quote!(#[derive(Debug)]));
    }

    input.attrs = attrs;
}

fn variant_error_message(variant: &syn::Variant) -> Result<syn::LitStr, syn::Error> {
    let Some(attr) = variant
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("error"))
    else {
        return Err(syn::Error::new_spanned(
            variant,
            "service_error variants require #[error(\"...\")]",
        ));
    };

    attr.parse_args::<syn::LitStr>().map_err(|_| {
        syn::Error::new_spanned(
            attr,
            "service_error only supports #[error(\"...\")] string literals",
        )
    })
}

fn display_arm_tokens(
    enum_ident: &syn::Ident,
    variant: &syn::Variant,
    message: &syn::LitStr,
) -> Result<TokenStream, syn::Error> {
    let (format_string, placeholders) = parse_error_format(message)?;
    let format_lit = syn::LitStr::new(&format_string, message.span());
    let v_ident = &variant.ident;

    match &variant.fields {
        Fields::Unit => {
            if !placeholders.is_empty() {
                return Err(syn::Error::new_spanned(
                    variant,
                    "service_error format placeholders require enum fields",
                ));
            }

            Ok(quote! {
                Self::#v_ident => ::std::write!(f, #format_lit),
            })
        }
        Fields::Unnamed(fields) => {
            let bindings: Vec<syn::Ident> = (0..fields.unnamed.len())
                .map(|index| {
                    syn::Ident::new(&format!("__field_{index}"), proc_macro2::Span::call_site())
                })
                .collect();
            let args = display_args_tokens(variant, &bindings, &[], &placeholders)?;

            Ok(if args.is_empty() {
                quote! {
                    #enum_ident::#v_ident(#(#bindings),*) => ::std::write!(f, #format_lit),
                }
            } else {
                quote! {
                    #enum_ident::#v_ident(#(#bindings),*) => ::std::write!(f, #format_lit, #(#args),*),
                }
            })
        }
        Fields::Named(fields) => {
            let field_idents: Vec<_> = fields
                .named
                .iter()
                .map(|field| field.ident.clone().expect("named field"))
                .collect();
            let bindings: Vec<syn::Ident> = field_idents
                .iter()
                .map(|ident| syn::Ident::new(&format!("__{ident}"), ident.span()))
                .collect();
            let args = display_args_tokens(variant, &bindings, &field_idents, &placeholders)?;

            Ok(if args.is_empty() {
                quote! {
                    #enum_ident::#v_ident { #(#field_idents: #bindings),* } => ::std::write!(f, #format_lit),
                }
            } else {
                quote! {
                    #enum_ident::#v_ident { #(#field_idents: #bindings),* } => ::std::write!(f, #format_lit, #(#args),*),
                }
            })
        }
    }
}

fn display_args_tokens(
    variant: &syn::Variant,
    bindings: &[syn::Ident],
    field_idents: &[syn::Ident],
    placeholders: &[FormatPlaceholder],
) -> Result<Vec<TokenStream>, syn::Error> {
    let mut args = Vec::with_capacity(placeholders.len());
    let mut next_implicit = 0usize;

    for placeholder in placeholders {
        let index = match &placeholder.key {
            PlaceholderKey::Implicit => {
                let index = next_implicit;
                next_implicit += 1;
                index
            }
            PlaceholderKey::Index(index) => *index,
            PlaceholderKey::Name(name) => field_idents
                .iter()
                .position(|ident| ident == name)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        variant,
                        format!("unknown service_error format field `{name}`"),
                    )
                })?,
        };

        let Some(binding) = bindings.get(index) else {
            return Err(syn::Error::new_spanned(
                variant,
                format!("service_error format placeholder {{{index}}} is out of bounds"),
            ));
        };

        args.push(quote! { #binding });
    }

    Ok(args)
}

fn parse_error_format(
    message: &syn::LitStr,
) -> Result<(String, Vec<FormatPlaceholder>), syn::Error> {
    let value = message.value();
    let chars: Vec<_> = value.chars().collect();
    let mut formatted = String::new();
    let mut placeholders = Vec::new();
    let mut index = 0usize;

    while index < chars.len() {
        match chars[index] {
            '{' => {
                if chars.get(index + 1) == Some(&'{') {
                    formatted.push_str("{{");
                    index += 2;
                    continue;
                }

                let start = index + 1;
                let mut end = start;
                while end < chars.len() && chars[end] != '}' {
                    if chars[end] == '{' {
                        return Err(syn::Error::new(
                            message.span(),
                            "nested braces are not supported in service_error format strings",
                        ));
                    }
                    end += 1;
                }

                if end == chars.len() {
                    return Err(syn::Error::new(
                        message.span(),
                        "unterminated format placeholder in service_error message",
                    ));
                }

                let placeholder = chars[start..end].iter().collect::<String>();
                let (key, format_spec) = parse_placeholder_key(&placeholder, message)?;
                placeholders.push(FormatPlaceholder { key });
                formatted.push('{');
                formatted.push_str(&format_spec);
                formatted.push('}');
                index = end + 1;
            }
            '}' => {
                if chars.get(index + 1) == Some(&'}') {
                    formatted.push_str("}}");
                    index += 2;
                } else {
                    return Err(syn::Error::new(
                        message.span(),
                        "unmatched closing brace in service_error message",
                    ));
                }
            }
            ch => {
                formatted.push(ch);
                index += 1;
            }
        }
    }

    Ok((formatted, placeholders))
}

fn parse_placeholder_key(
    placeholder: &str,
    message: &syn::LitStr,
) -> Result<(PlaceholderKey, String), syn::Error> {
    let (raw_key, raw_spec) = match placeholder.split_once(':') {
        Some((key, spec)) => (key, format!(":{spec}")),
        None => (placeholder, String::new()),
    };
    let key = raw_key.trim();

    if key.is_empty() {
        return Ok((PlaceholderKey::Implicit, raw_spec));
    }

    if let Ok(index) = key.parse::<usize>() {
        return Ok((PlaceholderKey::Index(index), raw_spec));
    }

    if syn::parse_str::<syn::Ident>(key).is_ok() {
        return Ok((PlaceholderKey::Name(format_ident!("{key}")), raw_spec));
    }

    Err(syn::Error::new(
        message.span(),
        format!("unsupported service_error placeholder `{{{key}}}`"),
    ))
}

struct FormatPlaceholder {
    key: PlaceholderKey,
}

enum PlaceholderKey {
    Implicit,
    Index(usize),
    Name(syn::Ident),
}

fn variant_is_internal(variant: &syn::Variant) -> bool {
    variant
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("internal"))
}

fn variant_code(variant: &syn::Variant, is_internal: bool) -> u16 {
    variant
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("code"))
        .and_then(|attr| attr.parse_args::<syn::LitInt>().ok())
        .and_then(|lit| lit.base10_parse().ok())
        .unwrap_or(if is_internal { 500 } else { 400 })
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
        let serialize_value = singleton_or_tuple(&bindings);
        quote! { #nats_micro::serde_json::to_value(#serialize_value).ok() }
    };
    let tuple_type = singleton_or_tuple(&types);
    let deserialize_pattern = singleton_or_tuple(&bindings);

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
        let serialize_value = singleton_or_tuple(&bindings);
        quote! { #nats_micro::serde_json::to_value(#serialize_value).ok() }
    };
    let tuple_type = singleton_or_tuple(&types);
    let deserialize_pattern = singleton_or_tuple(&bindings);

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

fn singleton_or_tuple<T: ToTokens>(items: &[T]) -> TokenStream {
    if let [item] = items {
        quote! { (#item,) }
    } else {
        quote! { (#(#items),*) }
    }
}

#[cfg(test)]
#[path = "tests/service_error_tests.rs"]
mod tests;
