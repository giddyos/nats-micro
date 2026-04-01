use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ImplItem, ImplItemFn, ItemImpl, ItemStruct, Meta, Type};

use crate::client::{ClientEndpointSpec, build_endpoint_client_spec, generate_client_module};
use crate::consumer::ConsumerArgs;
use crate::endpoint::{
    EndpointArgs, PayloadEncoding, ResponseEncoding, build_handler_body, extract_param_info,
    requires_auth,
};
use crate::utils::nats_micro_path;

#[derive(Debug, FromMeta)]
pub struct ServiceArgs {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    prefix: Option<String>,
}

pub fn expand_service(args: ServiceArgs, item_struct: ItemStruct) -> TokenStream {
    let nats_micro = nats_micro_path();
    let ident = &item_struct.ident;
    let service_name = args.name.unwrap_or_else(|| item_struct.ident.to_string());
    let version = args.version.unwrap_or_else(|| "0.1.0".to_string());
    let description = args.description.unwrap_or_default();
    let prefix = match args.prefix {
        Some(prefix) => quote! { Some(#prefix.to_string()) },
        None => quote! { None },
    };

    quote! {
        #item_struct

        impl #ident {
            pub fn __nats_micro_service_meta() -> #nats_micro::ServiceMetadata {
                #nats_micro::ServiceMetadata::new(
                    #service_name,
                    #version,
                    #description,
                    #prefix,
                )
            }
        }
    }
}

pub fn expand_service_handlers(item_impl: ItemImpl) -> TokenStream {
    let nats_micro = nats_micro_path();
    let struct_ty = &item_impl.self_ty;
    let Some(struct_ident) = extract_ident_from_type(struct_ty) else {
        return syn::Error::new_spanned(struct_ty, "expected a named struct type")
            .to_compile_error();
    };

    let mut cleaned_items: Vec<ImplItem> = Vec::new();
    let mut endpoint_def_fns = Vec::new();
    let mut endpoint_accessor_fns = Vec::new();
    let mut endpoint_def_calls = Vec::new();
    let mut endpoint_info_exprs = Vec::new();
    let mut consumer_def_fns = Vec::new();
    let mut consumer_accessor_fns = Vec::new();
    let mut consumer_def_calls = Vec::new();
    let mut consumer_info_exprs = Vec::new();
    let mut client_endpoints: Vec<ClientEndpointSpec> = Vec::new();

    for item in &item_impl.items {
        let ImplItem::Fn(method) = item else {
            cleaned_items.push(item.clone());
            continue;
        };

        let ep_idx = method
            .attrs
            .iter()
            .position(|a| a.path().is_ident("endpoint"));
        let con_idx = method
            .attrs
            .iter()
            .position(|a| a.path().is_ident("consumer"));

        if let Some(idx) = ep_idx {
            let attr = &method.attrs[idx];
            match process_endpoint_method(&struct_ident, method, attr) {
                Ok((result, client_data)) => {
                    let mut cleaned = method.clone();
                    cleaned.attrs.remove(idx);
                    cleaned_items.push(ImplItem::Fn(cleaned));
                    let attrs = &result.attrs;
                    let def_fn = &result.def_fn;
                    let accessor_fn = &result.accessor_fn;
                    let def_call = &result.def_call;
                    let info_expr = &result.info_expr;
                    endpoint_def_fns.push(quote! {
                        #(#attrs)*
                        #def_fn
                    });
                    endpoint_accessor_fns.push(quote! {
                        #(#attrs)*
                        #accessor_fn
                    });
                    endpoint_def_calls.push(quote! {
                        #(#attrs)*
                        #def_call
                    });
                    endpoint_info_exprs.push(quote! {
                        #(#attrs)*
                        #info_expr
                    });
                    client_endpoints.push(client_data);
                }
                Err(err) => return err,
            }
        } else if let Some(idx) = con_idx {
            let attr = &method.attrs[idx];
            match process_consumer_method(&struct_ident, method, attr) {
                Ok(result) => {
                    let mut cleaned = method.clone();
                    cleaned.attrs.remove(idx);
                    cleaned_items.push(ImplItem::Fn(cleaned));
                    let attrs = &result.attrs;
                    let def_fn = &result.def_fn;
                    let accessor_fn = &result.accessor_fn;
                    let def_call = &result.def_call;
                    let info_expr = &result.info_expr;
                    consumer_def_fns.push(quote! {
                        #(#attrs)*
                        #def_fn
                    });
                    consumer_accessor_fns.push(quote! {
                        #(#attrs)*
                        #accessor_fn
                    });
                    consumer_def_calls.push(quote! {
                        #(#attrs)*
                        #def_call
                    });
                    consumer_info_exprs.push(quote! {
                        #(#attrs)*
                        #info_expr
                    });
                }
                Err(err) => return err,
            }
        } else {
            cleaned_items.push(item.clone());
        }
    }

    let mut cleaned_impl = item_impl.clone();
    cleaned_impl.items = cleaned_items;

    let service_name_str = struct_ident.to_string();
    let client_module = if cfg!(feature = "macros_client_feature") {
        generate_client_module(&struct_ident, &service_name_str, &client_endpoints)
    } else {
        quote! {}
    };

    quote! {
        #cleaned_impl

        impl #struct_ident {
            #(#endpoint_def_fns)*
            #(#consumer_def_fns)*
            #(#endpoint_accessor_fns)*
            #(#consumer_accessor_fns)*
        }

        impl #nats_micro::__macros::NatsService for #struct_ident {
            fn definition() -> #nats_micro::__macros::ServiceDefinition {
                #nats_micro::__macros::ServiceDefinition {
                    metadata: #struct_ident::__nats_micro_service_meta(),
                    endpoints: vec![#(#endpoint_def_calls),*],
                    consumers: vec![#(#consumer_def_calls),*],
                    endpoint_info: vec![#(#endpoint_info_exprs),*],
                    consumer_info: vec![#(#consumer_info_exprs),*],
                }
            }
        }

        #nats_micro::__macros::inventory::submit! {
            #nats_micro::__macros::ServiceRegistration {
                constructor: <#struct_ident as #nats_micro::__macros::NatsService>::definition,
            }
        }

        #client_module
    }
}

fn extract_ident_from_type(ty: &Type) -> Option<syn::Ident> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|s| s.ident.clone())
    } else {
        None
    }
}

struct HandlerResult {
    attrs: Vec<syn::Attribute>,
    def_fn: TokenStream,
    accessor_fn: TokenStream,
    def_call: TokenStream,
    info_expr: TokenStream,
}

fn conditional_attrs(method: &ImplItemFn) -> Vec<syn::Attribute> {
    method
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .cloned()
        .collect()
}

fn process_endpoint_method(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    attr: &syn::Attribute,
) -> Result<(HandlerResult, ClientEndpointSpec), TokenStream> {
    let nats_micro = nats_micro_path();
    let args = parse_attr::<EndpointArgs>(attr)?;
    let auth_required = requires_auth(&method.sig);
    let concurrency_limit = args.concurrency_limit;
    let fn_name = &method.sig.ident;
    let attrs = conditional_attrs(method);
    let client_endpoint = build_endpoint_client_spec(method, &args, attrs.clone())?;
    let subject = &client_endpoint.subject.template;
    let group = &client_endpoint.group;
    let nats_subject = &client_endpoint.subject.pattern;
    let subject_template = if client_endpoint.subject.is_templated() {
        quote! { Some(#subject.to_string()) }
    } else {
        quote! { None }
    };

    let queue_group = match &args.queue_group {
        Some(qg) => quote! { Some(#qg.to_string()) },
        None => quote! { None },
    };
    let concurrency_limit_tokens = match concurrency_limit {
        Some(limit) => quote! { Some(#limit) },
        None => quote! { None },
    };

    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;

    let payload_meta_tokens = match &client_endpoint.payload_meta {
        Some(pm) => {
            let enc = payload_encoding_tokens(&pm.encoding);
            let encrypted = pm.encrypted;
            let inner_type = &pm.inner_type;
            let inner_type_str = quote!(#inner_type).to_string();
            quote! {
                Some(#nats_micro::__macros::PayloadMeta {
                    encoding: #enc,
                    encrypted: #encrypted,
                    inner_type: #inner_type_str.to_string(),
                })
            }
        }
        None => quote! { None },
    };

    let response_meta = &client_endpoint.response_meta;
    let resp_enc = response_encoding_tokens(&response_meta.encoding);
    let resp_encrypted = response_meta.encrypted;
    let resp_inner_str = response_meta
        .inner_type
        .as_ref()
        .map(|t| quote!(#t).to_string())
        .unwrap_or_default();
    let response_meta_tokens = quote! {
        #nats_micro::__macros::ResponseMeta {
            encoding: #resp_enc,
            encrypted: #resp_encrypted,
            inner_type: #resp_inner_str.to_string(),
        }
    };

    let def_method_name = format_ident!("__ep_{}", fn_name);
    let accessor_name = format_ident!("{}_endpoint", fn_name);
    let fn_name_str = fn_name.to_string();

    let handler_result = HandlerResult {
        attrs: attrs.clone(),
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
                    concurrency_limit: #concurrency_limit_tokens,
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
                concurrency_limit: #concurrency_limit_tokens,
                params: vec![#(#param_infos),*],
                payload_meta: #payload_meta_tokens,
                response_meta: #response_meta_tokens,
            }
        },
    };

    Ok((handler_result, client_endpoint))
}

fn process_consumer_method(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    attr: &syn::Attribute,
) -> Result<HandlerResult, TokenStream> {
    let nats_micro = nats_micro_path();
    let args = parse_attr::<ConsumerArgs>(attr)?;
    let fn_name = &method.sig.ident;
    let stream = args.stream.as_deref().unwrap_or("DEFAULT");
    let durable = args.durable.unwrap_or_else(|| fn_name.to_string());
    let auth_required = requires_auth(&method.sig);
    let concurrency_limit = args.concurrency_limit;
    let concurrency_limit_tokens = match concurrency_limit {
        Some(limit) => quote! { Some(#limit) },
        None => quote! { None },
    };

    let config_tokens = match args.config {
        Some(config) => {
            let expr = config.0;
            quote! {{
                let __config: #nats_micro::__macros::ConsumerConfig = (#expr);
                __config
            }}
        }
        None => quote! { #nats_micro::__macros::ConsumerConfig::default() },
    };

    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;

    let def_method_name = format_ident!("__con_{}", fn_name);
    let accessor_name = format_ident!("{}_consumer", fn_name);
    let fn_name_str = fn_name.to_string();
    let attrs = conditional_attrs(method);

    Ok(HandlerResult {
        attrs,
        def_fn: quote! {
            #[doc(hidden)]
            pub fn #def_method_name() -> #nats_micro::ConsumerDefinition {
                #nats_micro::ConsumerDefinition {
                    stream: #stream.to_string(),
                    durable: #durable.to_string(),
                    auth_required: #auth_required,
                    concurrency_limit: #concurrency_limit_tokens,
                    config: #config_tokens,
                    handler: #handler,
                }
            }
        },
        accessor_fn: quote! {
            pub fn #accessor_name() -> #nats_micro::ConsumerDefinition {
                Self::#def_method_name()
            }
        },
        def_call: quote! { Self::#def_method_name() },
        info_expr: quote! {
            #nats_micro::__macros::ConsumerInfo {
                fn_name: #fn_name_str.to_string(),
                stream: #stream.to_string(),
                durable: #durable.to_string(),
                auth_required: #auth_required,
                concurrency_limit: #concurrency_limit_tokens,
                params: vec![#(#param_infos),*],
            }
        },
    })
}

fn parse_attr<T: FromMeta>(attr: &syn::Attribute) -> Result<T, TokenStream> {
    let Meta::List(meta_list) = &attr.meta else {
        return Err(
            syn::Error::new_spanned(attr, "expected attribute arguments in parentheses")
                .to_compile_error(),
        );
    };
    let nested = NestedMeta::parse_meta_list(meta_list.tokens.clone())
        .map_err(|e| darling::Error::from(e).write_errors())?;
    T::from_list(&nested).map_err(|e| e.write_errors())
}

fn payload_encoding_tokens(enc: &PayloadEncoding) -> TokenStream {
    let nats_micro = nats_micro_path();
    match enc {
        PayloadEncoding::Json => quote! { #nats_micro::__macros::PayloadEncoding::Json },
        PayloadEncoding::Proto => quote! { #nats_micro::__macros::PayloadEncoding::Proto },
        PayloadEncoding::Serde => quote! { #nats_micro::__macros::PayloadEncoding::Serde },
        PayloadEncoding::Raw => quote! { #nats_micro::__macros::PayloadEncoding::Raw },
    }
}

fn response_encoding_tokens(enc: &ResponseEncoding) -> TokenStream {
    let nats_micro = nats_micro_path();
    match enc {
        ResponseEncoding::Json => quote! { #nats_micro::__macros::ResponseEncoding::Json },
        ResponseEncoding::Proto => quote! { #nats_micro::__macros::ResponseEncoding::Proto },
        ResponseEncoding::Serde => quote! { #nats_micro::__macros::ResponseEncoding::Serde },
        ResponseEncoding::Raw => quote! { #nats_micro::__macros::ResponseEncoding::Raw },
        ResponseEncoding::Unit => quote! { #nats_micro::__macros::ResponseEncoding::Unit },
    }
}

#[cfg(test)]
#[path = "tests/service_tests.rs"]
mod tests;
