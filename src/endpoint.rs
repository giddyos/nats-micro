use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, Type};

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

    let has_template = subject.contains('{');
    let nats_subject = if has_template {
        let mut result = String::new();
        let mut in_brace = false;
        for ch in subject.chars() {
            match ch {
                '{' => {
                    in_brace = true;
                    result.push('*');
                }
                '}' => in_brace = false,
                _ if in_brace => {}
                _ => result.push(ch),
            }
        }

        result.trim().to_string()
    } else {
        subject.clone().trim().to_string()
    };

    let subject_template = if has_template {
        quote! { Some(#subject.to_string()) }
    } else {
        quote! { None }
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
    }
}

fn build_handler(func: &ItemFn) -> Result<TokenStream, TokenStream> {
    let fn_name = &func.sig.ident;
    let mut extractors = Vec::new();
    let mut args = Vec::new();

    for input in &func.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(syn::Error::new_spanned(input, "endpoint handlers cannot take a receiver")
                .to_compile_error());
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
            quote! { &ctx.with_param_name(::std::string::ToString::to_string(stringify!(#ident))) }
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
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == "SubjectParam")
        .unwrap_or(false)
}
