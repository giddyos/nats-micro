use std::collections::BTreeSet;

use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, GenericArgument, ItemFn, Pat, PathArguments, Type};

#[derive(Debug, FromMeta)]
pub struct EndpointArgs {
    subject: String,
    group: Option<String>,
    queue_group: Option<String>,
    #[darling(default)]
    auth: bool,
}

pub fn expand_endpoint(args: EndpointArgs, func: ItemFn) -> TokenStream {
    let subject = &args.subject;
    let group = args.group.as_deref().unwrap_or("default");

    let (nats_subject, template_params) = match parse_subject_template(subject, &func) {
        Ok(value) => value,
        Err(err) => return err,
    };

    if let Err(err) = validate_template_bindings(&func, &template_params) {
        return err;
    }

    let subject_template = if template_params.is_empty() {
        quote! { None }
    } else {
        quote! { Some(#subject.to_string()) }
    };

    let queue_group = match &args.queue_group {
        Some(qg) => quote! { Some(#qg.to_string()) },
        None => quote! { None },
    };

    let auth_required = args.auth;
    let fn_name = &func.sig.ident;
    let def_fn_name = format_ident!("{}_endpoint", fn_name);
    let handler = match build_handler(&func) {
        Ok(handler) => handler,
        Err(err) => return err,
    };

    quote! {
        #func

        #[doc(hidden)]
        pub fn #def_fn_name() -> ::nats_micro::EndpointDefinition {
            ::nats_micro::EndpointDefinition {
                service_name: ::std::string::String::new(),
                service_version: ::std::string::String::new(),
                service_description: ::std::string::String::new(),
                group: #group.to_string(),
                subject: #nats_subject.to_string(),
                subject_template: #subject_template,
                queue_group: #queue_group,
                auth_required: #auth_required,
                handler: #handler,
            }
        }

        ::nats_micro::__private::inventory::submit! {
            ::nats_micro::__private::EndpointRegistration {
                constructor: #def_fn_name,
            }
        }
    }
}

fn build_handler(func: &ItemFn) -> Result<TokenStream, TokenStream> {
    let fn_name = &func.sig.ident;
    let mut extractors = Vec::new();
    let mut args = Vec::new();

    for input in &func.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(
                syn::Error::new_spanned(input, "endpoint handlers cannot take a receiver")
                    .to_compile_error(),
            );
        };

        let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "endpoint handler arguments must be simple identifiers",
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
            let #ident: #ty = match <#ty as ::nats_micro::__private::FromRequest>::from_request(#ctx_expr) {
                Ok(value) => value,
                Err(err) => return Err(err),
            };
        });
        args.push(quote! { #ident });
    }

    Ok(quote! {
        ::nats_micro::HandlerFn::new(move |ctx: ::nats_micro::__private::RequestContext| {
            ::std::boxed::Box::pin(async move {
                let __request_id = ctx.request.request_id.clone();
                #(#extractors)*
                let response = #fn_name(#(#args),*)
                    .await
                    .map_err(|err| ::nats_micro::__private::IntoNatsError::into_nats_error(err, __request_id.clone()))?;
                ::nats_micro::__private::IntoNatsResponse::into_response(response, __request_id)
            })
        })
    })
}

fn is_subject_param(ty: &Type) -> bool {
    subject_param_inner_type(ty).is_some()
}

fn parse_subject_template(
    subject: &str,
    func: &ItemFn,
) -> Result<(String, Vec<String>), TokenStream> {
    let mut rendered_segments = Vec::new();
    let mut params = Vec::new();

    for segment in subject.trim().split('.') {
        if segment.is_empty() {
            return Err(syn::Error::new_spanned(
                &func.sig.ident,
                format!("endpoint subject `{subject}` contains an empty segment"),
            )
            .to_compile_error());
        }

        if segment.starts_with('{') || segment.ends_with('}') {
            if !segment.starts_with('{') || !segment.ends_with('}') {
                return Err(syn::Error::new_spanned(
                    &func.sig.ident,
                    format!(
                        "endpoint subject `{subject}` has an invalid template segment `{segment}`"
                    ),
                )
                .to_compile_error());
            }

            let name = &segment[1..segment.len() - 1];
            if name.is_empty() || name.contains('{') || name.contains('}') {
                return Err(syn::Error::new_spanned(
                    &func.sig.ident,
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
                &func.sig.ident,
                format!("endpoint subject `{subject}` has an invalid template segment `{segment}`"),
            )
            .to_compile_error());
        }

        rendered_segments.push(segment.to_string());
    }

    Ok((rendered_segments.join("."), params))
}

fn validate_template_bindings(
    func: &ItemFn,
    template_params: &[String],
) -> Result<(), TokenStream> {
    let mut errors: Option<syn::Error> = None;
    let mut declared_params = BTreeSet::new();
    let template_param_names: BTreeSet<_> = template_params.iter().cloned().collect();

    if template_param_names.len() != template_params.len() {
        return Err(syn::Error::new_spanned(
            &func.sig.ident,
            "endpoint subject templates must not reuse the same parameter name more than once",
        )
        .to_compile_error());
    }

    for input in &func.sig.inputs {
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
                    &func.sig.ident,
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

fn subject_param_inner_type(ty: &Type) -> Option<&Type> {
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

fn subject_param_template_name(name: &str) -> &str {
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
