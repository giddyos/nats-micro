use std::collections::BTreeSet;

use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, GenericArgument, ItemFn, Pat, PathArguments, Signature, Type};

#[derive(Debug, FromMeta)]
pub(crate) struct EndpointArgs {
    pub subject: String,
    pub group: Option<String>,
    pub queue_group: Option<String>,
    #[darling(default)]
    pub auth: bool,
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
            let #ident: #ty = match <#ty as ::nats_micro::__macros::FromRequest>::from_request(#ctx_expr) {
                Ok(value) => value,
                Err(err) => return Err(err),
            };
        });
        args.push(quote! { #ident });
    }

    Ok(quote! {
        ::nats_micro::HandlerFn::new(move |ctx: ::nats_micro::__macros::RequestContext| {
            ::std::boxed::Box::pin(async move {
                let __request_id = ctx.request.request_id.clone();
                #(#extractors)*
                let response = #fn_path(#(#args),*)
                    .await
                    .map_err(|err| ::nats_micro::__macros::IntoNatsError::into_nats_error(err, __request_id.clone()))?;
                ::nats_micro::__macros::IntoNatsResponse::into_response(response, __request_id)
            })
        })
    })
}

pub(crate) fn extract_param_info(sig: &Signature) -> Result<Vec<TokenStream>, TokenStream> {
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
            ::nats_micro::__macros::ParamInfo {
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
