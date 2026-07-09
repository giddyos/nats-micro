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

    if let Err(error) = normalize_service_error_derives(&mut input) {
        return error.to_compile_error();
    }

    let mut display_arms = Vec::new();
    let mut source_arms = Vec::new();
    let mut from_impls = Vec::new();
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
        let error_attr = match variant_error_attr(variant) {
            Ok(attr) => attr,
            Err(error) => return error.to_compile_error(),
        };
        let field_meta = match FieldMeta::for_variant(variant) {
            Ok(meta) => meta,
            Err(error) => return error.to_compile_error(),
        };

        let is_internal = variant_is_internal(variant)
            || ((field_meta.has_from || matches!(error_attr, ErrorAttr::Transparent))
                && !variant_has_code(variant));
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

        let display_arm = match display_arm_tokens(&enum_ident, variant, &error_attr) {
            Ok(tokens) => tokens,
            Err(error) => return error.to_compile_error(),
        };
        let source_arm = match source_arm_tokens(&enum_ident, variant, &field_meta, &error_attr) {
            Ok(tokens) => tokens,
            Err(error) => return error.to_compile_error(),
        };
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
                        &field_meta,
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
                &field_meta,
                is_internal,
            ),
            Fields::Named(fields) => build_named_variant_tokens(
                &enum_ident,
                v_ident,
                code,
                &v_code,
                fields,
                &field_meta,
                is_internal,
            ),
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
            for field in &mut variant.fields {
                field.attrs.retain(|attr| {
                    !attr.path().is_ident("from")
                        && !attr.path().is_ident("source")
                        && !attr.path().is_ident("backtrace")
                });
            }
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

        impl #impl_generics ::std::error::Error for #enum_ident #ty_generics #where_clause {
            fn source(&self) -> ::std::option::Option<&(dyn ::std::error::Error + 'static)> {
                match self {
                    #(#source_arms)*
                    _ => None,
                }
            }
        }
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

fn normalize_service_error_derives(input: &mut DeriveInput) -> Result<(), syn::Error> {
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
                return Err(syn::Error::new_spanned(
                    path,
                    "#[service_error] already implements Display and Error; remove the Error derive",
                ));
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
    Ok(())
}

fn variant_error_attr(variant: &syn::Variant) -> Result<ErrorAttr, syn::Error> {
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
}

#[derive(Clone, Copy)]
struct FieldRole {
    source: bool,
    from: bool,
    backtrace: bool,
}

impl FieldMeta {
    fn for_variant(variant: &syn::Variant) -> Result<Self, syn::Error> {
        let mut roles = Vec::new();
        let mut source_index = None;
        let mut from_index = None;

        for (index, field) in variant.fields.iter().enumerate() {
            let from = field_has_attr(field, "from");
            let explicit_source = field_has_attr(field, "source");
            let implicit_source = field.ident.as_ref().is_some_and(|ident| ident == "source");
            let source = from || explicit_source || implicit_source;
            let backtrace = field_has_attr(field, "backtrace");

            if from {
                if from_index.replace(index).is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "#[from] is only valid on one field per service_error variant",
                    ));
                }
            }

            if source {
                if source_index.replace(index).is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "#[source] is only valid on one field per service_error variant",
                    ));
                }
            }

            roles.push(FieldRole {
                source,
                from,
                backtrace,
            });
        }

        if let Some(index) = from_index {
            if roles.len() != 1 {
                let field = variant.fields.iter().nth(index).expect("from field index");
                return Err(syn::Error::new_spanned(
                    field,
                    "#[from] is only valid on a single-field service_error variant",
                ));
            }
        }

        if matches!(variant_error_attr(variant)?, ErrorAttr::Transparent) {
            if roles.len() != 1 {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[error(transparent)] requires exactly one field",
                ));
            }
            roles[0].source = true;
            source_index = Some(0);
        }

        Ok(Self {
            roles,
            from_index,
            source_index,
            has_from: from_index.is_some(),
        })
    }

    fn wire_indices(&self) -> Vec<usize> {
        self.roles
            .iter()
            .enumerate()
            .filter_map(|(index, role)| {
                (!role.source && !role.from && !role.backtrace).then_some(index)
            })
            .collect()
    }

    fn has_skipped_wire_fields(&self) -> bool {
        self.roles
            .iter()
            .any(|role| role.source || role.from || role.backtrace)
    }
}

fn field_has_attr(field: &syn::Field, name: &str) -> bool {
    field.attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn source_arm_tokens(
    enum_ident: &syn::Ident,
    variant: &syn::Variant,
    field_meta: &FieldMeta,
    error_attr: &ErrorAttr,
) -> Result<TokenStream, syn::Error> {
    let Some(source_index) = field_meta
        .source_index
        .or_else(|| matches!(error_attr, ErrorAttr::Transparent).then_some(0))
    else {
        return Ok(quote! {});
    };

    let v_ident = &variant.ident;
    match &variant.fields {
        Fields::Unit => Ok(quote! {}),
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
            Ok(quote! {
                #enum_ident::#v_ident(#(#bindings),*) => {
                    let __source: &(dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync + 'static) = __source;
                    Some(__source)
                }
            })
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
            Ok(quote! {
                #enum_ident::#v_ident { #(#field_idents: #bindings),* } => {
                    let __source: &(dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync + 'static) = __source;
                    Some(__source)
                }
            })
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

fn variant_has_code(variant: &syn::Variant) -> bool {
    variant
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("code"))
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
