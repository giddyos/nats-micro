use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ImplItem, ImplItemFn, ItemImpl, ItemStruct, Meta, Type};

use crate::consumer::ConsumerArgs;
use crate::endpoint::{
    EndpointArgs, build_handler_body, extract_param_info, parse_subject_template,
    validate_template_bindings,
};

#[derive(Debug, FromMeta)]
pub struct ServiceArgs {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
}

pub fn expand_service(args: ServiceArgs, item_struct: ItemStruct) -> TokenStream {
    let ident = &item_struct.ident;
    let service_name = args.name.unwrap_or_else(|| item_struct.ident.to_string());
    let version = args.version.unwrap_or_else(|| "0.1.0".to_string());
    let description = args.description.unwrap_or_default();

    quote! {
        #item_struct

        impl #ident {
            pub fn __nats_micro_service_meta() -> ::nats_micro::ServiceMetadata {
                ::nats_micro::ServiceMetadata::new(
                    #service_name,
                    #version,
                    #description,
                )
            }
        }
    }
}

pub fn expand_service_handlers(item_impl: ItemImpl) -> TokenStream {
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
                Ok(result) => {
                    let mut cleaned = method.clone();
                    cleaned.attrs.remove(idx);
                    cleaned_items.push(ImplItem::Fn(cleaned));
                    endpoint_def_fns.push(result.def_fn);
                    endpoint_accessor_fns.push(result.accessor_fn);
                    endpoint_def_calls.push(result.def_call);
                    endpoint_info_exprs.push(result.info_expr);
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
                    consumer_def_fns.push(result.def_fn);
                    consumer_accessor_fns.push(result.accessor_fn);
                    consumer_def_calls.push(result.def_call);
                    consumer_info_exprs.push(result.info_expr);
                }
                Err(err) => return err,
            }
        } else {
            cleaned_items.push(item.clone());
        }
    }

    let mut cleaned_impl = item_impl.clone();
    cleaned_impl.items = cleaned_items;

    quote! {
        #cleaned_impl

        impl #struct_ident {
            #(#endpoint_def_fns)*
            #(#consumer_def_fns)*
            #(#endpoint_accessor_fns)*
            #(#consumer_accessor_fns)*
        }

        impl ::nats_micro::__macros::NatsService for #struct_ident {
            fn definition() -> ::nats_micro::__macros::ServiceDefinition {
                ::nats_micro::__macros::ServiceDefinition {
                    metadata: #struct_ident::__nats_micro_service_meta(),
                    endpoints: vec![#(#endpoint_def_calls),*],
                    consumers: vec![#(#consumer_def_calls),*],
                    endpoint_info: vec![#(#endpoint_info_exprs),*],
                    consumer_info: vec![#(#consumer_info_exprs),*],
                }
            }
        }

        ::nats_micro::__macros::inventory::submit! {
            ::nats_micro::__macros::ServiceRegistration {
                constructor: <#struct_ident as ::nats_micro::__macros::NatsService>::definition,
            }
        }
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
    def_fn: TokenStream,
    accessor_fn: TokenStream,
    def_call: TokenStream,
    info_expr: TokenStream,
}

fn process_endpoint_method(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    attr: &syn::Attribute,
) -> Result<HandlerResult, TokenStream> {
    let args = parse_attr::<EndpointArgs>(attr)?;
    let subject = &args.subject;
    let group = args.group.as_deref().unwrap_or("");
    let auth_required = args.auth;
    let fn_name = &method.sig.ident;

    let (nats_subject, template_params) = parse_subject_template(subject, fn_name)?;
    validate_template_bindings(&method.sig, &template_params)?;

    let subject_template = if template_params.is_empty() {
        quote! { None }
    } else {
        quote! { Some(#subject.to_string()) }
    };

    let queue_group = match &args.queue_group {
        Some(qg) => quote! { Some(#qg.to_string()) },
        None => quote! { None },
    };

    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;

    let def_method_name = format_ident!("__ep_{}", fn_name);
    let accessor_name = format_ident!("{}_endpoint", fn_name);
    let fn_name_str = fn_name.to_string();

    Ok(HandlerResult {
        def_fn: quote! {
            #[doc(hidden)]
            pub fn #def_method_name() -> ::nats_micro::EndpointDefinition {
                let __meta = Self::__nats_micro_service_meta();
                ::nats_micro::EndpointDefinition {
                    service_name: __meta.name,
                    service_version: __meta.version,
                    service_description: __meta.description,
                    group: #group.to_string(),
                    subject: #nats_subject.to_string(),
                    subject_template: #subject_template,
                    queue_group: #queue_group,
                    auth_required: #auth_required,
                    handler: #handler,
                }
            }
        },
        accessor_fn: quote! {
            pub fn #accessor_name() -> ::nats_micro::EndpointDefinition {
                Self::#def_method_name()
            }
        },
        def_call: quote! { Self::#def_method_name() },
        info_expr: quote! {
            ::nats_micro::__macros::EndpointInfo {
                fn_name: #fn_name_str.to_string(),
                subject_template: #subject.to_string(),
                subject_pattern: #nats_subject.to_string(),
                group: #group.to_string(),
                queue_group: #queue_group,
                auth_required: #auth_required,
                params: vec![#(#param_infos),*],
            }
        },
    })
}

fn process_consumer_method(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    attr: &syn::Attribute,
) -> Result<HandlerResult, TokenStream> {
    let args = parse_attr::<ConsumerArgs>(attr)?;
    let fn_name = &method.sig.ident;
    let stream = args.stream.as_deref().unwrap_or("DEFAULT");
    let durable = args.durable.unwrap_or_else(|| fn_name.to_string());
    let auth_required = args.auth;

    let config_tokens = match args.config {
        Some(config) => {
            let expr = config.0;
            quote! {{
                let __config: ::nats_micro::__macros::ConsumerConfig = (#expr);
                __config
            }}
        }
        None => quote! { ::nats_micro::__macros::ConsumerConfig::default() },
    };

    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;

    let def_method_name = format_ident!("__con_{}", fn_name);
    let accessor_name = format_ident!("{}_consumer", fn_name);
    let fn_name_str = fn_name.to_string();

    Ok(HandlerResult {
        def_fn: quote! {
            #[doc(hidden)]
            pub fn #def_method_name() -> ::nats_micro::ConsumerDefinition {
                ::nats_micro::ConsumerDefinition {
                    stream: #stream.to_string(),
                    durable: #durable.to_string(),
                    auth_required: #auth_required,
                    config: #config_tokens,
                    handler: #handler,
                }
            }
        },
        accessor_fn: quote! {
            pub fn #accessor_name() -> ::nats_micro::ConsumerDefinition {
                Self::#def_method_name()
            }
        },
        def_call: quote! { Self::#def_method_name() },
        info_expr: quote! {
            ::nats_micro::__macros::ConsumerInfo {
                fn_name: #fn_name_str.to_string(),
                stream: #stream.to_string(),
                durable: #durable.to_string(),
                auth_required: #auth_required,
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
