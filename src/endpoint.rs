use std::collections::BTreeSet;

use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, GenericArgument, ImplItemFn, Pat, PathArguments, ReturnType, Signature, Type};

use crate::{
    client::{ClientEndpointSpec, build_endpoint_client_spec},
    service::GeneratedHandlerItem,
    utils::{conditional_attrs, nats_micro_path, parse_attr},
};

#[derive(Debug, FromMeta)]
pub(crate) struct EndpointArgs {
    pub subject: String,
    pub group: Option<String>,
    pub queue_group: Option<String>,
    pub concurrency_limit: Option<u64>,
}

pub(crate) fn process_endpoint_method(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    attr: &syn::Attribute,
) -> Result<(GeneratedHandlerItem, ClientEndpointSpec), TokenStream> {
    let args = parse_attr::<EndpointArgs>(attr)?;
    let attrs = conditional_attrs(method);
    let client_endpoint = build_endpoint_client_spec(method, &args, attrs.clone())?;
    let generated = build_endpoint_handler_item(struct_ident, method, &args, &client_endpoint)?;
    Ok((generated, client_endpoint))
}

fn build_endpoint_handler_item(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    args: &EndpointArgs,
    client_endpoint: &ClientEndpointSpec,
) -> Result<GeneratedHandlerItem, TokenStream> {
    let nats_micro = nats_micro_path();
    let auth_required = requires_auth(&method.sig);
    let fn_name = &method.sig.ident;
    let subject = &client_endpoint.subject.template;
    let group = &client_endpoint.group;
    let nats_subject = &client_endpoint.subject.pattern;
    let subject_template = if client_endpoint.subject.is_templated() {
        quote! { Some(#subject.to_string()) }
    } else {
        quote! { None }
    };
    let queue_group = optional_string_tokens(args.queue_group.as_deref());
    let concurrency_limit = optional_u64_tokens(args.concurrency_limit);
    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;
    let payload_meta = payload_meta_tokens(client_endpoint.payload_meta.as_ref(), &nats_micro);
    let response_meta = response_meta_tokens(&client_endpoint.response_meta, &nats_micro);
    let def_method_name = format_ident!("__ep_{}", fn_name);
    let accessor_name = format_ident!("{}_endpoint", fn_name);
    let fn_name_str = fn_name.to_string();

    Ok(GeneratedHandlerItem {
        attrs: conditional_attrs(method),
        def_fn: quote! {
            #[doc(hidden)]
            pub fn #def_method_name() -> #nats_micro::EndpointDefinition {
                let __meta = Self::__nats_micro_service_meta();
                #nats_micro::EndpointDefinition {
                    subject_prefix: __meta.subject_prefix.clone(),
                    service_name: __meta.name,
                    service_version: __meta.version,
                    service_description: __meta.description,
                    fn_name: #fn_name_str.to_string(),
                    group: #group.to_string(),
                    subject: #nats_subject.to_string(),
                    subject_template: #subject_template,
                    queue_group: #queue_group,
                    auth_required: #auth_required,
                    concurrency_limit: #concurrency_limit,
                    handler: #handler,
                }
            }
        },
        accessor_fn: quote! {
            pub fn #accessor_name() -> #nats_micro::EndpointDefinition {
                Self::#def_method_name()
            }
        },
        def_call: quote! { Self::#def_method_name() },
        info_expr: quote! {
            #nats_micro::__macros::EndpointInfo {
                fn_name: #fn_name_str.to_string(),
                subject_template: #subject.to_string(),
                subject_pattern: #nats_subject.to_string(),
                group: #group.to_string(),
                queue_group: #queue_group,
                auth_required: #auth_required,
                concurrency_limit: #concurrency_limit,
                params: vec![#(#param_infos),*],
                payload_meta: #payload_meta,
                response_meta: #response_meta,
            }
        },
    })
}

fn optional_string_tokens(value: Option<&str>) -> TokenStream {
    if let Some(value) = value {
        quote! { Some(#value.to_string()) }
    } else {
        quote! { None }
    }
}

fn optional_u64_tokens(value: Option<u64>) -> TokenStream {
    if let Some(value) = value {
        quote! { Some(#value) }
    } else {
        quote! { None }
    }
}

fn payload_meta_tokens(payload_meta: Option<&PayloadMeta>, nats_micro: &syn::Path) -> TokenStream {
    if let Some(payload_meta) = payload_meta {
        let encoding = payload_encoding_tokens(&payload_meta.encoding, nats_micro);
        let encrypted = payload_meta.encrypted;
        let inner_type = &payload_meta.inner_type;
        let inner_type_str = quote!(#inner_type).to_string();
        quote! {
            Some(#nats_micro::__macros::PayloadMeta {
                encoding: #encoding,
                encrypted: #encrypted,
                inner_type: #inner_type_str.to_string(),
            })
        }
    } else {
        quote! { None }
    }
}

fn response_meta_tokens(response_meta: &ResponseMeta, nats_micro: &syn::Path) -> TokenStream {
    let encoding = response_encoding_tokens(&response_meta.encoding, nats_micro);
    let encrypted = response_meta.encrypted;
    let inner_type = response_meta
        .inner_type
        .as_ref()
        .map(|inner_type| quote!(#inner_type).to_string())
        .unwrap_or_default();

    quote! {
        #nats_micro::__macros::ResponseMeta {
            encoding: #encoding,
            encrypted: #encrypted,
            inner_type: #inner_type.to_string(),
        }
    }
}

fn payload_encoding_tokens(enc: &PayloadEncoding, nats_micro: &syn::Path) -> TokenStream {
    match enc {
        PayloadEncoding::Json => quote! { #nats_micro::__macros::PayloadEncoding::Json },
        PayloadEncoding::Proto => quote! { #nats_micro::__macros::PayloadEncoding::Proto },
        PayloadEncoding::Serde => quote! { #nats_micro::__macros::PayloadEncoding::Serde },
        PayloadEncoding::Raw => quote! { #nats_micro::__macros::PayloadEncoding::Raw },
    }
}

fn response_encoding_tokens(enc: &ResponseEncoding, nats_micro: &syn::Path) -> TokenStream {
    match enc {
        ResponseEncoding::Json => quote! { #nats_micro::__macros::ResponseEncoding::Json },
        ResponseEncoding::Proto => quote! { #nats_micro::__macros::ResponseEncoding::Proto },
        ResponseEncoding::Serde => quote! { #nats_micro::__macros::ResponseEncoding::Serde },
        ResponseEncoding::Raw => quote! { #nats_micro::__macros::ResponseEncoding::Raw },
        ResponseEncoding::Unit => quote! { #nats_micro::__macros::ResponseEncoding::Unit },
    }
}

pub(crate) fn build_handler_body(
    fn_path: &TokenStream,
    sig: &Signature,
) -> Result<TokenStream, TokenStream> {
    let nats_micro = nats_micro_path();
    let requires_shutdown_signal = requires_shutdown_signal(sig);
    let mut extractors = Vec::new();
    let mut args = Vec::new();

    for input in &sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(
                syn::Error::new_spanned(input, "handlers cannot take a receiver")
                    .to_compile_error(),
            );
        };

        let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "handler arguments must be simple identifiers",
            )
            .to_compile_error());
        };

        let ident = &pat_ident.ident;
        let ty = &pat_type.ty;
        let ctx_expr = if is_subject_param(ty) {
            let param_name = subject_param_template_name(&ident.to_string()).to_string();
            quote! { &ctx.with_param_name(#param_name) }
        } else {
            quote! { &ctx }
        };

        extractors.push(quote! {
            let #ident: #ty = match <#ty as #nats_micro::__macros::FromRequest>::from_request(#ctx_expr).await {
                Ok(value) => value,
                Err(err) => return Err(err),
            };
        });
        args.push(quote! { #ident });
    }

    Ok(quote! {
        #nats_micro::HandlerFn::new_with_shutdown_signal_support(#requires_shutdown_signal, move |ctx: #nats_micro::__macros::RequestContext| {
            ::std::boxed::Box::pin(async move {
                let __request_id = ctx.request.request_id.clone();
                #(#extractors)*
                let response = #fn_path(#(#args),*)
                    .await
                    .map_err(|err| #nats_micro::__macros::IntoNatsError::into_nats_error(err, __request_id.clone()))?;
                #nats_micro::__macros::IntoNatsResponse::into_response(response, &ctx)
            })
        })
    })
}

pub(crate) fn extract_param_info(sig: &Signature) -> Result<Vec<TokenStream>, TokenStream> {
    let nats_micro = nats_micro_path();
    let mut params = Vec::new();
    for input in &sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(
                syn::Error::new_spanned(input, "handlers cannot take a receiver")
                    .to_compile_error(),
            );
        };
        let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "handler arguments must be simple identifiers",
            )
            .to_compile_error());
        };

        let name = pat_ident.ident.to_string();
        let ty = &pat_type.ty;
        let type_name = quote!(#ty).to_string();
        let is_sp = is_subject_param(ty);

        params.push(quote! {
            #nats_micro::__macros::ParamInfo {
                name: #name.to_string(),
                type_name: #type_name.to_string(),
                is_subject_param: #is_sp,
            }
        });
    }
    Ok(params)
}

pub(crate) fn is_subject_param(ty: &Type) -> bool {
    subject_param_inner_type(ty).is_some()
}

pub(crate) fn is_auth_type(ty: &Type) -> bool {
    last_segment_ident(ty).as_deref() == Some("Auth")
}

pub(crate) fn requires_auth(sig: &Signature) -> bool {
    sig.inputs.iter().any(|input| {
        let FnArg::Typed(pat_type) = input else {
            return false;
        };
        let ty = pat_type.ty.as_ref();
        is_auth_type(ty) && !is_option_auth(ty)
    })
}

pub(crate) fn is_shutdown_signal_type(ty: &Type) -> bool {
    last_segment_ident(ty).as_deref() == Some("ShutdownSignal")
}

pub(crate) fn requires_shutdown_signal(sig: &Signature) -> bool {
    sig.inputs.iter().any(|input| {
        let FnArg::Typed(pat_type) = input else {
            return false;
        };
        is_shutdown_signal_type(pat_type.ty.as_ref())
    })
}

pub(crate) fn parse_subject_template(
    subject: &str,
    ident: &syn::Ident,
) -> Result<(String, Vec<String>), TokenStream> {
    let mut rendered_segments = Vec::new();
    let mut params = Vec::new();

    for segment in subject.trim().split('.') {
        if segment.is_empty() {
            return Err(syn::Error::new_spanned(
                ident,
                format!("endpoint subject `{subject}` contains an empty segment"),
            )
            .to_compile_error());
        }

        if segment.starts_with('{') || segment.ends_with('}') {
            if !segment.starts_with('{') || !segment.ends_with('}') {
                return Err(syn::Error::new_spanned(
                    ident,
                    format!(
                        "endpoint subject `{subject}` has an invalid template segment `{segment}`"
                    ),
                )
                .to_compile_error());
            }

            let name = &segment[1..segment.len() - 1];
            if name.is_empty() || name.contains('{') || name.contains('}') {
                return Err(syn::Error::new_spanned(
                    ident,
                    format!(
                        "endpoint subject `{subject}` has an invalid template parameter `{segment}`"
                    ),
                )
                .to_compile_error());
            }

            rendered_segments.push("*".to_string());
            params.push(name.to_string());
            continue;
        }

        if segment.contains('{') || segment.contains('}') {
            return Err(syn::Error::new_spanned(
                ident,
                format!("endpoint subject `{subject}` has an invalid template segment `{segment}`"),
            )
            .to_compile_error());
        }

        rendered_segments.push(segment.to_string());
    }

    Ok((rendered_segments.join("."), params))
}

pub(crate) fn validate_template_bindings(
    sig: &Signature,
    template_params: &[String],
) -> Result<(), TokenStream> {
    let mut errors: Option<syn::Error> = None;
    let mut declared_params = BTreeSet::new();
    let template_param_names: BTreeSet<_> = template_params.iter().cloned().collect();

    if template_param_names.len() != template_params.len() {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            "endpoint subject templates must not reuse the same parameter name more than once",
        )
        .to_compile_error());
    }

    for input in &sig.inputs {
        let Some(subject_param) = inspect_subject_param(input)? else {
            continue;
        };

        if !template_param_names.contains(&subject_param.template_name) {
            combine_error(
                &mut errors,
                syn::Error::new(
                    subject_param.span,
                    format!(
                        "subject parameter `{}` is not declared in endpoint subject template",
                        subject_param.template_name
                    ),
                ),
            );
            continue;
        }

        if !declared_params.insert(subject_param.template_name.clone()) {
            combine_error(
                &mut errors,
                syn::Error::new(
                    subject_param.span,
                    format!(
                        "subject parameter `{}` is bound more than once in the handler signature",
                        subject_param.template_name
                    ),
                ),
            );
        }
    }

    for missing in template_params {
        if !declared_params.contains(missing) {
            combine_error(
                &mut errors,
                syn::Error::new_spanned(
                    &sig.ident,
                    format!(
                        "endpoint subject template requires handler argument `{missing}: SubjectParam<...>` or `_{missing}: SubjectParam<...>`"
                    ),
                ),
            );
        }
    }

    match errors {
        Some(err) => Err(err.to_compile_error()),
        None => Ok(()),
    }
}

fn inspect_subject_param(input: &FnArg) -> Result<Option<SubjectParamBinding>, TokenStream> {
    let FnArg::Typed(pat_type) = input else {
        return Err(
            syn::Error::new_spanned(input, "endpoint handlers cannot take a receiver")
                .to_compile_error(),
        );
    };

    let Some(inner) = subject_param_inner_type(&pat_type.ty) else {
        return Ok(None);
    };

    let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
        return Err(syn::Error::new_spanned(
            &pat_type.pat,
            "endpoint handler arguments must be simple identifiers",
        )
        .to_compile_error());
    };

    let _ = inner;

    Ok(Some(SubjectParamBinding {
        template_name: subject_param_template_name(&pat_ident.ident.to_string()).to_string(),
        span: pat_ident.ident.span(),
    }))
}

pub(crate) fn subject_param_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };

    let segment = type_path.path.segments.last()?;
    if segment.ident != "SubjectParam" {
        return None;
    }

    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    match arguments.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

pub(crate) fn subject_param_template_name(name: &str) -> &str {
    name.strip_prefix('_')
        .filter(|value| !value.is_empty())
        .unwrap_or(name)
}

fn combine_error(target: &mut Option<syn::Error>, error: syn::Error) {
    match target {
        Some(existing) => existing.combine(error),
        None => *target = Some(error),
    }
}

struct SubjectParamBinding {
    template_name: String,
    span: proc_macro2::Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PayloadEncoding {
    Json,
    Proto,
    Serde,
    Raw,
}

#[derive(Debug, Clone)]
pub(crate) struct PayloadMeta {
    pub encoding: PayloadEncoding,
    pub encrypted: bool,
    pub optional: bool,
    pub inner_type: Type,
}

#[derive(Debug, Clone)]
pub(crate) enum ResponseEncoding {
    Json,
    Proto,
    Serde,
    Raw,
    Unit,
}

#[derive(Debug, Clone)]
pub(crate) struct ResponseMeta {
    pub encoding: ResponseEncoding,
    pub encrypted: bool,
    pub optional: bool,
    pub inner_type: Option<Type>,
}

#[derive(Debug, Clone)]
pub(crate) struct SubjectParamMeta {
    pub name: String,
    pub inner_type: Type,
}

#[derive(Debug, Clone)]
pub(crate) struct EndpointClientMeta {
    pub subject_params: Vec<SubjectParamMeta>,
    pub payload: Option<PayloadMeta>,
    pub response: ResponseMeta,
}

pub(crate) fn last_segment_ident(ty: &Type) -> Option<String> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

pub(crate) fn extract_single_generic_arg(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn is_raw_type(ty: &Type) -> bool {
    matches!(last_segment_ident(ty).as_deref(), Some("Bytes" | "String")) || is_vec_u8(ty)
}

fn is_vec_u8(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let segment = type_path.path.segments.last().unwrap();
    if segment.ident != "Vec" {
        return false;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    if let Some(GenericArgument::Type(Type::Path(inner))) = arguments.args.first() {
        inner.path.segments.last().is_some_and(|s| s.ident == "u8")
    } else {
        false
    }
}

fn is_str_ref(ty: &Type) -> bool {
    if let Type::Reference(r) = ty
        && let Type::Path(p) = &*r.elem
    {
        return p.path.is_ident("str");
    }
    false
}

fn unwrap_optional_payload_markers(mut ty: &Type) -> Result<&Type, syn::Error> {
    loop {
        match last_segment_ident(ty).as_deref() {
            Some("Encrypted" | "Json" | "Proto") => {
                ty = extract_single_generic_arg(ty).ok_or_else(|| {
                    syn::Error::new_spanned(
                        ty,
                        "payload marker types require a single generic argument",
                    )
                })?;
            }
            _ => return Ok(ty),
        }
    }
}

fn unwrap_optional_response_markers(mut ty: &Type) -> Result<&Type, syn::Error> {
    loop {
        match last_segment_ident(ty).as_deref() {
            Some("Encrypted" | "Json" | "Proto") => {
                ty = extract_single_generic_arg(ty).ok_or_else(|| {
                    syn::Error::new_spanned(
                        ty,
                        "response marker types require a single generic argument",
                    )
                })?;
            }
            _ => return Ok(ty),
        }
    }
}

fn classify_payload(ty: &Type) -> Result<PayloadMeta, syn::Error> {
    let (optional, inner) = if last_segment_ident(ty).as_deref() == Some("Option") {
        let inner = extract_single_generic_arg(ty).ok_or_else(|| {
            syn::Error::new_spanned(ty, "`Payload<Option<T>>` requires a single inner type")
        })?;
        let nested = unwrap_optional_payload_markers(inner)?;
        if last_segment_ident(nested).as_deref() == Some("Option") {
            return Err(syn::Error::new_spanned(
                nested,
                "`Payload<Option<T>>` does not support nested `Option` payloads",
            ));
        }
        (true, inner)
    } else {
        (false, ty)
    };

    let mut meta = classify_payload_inner(inner);
    meta.optional = optional;
    Ok(meta)
}

fn classify_payload_inner(ty: &Type) -> PayloadMeta {
    let ident = last_segment_ident(ty);
    if ident.as_deref() == Some("Encrypted")
        && let Some(inner) = extract_single_generic_arg(ty)
    {
        let inner_ident = last_segment_ident(inner);
        match inner_ident.as_deref() {
            Some("Json") => {
                let json_inner = extract_single_generic_arg(inner)
                    .cloned()
                    .unwrap_or_else(|| inner.clone());
                return PayloadMeta {
                    encoding: PayloadEncoding::Json,
                    encrypted: true,
                    optional: false,
                    inner_type: json_inner,
                };
            }
            Some("Proto") => {
                let proto_inner = extract_single_generic_arg(inner)
                    .cloned()
                    .unwrap_or_else(|| inner.clone());
                return PayloadMeta {
                    encoding: PayloadEncoding::Proto,
                    encrypted: true,
                    optional: false,
                    inner_type: proto_inner,
                };
            }
            _ if is_raw_type(inner) => {
                return PayloadMeta {
                    encoding: PayloadEncoding::Raw,
                    encrypted: true,
                    optional: false,
                    inner_type: inner.clone(),
                };
            }
            _ => {
                return PayloadMeta {
                    encoding: PayloadEncoding::Serde,
                    encrypted: true,
                    optional: false,
                    inner_type: inner.clone(),
                };
            }
        }
    }

    match ident.as_deref() {
        Some("Json") => {
            let json_inner = extract_single_generic_arg(ty)
                .cloned()
                .unwrap_or_else(|| ty.clone());
            PayloadMeta {
                encoding: PayloadEncoding::Json,
                encrypted: false,
                optional: false,
                inner_type: json_inner,
            }
        }
        Some("Proto") => {
            let proto_inner = extract_single_generic_arg(ty)
                .cloned()
                .unwrap_or_else(|| ty.clone());
            PayloadMeta {
                encoding: PayloadEncoding::Proto,
                encrypted: false,
                optional: false,
                inner_type: proto_inner,
            }
        }
        _ if is_raw_type(ty) => PayloadMeta {
            encoding: PayloadEncoding::Raw,
            encrypted: false,
            optional: false,
            inner_type: ty.clone(),
        },
        _ => PayloadMeta {
            encoding: PayloadEncoding::Serde,
            encrypted: false,
            optional: false,
            inner_type: ty.clone(),
        },
    }
}

fn classify_response_type(ty: &Type) -> ResponseMeta {
    let ident = last_segment_ident(ty);

    if ident.as_deref() == Some("Encrypted")
        && let Some(inner) = extract_single_generic_arg(ty)
    {
        let inner_ident = last_segment_ident(inner);
        match inner_ident.as_deref() {
            Some("Json") => {
                let json_inner = extract_single_generic_arg(inner).cloned();
                return ResponseMeta {
                    encoding: ResponseEncoding::Json,
                    encrypted: true,
                    optional: false,
                    inner_type: json_inner,
                };
            }
            Some("Proto") => {
                let proto_inner = extract_single_generic_arg(inner).cloned();
                return ResponseMeta {
                    encoding: ResponseEncoding::Proto,
                    encrypted: true,
                    optional: false,
                    inner_type: proto_inner,
                };
            }
            _ if is_raw_type(inner) => {
                return ResponseMeta {
                    encoding: ResponseEncoding::Raw,
                    encrypted: true,
                    optional: false,
                    inner_type: Some(inner.clone()),
                };
            }
            _ => {
                return ResponseMeta {
                    encoding: ResponseEncoding::Serde,
                    encrypted: true,
                    optional: false,
                    inner_type: Some(inner.clone()),
                };
            }
        }
    }

    match ident.as_deref() {
        Some("Json") => ResponseMeta {
            encoding: ResponseEncoding::Json,
            encrypted: false,
            optional: false,
            inner_type: extract_single_generic_arg(ty).cloned(),
        },
        Some("Proto") => ResponseMeta {
            encoding: ResponseEncoding::Proto,
            encrypted: false,
            optional: false,
            inner_type: extract_single_generic_arg(ty).cloned(),
        },
        Some("String" | "Bytes") => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            optional: false,
            inner_type: Some(ty.clone()),
        },
        _ if is_vec_u8(ty) => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            optional: false,
            inner_type: Some(ty.clone()),
        },
        _ if is_str_ref(ty) => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            optional: false,
            inner_type: Some(ty.clone()),
        },
        _ => ResponseMeta {
            encoding: ResponseEncoding::Serde,
            encrypted: false,
            optional: false,
            inner_type: Some(ty.clone()),
        },
    }
}

fn classify_response(ty: &Type) -> Result<ResponseMeta, syn::Error> {
    let (optional, inner) = if last_segment_ident(ty).as_deref() == Some("Option") {
        let inner = extract_single_generic_arg(ty).ok_or_else(|| {
            syn::Error::new_spanned(ty, "`Option<T>` responses require a single inner type")
        })?;
        let nested = unwrap_optional_response_markers(inner)?;
        if last_segment_ident(nested).as_deref() == Some("Option") {
            return Err(syn::Error::new_spanned(
                nested,
                "optional responses do not support nested `Option` types",
            ));
        }
        (true, inner)
    } else {
        (false, ty)
    };

    if let Type::Tuple(tuple) = inner
        && tuple.elems.is_empty()
    {
        return Ok(ResponseMeta {
            encoding: ResponseEncoding::Unit,
            encrypted: false,
            optional,
            inner_type: None,
        });
    }

    let mut meta = classify_response_type(inner);
    meta.optional = optional;
    Ok(meta)
}

fn is_option_auth(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let segment = type_path.path.segments.last().unwrap();
    if segment.ident != "Option" {
        return false;
    }
    if let Some(inner) = extract_single_generic_arg(ty) {
        last_segment_ident(inner).as_deref() == Some("Auth")
    } else {
        false
    }
}

pub(crate) fn classify_return_type(sig: &Signature) -> Result<ResponseMeta, TokenStream> {
    let ReturnType::Type(_, return_type) = &sig.output else {
        return Ok(ResponseMeta {
            encoding: ResponseEncoding::Unit,
            encrypted: false,
            optional: false,
            inner_type: None,
        });
    };

    let ty = return_type.as_ref();

    if let Type::Tuple(tuple) = ty
        && tuple.elems.is_empty()
    {
        return Ok(ResponseMeta {
            encoding: ResponseEncoding::Unit,
            encrypted: false,
            optional: false,
            inner_type: None,
        });
    }

    let Type::Path(type_path) = ty else {
        return classify_response(ty).map_err(|error| error.to_compile_error());
    };

    let segment = type_path.path.segments.last().unwrap();
    if segment.ident == "Result"
        && let PathArguments::AngleBracketed(arguments) = &segment.arguments
        && let Some(GenericArgument::Type(ok_type)) = arguments.args.first()
    {
        if let Type::Tuple(tuple) = ok_type
            && tuple.elems.is_empty()
        {
            return Ok(ResponseMeta {
                encoding: ResponseEncoding::Unit,
                encrypted: false,
                optional: false,
                inner_type: None,
            });
        }
        return classify_response(ok_type).map_err(|error| error.to_compile_error());
    }

    classify_response(ty).map_err(|error| error.to_compile_error())
}

pub(crate) fn extract_client_meta(sig: &Signature) -> Result<EndpointClientMeta, TokenStream> {
    let mut subject_params = Vec::new();
    let mut payload = None;
    let mut payload_count = 0;

    for input in &sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            continue;
        };
        let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            continue;
        };
        let ty = &*pat_type.ty;
        let name = pat_ident.ident.to_string();

        if let Some(inner) = subject_param_inner_type(ty) {
            subject_params.push(SubjectParamMeta {
                name: subject_param_template_name(&name).to_string(),
                inner_type: inner.clone(),
            });
            continue;
        }

        if last_segment_ident(ty).as_deref() == Some("Payload") {
            payload_count += 1;
            if payload_count > 1 {
                return Err(syn::Error::new_spanned(
                    &pat_type.ty,
                    "handlers may have at most one `Payload<T>` parameter",
                )
                .to_compile_error());
            }
            if let Some(inner) = extract_single_generic_arg(ty) {
                payload = Some(classify_payload(inner).map_err(|error| error.to_compile_error())?);
            }
        }
    }

    let response = classify_return_type(sig)?;

    Ok(EndpointClientMeta {
        subject_params,
        payload,
        response,
    })
}

#[cfg(test)]
#[path = "tests/endpoint_tests.rs"]
mod tests;
