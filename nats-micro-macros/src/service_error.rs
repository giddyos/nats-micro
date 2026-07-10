use std::collections::BTreeSet;

use heck::{AsShoutySnakeCase, ToSnakeCase};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, Path, Token, parse::Parse, parse::ParseStream,
    punctuated::Punctuated,
};

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

    if cfg!(feature = "macros_client_feature") && !matches!(input.vis, syn::Visibility::Public(_)) {
        return error_stream(
            span,
            "#[service_error] enums must be declared `pub` when the `client` feature is enabled",
            &input,
        );
    }

    let mode = match normalize_service_error_derives(&mut input) {
        Ok(mode) => mode,
        Err(error) => return error.to_compile_error(),
    };

    let mut display_arms = Vec::new();
    let mut source_arms = Vec::new();
    let mut from_impls = Vec::new();
    let mut variant_arms = Vec::new();
    let mut from_response_arms = Vec::new();
    let mut napi_js_variants = Vec::new();
    let mut napi_as_ref_arms = Vec::new();
    let mut napi_from_error_arms = Vec::new();
    let mut wire_kinds = BTreeSet::<(u16, String)>::new();
    let js_enum_ident = format_ident!("Js{}", enum_ident);

    for variant in &data_enum.variants {
        let v_ident = &variant.ident;
        let v_name = v_ident.to_string();
        let default_kind = format!("{}", AsShoutySnakeCase(&v_name));
        let js_variant_ident = format_ident!("{}", default_kind);
        let error_attr = if mode == ErrorImplMode::SelfContained {
            match variant_error_attr_strict(variant) {
                Ok(attr) => Some(attr),
                Err(error) => return error.to_compile_error(),
            }
        } else {
            None
        };
        let is_transparent = match mode {
            ErrorImplMode::SelfContained => matches!(error_attr, Some(ErrorAttr::Transparent)),
            ErrorImplMode::ExternalDerive => variant_is_transparent_light(variant),
        };
        let field_meta = match FieldMeta::for_variant(variant, mode, is_transparent) {
            Ok(meta) => meta,
            Err(error) => return error.to_compile_error(),
        };
        let code_attr = match variant_code_attr(variant) {
            Ok(code) => code,
            Err(error) => return error.to_compile_error(),
        };
        let internal_attr = match variant_internal_attr(variant) {
            Ok(internal) => internal,
            Err(error) => return error.to_compile_error(),
        };
        let kind_attr = match variant_kind_attr(variant) {
            Ok(kind) => kind,
            Err(error) => return error.to_compile_error(),
        };

        let is_internal = internal_attr.present
            || ((field_meta.has_from || is_transparent) && code_attr.is_none());
        let code = code_attr.unwrap_or(if is_internal { 500 } else { 400 });
        let wire_kind = kind_attr.unwrap_or_else(|| {
            if is_internal && !internal_attr.expose_kind {
                "INTERNAL_ERROR".to_string()
            } else {
                default_kind.clone()
            }
        });
        let generic_internal_kind = is_internal && wire_kind == "INTERNAL_ERROR";
        if !generic_internal_kind && !wire_kinds.insert((code, wire_kind.clone())) {
            return syn::Error::new_spanned(
                variant,
                format!(
                    "duplicate service_error wire kind `{wire_kind}` for code {code}; rename one variant or use #[kind(...)]"
                ),
            )
            .to_compile_error();
        }

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

        if let Some(error_attr) = &error_attr {
            let display_arm = match display_arm_tokens(&enum_ident, variant, error_attr) {
                Ok(tokens) => tokens,
                Err(error) => return error.to_compile_error(),
            };
            let source_arm = source_arm_tokens(&enum_ident, variant, &field_meta, error_attr);
            from_impls.extend(from_impl_tokens(
                &enum_ident,
                &impl_generics,
                &ty_generics,
                where_clause,
                variant,
                &field_meta,
            ));

            display_arms.push(display_arm);
            source_arms.push(source_arm);
        }
        napi_js_variants.push(quote! { #js_variant_ident });
        napi_as_ref_arms.push(quote! {
            Self::#js_variant_ident => #wire_kind,
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
                    (#code, #wire_kind) => #nats_micro::ServiceErrorMatch::Typed(#enum_ident::#v_ident),
                },
            ),
            Fields::Unnamed(fields)
                if fields.unnamed.len() == 1 && field_meta.wire_indices().len() == 1 =>
            {
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
                            (#code, #wire_kind) => #nats_micro::ServiceErrorMatch::Typed(
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
                        &wire_kind,
                        fields,
                        &field_meta,
                        is_internal,
                    ),
                }
            }
            Fields::Unnamed(fields) => build_unnamed_variant_tokens(
                &enum_ident,
                v_ident,
                code,
                &wire_kind,
                fields,
                &field_meta,
                is_internal,
            ),
            Fields::Named(fields) => build_named_variant_tokens(
                &enum_ident,
                v_ident,
                code,
                &wire_kind,
                fields,
                &field_meta,
                is_internal,
            ),
        };
        let from_response = if is_internal {
            if generic_internal_kind {
                quote! {}
            } else {
                quote! {
                    (#code, #wire_kind) => #nats_micro::ServiceErrorMatch::Untyped(response),
                }
            }
        } else {
            from_response
        };

        variant_arms.push(quote! {
            #pattern => {
                let __error = #nats_micro::NatsErrorResponse::new(
                    #code,
                    #wire_kind,
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

    strip_service_error_only_attrs(&mut input, mode);

    let display_impl = match mode {
        ErrorImplMode::SelfContained => quote! {
            impl #impl_generics ::std::fmt::Display for #enum_ident #ty_generics #where_clause {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    match self {
                        #(#display_arms)*
                    }
                }
            }

            impl #impl_generics ::std::error::Error for #enum_ident #ty_generics #where_clause {
                fn source(&self) -> ::std::option::Option<&(dyn ::std::error::Error + 'static)> {
                    match self {
                        #(#source_arms)*
                        _ => None,
                    }
                }
            }
        },
        ErrorImplMode::ExternalDerive => quote! {},
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

        #(#from_impls)*
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorImplMode {
    SelfContained,
    ExternalDerive,
}

#[allow(clippy::unnecessary_wraps)]
fn normalize_service_error_derives(input: &mut DeriveInput) -> Result<ErrorImplMode, syn::Error> {
    let mut has_debug = false;
    let mut has_error = false;
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
                has_error = true;
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
    Ok(if has_error {
        ErrorImplMode::ExternalDerive
    } else {
        ErrorImplMode::SelfContained
    })
}

fn variant_error_attr_strict(variant: &syn::Variant) -> Result<ErrorAttr, syn::Error> {
    let Some(attr) = variant
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("error"))
    else {
        return Err(syn::Error::new_spanned(
            variant,
            "service_error variants require #[error(\"...\")] or #[error(transparent)]",
        ));
    };

    attr.parse_args::<ErrorAttr>()
        .map_err(|error| syn::Error::new_spanned(attr, error))
}

fn variant_is_transparent_light(variant: &syn::Variant) -> bool {
    variant
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("error"))
        .is_some_and(|attr| {
            attr.parse_args_with(|input: ParseStream<'_>| {
                let ident: syn::Ident = input.parse()?;
                if ident == "transparent" && input.is_empty() {
                    Ok(())
                } else {
                    Err(input.error("not transparent"))
                }
            })
            .is_ok()
        })
}

fn display_arm_tokens(
    enum_ident: &syn::Ident,
    variant: &syn::Variant,
    error_attr: &ErrorAttr,
) -> Result<TokenStream, syn::Error> {
    let v_ident = &variant.ident;

    if matches!(error_attr, ErrorAttr::Transparent) {
        return transparent_display_arm_tokens(enum_ident, variant);
    }

    let ErrorAttr::Message {
        message,
        args: explicit_args,
    } = error_attr
    else {
        unreachable!("transparent handled above");
    };
    let (format_string, placeholders) = parse_error_format(message)?;
    let format_lit = syn::LitStr::new(&format_string, message.span());

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
            let args = display_args_tokens(variant, &bindings, &[], &placeholders, explicit_args)?;

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
            let args = display_args_tokens(
                variant,
                &bindings,
                &field_idents,
                &placeholders,
                explicit_args,
            )?;

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
    explicit_args: &[FormatArg],
) -> Result<Vec<TokenStream>, syn::Error> {
    let mut args = Vec::with_capacity(placeholders.len());
    let mut next_implicit = 0usize;
    let mut next_explicit = 0usize;

    for placeholder in placeholders {
        if !explicit_args.is_empty() {
            if !matches!(placeholder.key, PlaceholderKey::Implicit) {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[service_error] does not support mixing explicit error format arguments with named or indexed placeholders",
                ));
            }

            let Some(arg) = explicit_args.get(next_explicit) else {
                return Err(syn::Error::new_spanned(
                    variant,
                    "service_error format placeholder has no matching explicit argument",
                ));
            };
            next_explicit += 1;
            args.push(format_arg_tokens(variant, arg, bindings, field_idents)?);
            continue;
        }

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

    if next_explicit != explicit_args.len() {
        return Err(syn::Error::new_spanned(
            variant,
            "#[service_error] does not support thiserror expression argument syntax here; use a simpler format or implement IntoNatsError manually",
        ));
    }

    Ok(args)
}

fn format_arg_tokens(
    variant: &syn::Variant,
    arg: &FormatArg,
    bindings: &[syn::Ident],
    field_idents: &[syn::Ident],
) -> Result<TokenStream, syn::Error> {
    let index = match arg {
        FormatArg::Index(index) => *index,
        FormatArg::Name(name) => field_idents
            .iter()
            .position(|ident| ident == name)
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    variant,
                    format!("unknown service_error explicit format field `{name}`"),
                )
            })?,
    };

    let Some(binding) = bindings.get(index) else {
        return Err(syn::Error::new_spanned(
            variant,
            format!("service_error explicit format field `{index}` is out of bounds"),
        ));
    };

    Ok(quote! { #binding })
}

fn transparent_display_arm_tokens(
    enum_ident: &syn::Ident,
    variant: &syn::Variant,
) -> Result<TokenStream, syn::Error> {
    let v_ident = &variant.ident;

    match &variant.fields {
        Fields::Unit => Err(syn::Error::new_spanned(
            variant,
            "#[error(transparent)] requires exactly one field",
        )),
        Fields::Unnamed(fields) => {
            if fields.unnamed.len() != 1 {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[error(transparent)] requires exactly one field",
                ));
            }
            Ok(quote! {
                #enum_ident::#v_ident(__source) => ::std::fmt::Display::fmt(__source, f),
            })
        }
        Fields::Named(fields) => {
            if fields.named.len() != 1 {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[error(transparent)] requires exactly one field",
                ));
            }
            let field_ident = fields
                .named
                .first()
                .and_then(|field| field.ident.clone())
                .expect("named field");
            Ok(quote! {
                #enum_ident::#v_ident { #field_ident: __source } => ::std::fmt::Display::fmt(__source, f),
            })
        }
    }
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

enum ErrorAttr {
    Message {
        message: syn::LitStr,
        args: Vec<FormatArg>,
    },
    Transparent,
}

impl Parse for ErrorAttr {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(syn::Ident) {
            let lookahead = input.fork();
            let ident: syn::Ident = lookahead.parse()?;
            if ident == "transparent" && lookahead.is_empty() {
                input.parse::<syn::Ident>()?;
                return Ok(Self::Transparent);
            }
        }

        let message: syn::LitStr = input.parse()?;
        let mut args = Vec::new();
        while input.parse::<Option<Token![,]>>()?.is_some() {
            if input.is_empty() {
                break;
            }
            args.push(input.parse()?);
        }

        Ok(Self::Message { message, args })
    }
}

enum FormatArg {
    Index(usize),
    Name(syn::Ident),
}

impl Parse for FormatArg {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![.]>()?;
        if input.peek(syn::LitInt) {
            let lit: syn::LitInt = input.parse()?;
            let index = lit.base10_parse()?;
            return Ok(Self::Index(index));
        }
        if input.peek(syn::Ident) {
            return Ok(Self::Name(input.parse()?));
        }
        Err(input.error(
            "#[service_error] does not support thiserror expression argument syntax here; use a simpler format or implement IntoNatsError manually",
        ))
    }
}

struct FieldMeta {
    roles: Vec<FieldRole>,
    from_index: Option<usize>,
    source_index: Option<usize>,
    has_from: bool,
    details_skip_all: bool,
}

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct FieldRole {
    source: bool,
    from: bool,
    backtrace: bool,
    details_skip: bool,
}

impl FieldMeta {
    fn for_variant(
        variant: &syn::Variant,
        mode: ErrorImplMode,
        is_transparent: bool,
    ) -> Result<Self, syn::Error> {
        let mut roles = Vec::new();
        let mut source_index = None;
        let mut from_index = None;
        let details_skip_all = variant_details_skip_all(variant)?;

        for (index, field) in variant.fields.iter().enumerate() {
            let from = field_has_attr(field, "from");
            let explicit_source = field_has_attr(field, "source");
            let implicit_source = field.ident.as_ref().is_some_and(|ident| ident == "source");
            let source = from || explicit_source || implicit_source;
            let backtrace = field_has_attr(field, "backtrace");
            let details_skip = field_details_skip(field)?;

            if from && from_index.replace(index).is_some() && mode == ErrorImplMode::SelfContained {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[from] is only valid on one field per service_error variant",
                ));
            }

            if source
                && source_index.replace(index).is_some()
                && mode == ErrorImplMode::SelfContained
            {
                return Err(syn::Error::new_spanned(
                    field,
                    "#[source] is only valid on one field per service_error variant",
                ));
            }

            roles.push(FieldRole {
                source,
                from,
                backtrace,
                details_skip,
            });
        }

        if mode == ErrorImplMode::SelfContained
            && let Some(index) = from_index
            && roles.len() != 1
        {
            let field = variant.fields.iter().nth(index).expect("from field index");
            return Err(syn::Error::new_spanned(
                field,
                "#[from] is only valid on a single-field service_error variant",
            ));
        }

        if is_transparent && mode == ErrorImplMode::SelfContained {
            if roles.len() != 1 {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[error(transparent)] requires exactly one field",
                ));
            }
            roles[0].source = true;
            source_index = Some(0);
        } else if is_transparent && roles.len() == 1 {
            roles[0].source = true;
            source_index = Some(0);
        }

        Ok(Self {
            roles,
            from_index,
            source_index,
            has_from: from_index.is_some(),
            details_skip_all,
        })
    }

    fn wire_indices(&self) -> Vec<usize> {
        self.roles
            .iter()
            .enumerate()
            .filter_map(|(index, role)| {
                (!self.details_skip_all
                    && !role.source
                    && !role.from
                    && !role.backtrace
                    && !role.details_skip)
                    .then_some(index)
            })
            .collect()
    }

    fn has_skipped_wire_fields(&self) -> bool {
        (self.details_skip_all && !self.roles.is_empty())
            || self
                .roles
                .iter()
                .any(|role| role.source || role.from || role.backtrace || role.details_skip)
    }
}

fn field_has_attr(field: &syn::Field, name: &str) -> bool {
    field.attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn strip_service_error_only_attrs(input: &mut DeriveInput, mode: ErrorImplMode) {
    if let Data::Enum(ref mut data_enum) = input.data {
        for variant in &mut data_enum.variants {
            variant.attrs.retain(|attr| {
                !attr.path().is_ident("internal")
                    && !attr.path().is_ident("code")
                    && !attr.path().is_ident("kind")
                    && !attr.path().is_ident("details")
                    && match mode {
                        ErrorImplMode::SelfContained => !attr.path().is_ident("error"),
                        ErrorImplMode::ExternalDerive => true,
                    }
            });
            for field in &mut variant.fields {
                field.attrs.retain(|attr| match mode {
                    ErrorImplMode::SelfContained => {
                        !attr.path().is_ident("from")
                            && !attr.path().is_ident("source")
                            && !attr.path().is_ident("backtrace")
                            && !attr.path().is_ident("details")
                    }
                    ErrorImplMode::ExternalDerive => !attr.path().is_ident("details"),
                });
            }
        }
    }
}

fn source_arm_tokens(
    enum_ident: &syn::Ident,
    variant: &syn::Variant,
    field_meta: &FieldMeta,
    error_attr: &ErrorAttr,
) -> TokenStream {
    let Some(source_index) = field_meta
        .source_index
        .or_else(|| matches!(error_attr, ErrorAttr::Transparent).then_some(0))
    else {
        return quote! {};
    };

    let v_ident = &variant.ident;
    match &variant.fields {
        Fields::Unit => quote! {},
        Fields::Unnamed(fields) => {
            let bindings: Vec<_> = (0..fields.unnamed.len())
                .map(|index| {
                    if index == source_index {
                        format_ident!("__source")
                    } else {
                        format_ident!("__field_{index}")
                    }
                })
                .collect();
            quote! {
                #enum_ident::#v_ident(#(#bindings),*) => {
                    let __source: &(dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync + 'static) = __source;
                    Some(__source)
                }
            }
        }
        Fields::Named(fields) => {
            let field_idents: Vec<_> = fields
                .named
                .iter()
                .map(|field| field.ident.clone().expect("named field"))
                .collect();
            let bindings: Vec<_> = field_idents
                .iter()
                .enumerate()
                .map(|(index, ident)| {
                    if index == source_index {
                        format_ident!("__source")
                    } else {
                        format_ident!("__{ident}")
                    }
                })
                .collect();
            quote! {
                #enum_ident::#v_ident { #(#field_idents: #bindings),* } => {
                    let __source: &(dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync + 'static) = __source;
                    Some(__source)
                }
            }
        }
    }
}

fn from_impl_tokens(
    enum_ident: &syn::Ident,
    impl_generics: &syn::ImplGenerics<'_>,
    ty_generics: &syn::TypeGenerics<'_>,
    where_clause: Option<&syn::WhereClause>,
    variant: &syn::Variant,
    field_meta: &FieldMeta,
) -> Vec<TokenStream> {
    let Some(from_index) = field_meta.from_index else {
        return Vec::new();
    };

    let v_ident = &variant.ident;
    let source_ty = variant
        .fields
        .iter()
        .nth(from_index)
        .map(|field| &field.ty)
        .expect("from field type");

    let body = match &variant.fields {
        Fields::Unnamed(_) => quote! { #enum_ident::#v_ident(value) },
        Fields::Named(fields) => {
            let field_ident = fields
                .named
                .iter()
                .nth(from_index)
                .and_then(|field| field.ident.as_ref())
                .expect("from named field");
            quote! { #enum_ident::#v_ident { #field_ident: value } }
        }
        Fields::Unit => return Vec::new(),
    };

    vec![quote! {
        impl #impl_generics ::std::convert::From<#source_ty> for #enum_ident #ty_generics #where_clause {
            fn from(value: #source_ty) -> Self {
                #body
            }
        }
    }]
}

#[derive(Default)]
struct InternalAttr {
    present: bool,
    expose_kind: bool,
}

fn variant_code_attr(variant: &syn::Variant) -> Result<Option<u16>, syn::Error> {
    let mut code = None;
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("code"))
    {
        if code.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "#[code(...)] may only be specified once per service_error variant",
            ));
        }

        let lit = attr.parse_args::<syn::LitInt>().map_err(|_| {
            syn::Error::new_spanned(attr, "#[code(...)] requires an integer literal")
        })?;
        let value = lit.base10_parse::<u16>().map_err(|_| {
            syn::Error::new_spanned(attr, "#[code(...)] must be in the range 400..=599")
        })?;
        if !(400..=599).contains(&value) {
            return Err(syn::Error::new_spanned(
                attr,
                "#[code(...)] must be in the range 400..=599",
            ));
        }
        code = Some(value);
    }
    Ok(code)
}

fn variant_internal_attr(variant: &syn::Variant) -> Result<InternalAttr, syn::Error> {
    let mut parsed = InternalAttr::default();
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("internal"))
    {
        if parsed.present {
            return Err(syn::Error::new_spanned(
                attr,
                "#[internal] may only be specified once per service_error variant",
            ));
        }

        let expose_kind = if matches!(attr.meta, syn::Meta::Path(_)) {
            false
        } else {
            attr.parse_args_with(|input: ParseStream<'_>| {
                let ident: syn::Ident = input.parse()?;
                if ident == "expose_kind" && input.is_empty() {
                    Ok(true)
                } else {
                    Err(input.error("#[internal] only accepts optional expose_kind"))
                }
            })
            .map_err(|_| {
                syn::Error::new_spanned(
                    attr,
                    "#[internal] only accepts #[internal] or #[internal(expose_kind)]",
                )
            })?
        };

        parsed.present = true;
        parsed.expose_kind = expose_kind;
    }
    Ok(parsed)
}

fn variant_kind_attr(variant: &syn::Variant) -> Result<Option<String>, syn::Error> {
    let mut kind = None;
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("kind"))
    {
        if kind.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "#[kind(...)] may only be specified once per service_error variant",
            ));
        }

        let lit = attr
            .parse_args::<syn::LitStr>()
            .map_err(|_| syn::Error::new_spanned(attr, "#[kind(...)] requires a string literal"))?;
        let value = lit.value();
        if value.is_empty() {
            return Err(syn::Error::new_spanned(
                attr,
                "#[kind(...)] cannot be empty",
            ));
        }
        kind = Some(value);
    }
    Ok(kind)
}

fn variant_details_skip_all(variant: &syn::Variant) -> Result<bool, syn::Error> {
    let mut skip_all = false;
    for attr in variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("details"))
    {
        if skip_all {
            return Err(syn::Error::new_spanned(
                attr,
                "#[details(...)] may only be specified once per service_error variant",
            ));
        }
        skip_all = attr
            .parse_args_with(|input: ParseStream<'_>| {
                let ident: syn::Ident = input.parse()?;
                if (ident == "skip" || ident == "skip_all") && input.is_empty() {
                    Ok(true)
                } else {
                    Err(input.error("#[details(...)] on a variant only accepts skip or skip_all"))
                }
            })
            .map_err(|_| {
                syn::Error::new_spanned(
                    attr,
                    "#[details(...)] on a variant only accepts skip or skip_all",
                )
            })?;
    }
    Ok(skip_all)
}

fn field_details_skip(field: &syn::Field) -> Result<bool, syn::Error> {
    let mut skip = false;
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("details"))
    {
        if skip {
            return Err(syn::Error::new_spanned(
                attr,
                "#[details(skip)] may only be specified once per service_error field",
            ));
        }
        skip = attr
            .parse_args_with(|input: ParseStream<'_>| {
                let ident: syn::Ident = input.parse()?;
                if ident == "skip" && input.is_empty() {
                    Ok(true)
                } else {
                    Err(input.error("#[details(...)] on a field only accepts skip"))
                }
            })
            .map_err(|_| {
                syn::Error::new_spanned(attr, "#[details(...)] on a field only accepts skip")
            })?;
    }
    Ok(skip)
}

fn build_unnamed_variant_tokens(
    enum_ident: &syn::Ident,
    variant_ident: &syn::Ident,
    code: u16,
    variant_name: &str,
    fields: &syn::FieldsUnnamed,
    field_meta: &FieldMeta,
    is_internal: bool,
) -> (TokenStream, TokenStream, TokenStream) {
    let nats_micro = nats_micro_path();
    // Public tuple variants are serialized as positional tuples rather than
    // object maps so both tuple and named variants can share one stable,
    // order-based round-trip format.
    let bindings: Vec<syn::Ident> = (0..fields.unnamed.len())
        .map(|index| syn::Ident::new(&format!("__field_{index}"), proc_macro2::Span::call_site()))
        .collect();
    let wire_indices = field_meta.wire_indices();
    let wire_bindings: Vec<_> = wire_indices.iter().map(|index| &bindings[*index]).collect();
    let details = if is_internal || wire_bindings.is_empty() {
        quote! { None }
    } else {
        let serialize_value = singleton_or_tuple(&wire_bindings);
        quote! { #nats_micro::serde_json::to_value(#serialize_value).ok() }
    };
    let from_response = if field_meta.has_skipped_wire_fields() {
        quote! {
            (#code, #variant_name) => #nats_micro::ServiceErrorMatch::Untyped(response),
        }
    } else {
        let types: Vec<_> = fields.unnamed.iter().map(|field| &field.ty).collect();
        let tuple_type = singleton_or_tuple(&types);
        let deserialize_pattern = singleton_or_tuple(&bindings);
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
        }
    };

    (
        quote! { #enum_ident::#variant_ident(#(#bindings),*) },
        details,
        from_response,
    )
}

fn build_named_variant_tokens(
    enum_ident: &syn::Ident,
    variant_ident: &syn::Ident,
    code: u16,
    variant_name: &str,
    fields: &syn::FieldsNamed,
    field_meta: &FieldMeta,
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
    let wire_indices = field_meta.wire_indices();
    let wire_bindings: Vec<_> = wire_indices.iter().map(|index| &bindings[*index]).collect();
    let details = if is_internal || wire_bindings.is_empty() {
        quote! { None }
    } else {
        let serialize_value = singleton_or_tuple(&wire_bindings);
        quote! { #nats_micro::serde_json::to_value(#serialize_value).ok() }
    };
    let from_response = if field_meta.has_skipped_wire_fields() {
        quote! {
            (#code, #variant_name) => #nats_micro::ServiceErrorMatch::Untyped(response),
        }
    } else {
        let types: Vec<_> = fields.named.iter().map(|field| &field.ty).collect();
        let tuple_type = singleton_or_tuple(&types);
        let deserialize_pattern = singleton_or_tuple(&bindings);
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
        }
    };

    (
        quote! { #enum_ident::#variant_ident { #(#field_idents: #bindings),* } },
        details,
        from_response,
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
