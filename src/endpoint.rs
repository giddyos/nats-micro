use std::collections::BTreeSet;

use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, GenericArgument, ItemFn, Pat, PathArguments, ReturnType, Signature, Type};

use crate::utils::nats_micro_path;

#[derive(Debug, FromMeta)]
pub(crate) struct EndpointArgs {
    pub subject: String,
    pub group: Option<String>,
    pub queue_group: Option<String>,
    pub concurrency_limit: Option<u64>,
}

pub fn expand_endpoint(args: EndpointArgs, func: ItemFn) -> TokenStream {
    let _ = args;
    crate::utils::error_stream(
        func.sig.ident.span(),
        "#[endpoint] may only be used on methods inside a #[service_handlers] impl block",
        func,
    )
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
                        "endpoint subject template requires handler argument `{0}: SubjectParam<...>` or `_{0}: SubjectParam<...>`",
                        missing
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

#[derive(Debug, Clone)]
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
                    inner_type: proto_inner,
                };
            }
            _ if is_raw_type(inner) => {
                return PayloadMeta {
                    encoding: PayloadEncoding::Raw,
                    encrypted: true,
                    inner_type: inner.clone(),
                };
            }
            _ => {
                return PayloadMeta {
                    encoding: PayloadEncoding::Serde,
                    encrypted: true,
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
                inner_type: proto_inner,
            }
        }
        _ if is_raw_type(ty) => PayloadMeta {
            encoding: PayloadEncoding::Raw,
            encrypted: false,
            inner_type: ty.clone(),
        },
        _ => PayloadMeta {
            encoding: PayloadEncoding::Serde,
            encrypted: false,
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
                    inner_type: json_inner,
                };
            }
            Some("Proto") => {
                let proto_inner = extract_single_generic_arg(inner).cloned();
                return ResponseMeta {
                    encoding: ResponseEncoding::Proto,
                    encrypted: true,
                    inner_type: proto_inner,
                };
            }
            _ if is_raw_type(inner) => {
                return ResponseMeta {
                    encoding: ResponseEncoding::Raw,
                    encrypted: true,
                    inner_type: Some(inner.clone()),
                };
            }
            _ => {
                return ResponseMeta {
                    encoding: ResponseEncoding::Serde,
                    encrypted: true,
                    inner_type: Some(inner.clone()),
                };
            }
        }
    }

    match ident.as_deref() {
        Some("Json") => ResponseMeta {
            encoding: ResponseEncoding::Json,
            encrypted: false,
            inner_type: extract_single_generic_arg(ty).cloned(),
        },
        Some("Proto") => ResponseMeta {
            encoding: ResponseEncoding::Proto,
            encrypted: false,
            inner_type: extract_single_generic_arg(ty).cloned(),
        },
        Some("String") => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            inner_type: Some(ty.clone()),
        },
        Some("Bytes") => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            inner_type: Some(ty.clone()),
        },
        _ if is_vec_u8(ty) => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            inner_type: Some(ty.clone()),
        },
        _ if is_str_ref(ty) => ResponseMeta {
            encoding: ResponseEncoding::Raw,
            encrypted: false,
            inner_type: Some(ty.clone()),
        },
        _ => ResponseMeta {
            encoding: ResponseEncoding::Serde,
            encrypted: false,
            inner_type: Some(ty.clone()),
        },
    }
}

fn is_server_only_type(ty: &Type) -> bool {
    matches!(
        last_segment_ident(ty).as_deref(),
        Some("State" | "Auth" | "RequestId" | "Subject" | "NatsRequest")
    ) || is_option_auth(ty)
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

pub(crate) fn classify_return_type(sig: &Signature) -> ResponseMeta {
    let ReturnType::Type(_, return_type) = &sig.output else {
        return ResponseMeta {
            encoding: ResponseEncoding::Unit,
            encrypted: false,
            inner_type: None,
        };
    };

    let ty = return_type.as_ref();

    if let Type::Reference(ref_type) = ty {
        if let Type::Path(inner_path) = &*ref_type.elem
            && inner_path.path.is_ident("str")
        {
            return ResponseMeta {
                encoding: ResponseEncoding::Raw,
                encrypted: false,
                inner_type: Some(ty.clone()),
            };
        }
        return ResponseMeta {
            encoding: ResponseEncoding::Serde,
            encrypted: false,
            inner_type: Some(ty.clone()),
        };
    }

    let Type::Path(type_path) = ty else {
        return ResponseMeta {
            encoding: ResponseEncoding::Unit,
            encrypted: false,
            inner_type: None,
        };
    };

    let segment = type_path.path.segments.last().unwrap();
    if segment.ident == "Result"
        && let PathArguments::AngleBracketed(arguments) = &segment.arguments
        && let Some(GenericArgument::Type(ok_type)) = arguments.args.first()
    {
        if let Type::Tuple(tuple) = ok_type
            && tuple.elems.is_empty()
        {
            return ResponseMeta {
                encoding: ResponseEncoding::Unit,
                encrypted: false,
                inner_type: None,
            };
        }
        return classify_response_type(ok_type);
    }

    classify_response_type(ty)
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
                payload = Some(classify_payload_inner(inner));
            }
            continue;
        }

        if is_server_only_type(ty) {
            continue;
        }
    }

    let response = classify_return_type(sig);

    Ok(EndpointClientMeta {
        subject_params,
        payload,
        response,
    })
}
