use darling::FromMeta;
use darling::ast::NestedMeta;
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{GenericArgument, ImplItem, ImplItemFn, ItemImpl, ItemStruct, Meta, PathArguments, Type};

use crate::consumer::ConsumerArgs;
use crate::endpoint::{
    EndpointArgs, PayloadEncoding, PayloadMeta, ResponseEncoding, ResponseMeta, build_handler_body,
    classify_return_type, extract_client_meta, extract_param_info, parse_subject_template,
    requires_auth, validate_template_bindings,
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
    let mut client_endpoints: Vec<ClientEndpointData> = Vec::new();

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

struct ClientEndpointData {
    attrs: Vec<syn::Attribute>,
    fn_name: syn::Ident,
    error_type: Type,
    group: String,
    subject_template: String,
    nats_subject: String,
    payload: Option<PayloadMeta>,
    response: ResponseMeta,
    subject_params: Vec<crate::endpoint::SubjectParamMeta>,
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
) -> Result<(HandlerResult, ClientEndpointData), TokenStream> {
    let nats_micro = nats_micro_path();
    let args = parse_attr::<EndpointArgs>(attr)?;
    let subject = &args.subject;
    let group = args.group.as_deref().unwrap_or("");
    let auth_required = requires_auth(&method.sig);
    let concurrency_limit = args.concurrency_limit;
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
    let concurrency_limit_tokens = match concurrency_limit {
        Some(limit) => quote! { Some(#limit) },
        None => quote! { None },
    };

    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;
    let client_meta = extract_client_meta(&method.sig)?;
    let error_type = extract_result_error_type(&method.sig)?;

    let payload_meta_tokens = match &client_meta.payload {
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

    let response_meta = &client_meta.response;
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
    let attrs = conditional_attrs(method);

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

    let client_data = ClientEndpointData {
        attrs,
        fn_name: fn_name.clone(),
        error_type,
        group: group.to_string(),
        subject_template: subject.clone(),
        nats_subject: nats_subject.clone(),
        payload: client_meta.payload,
        response: client_meta.response,
        subject_params: client_meta.subject_params,
    };

    Ok((handler_result, client_data))
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

fn extract_result_error_type(sig: &syn::Signature) -> Result<Type, TokenStream> {
    let syn::ReturnType::Type(_, ty) = &sig.output else {
        return Err(
            syn::Error::new_spanned(&sig.output, "endpoint must return Result<T, E>")
                .to_compile_error(),
        );
    };

    let Type::Path(type_path) = ty.as_ref() else {
        return Err(
            syn::Error::new_spanned(ty, "endpoint must return Result<T, E>").to_compile_error(),
        );
    };

    let Some(segment) = type_path.path.segments.last() else {
        return Err(
            syn::Error::new_spanned(ty, "endpoint must return Result<T, E>").to_compile_error(),
        );
    };

    if segment.ident != "Result" {
        return Err(
            syn::Error::new_spanned(ty, "endpoint must return Result<T, E>").to_compile_error(),
        );
    }

    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(
            syn::Error::new_spanned(ty, "endpoint must return Result<T, E>").to_compile_error(),
        );
    };

    let mut generic_args = arguments.args.iter();
    let _ok_type = generic_args.next();
    let Some(GenericArgument::Type(error_type)) = generic_args.next() else {
        return Err(
            syn::Error::new_spanned(ty, "endpoint must return Result<T, E>").to_compile_error(),
        );
    };

    Ok(error_type.clone())
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

fn generate_client_module(
    struct_ident: &syn::Ident,
    service_name_str: &str,
    endpoints: &[ClientEndpointData],
) -> TokenStream {
    let nats_micro = nats_micro_path();
    let module_name = format_ident!("{}_client", service_name_str.to_snake_case());
    let client_struct_name = format_ident!("{}Client", service_name_str);
    let encryption_enabled = cfg!(feature = "macros_encryption_feature");
    let mut methods = Vec::new();

    for ep in endpoints {
        let attrs = &ep.attrs;
        let fn_ident = &ep.fn_name;
        let fn_with_ident = format_ident!("{}_with", fn_ident);
        let error_type = &ep.error_type;
        let group = &ep.group;

        let subject_param_args: Vec<TokenStream> = ep
            .subject_params
            .iter()
            .map(|sp| {
                let name = format_ident!("{}", sp.name);
                let ty = &sp.inner_type;
                quote! { #name: &#ty }
            })
            .collect();

        let subject_param_str_conversions: Vec<TokenStream> = ep
            .subject_params
            .iter()
            .map(|sp| {
                let name = format_ident!("{}", sp.name);
                quote! { #name }
            })
            .collect();

        let endpoint_subject_expr = if ep.subject_params.is_empty() {
            let nats_subject = &ep.nats_subject;
            quote! { #nats_subject.to_string() }
        } else {
            let template = &ep.subject_template;
            let mut fmt_str = String::new();
            let mut fmt_args = Vec::new();
            for segment in template.split('.') {
                if !fmt_str.is_empty() {
                    fmt_str.push('.');
                }
                if segment.starts_with('{') && segment.ends_with('}') {
                    fmt_str.push_str("{}");
                    let param_name = &segment[1..segment.len() - 1];
                    let param_ident = format_ident!("{}", param_name);
                    fmt_args.push(quote! { #param_ident });
                } else {
                    fmt_str.push_str(segment);
                }
            }
            quote! { format!(#fmt_str, #(#fmt_args),*) }
        };

        let needs_encryption =
            ep.payload.as_ref().is_some_and(|p| p.encrypted) || ep.response.encrypted;

        let has_payload = ep.payload.is_some();
        let payload_type = ep.payload.as_ref().map(|p| &p.inner_type);
        let payload_encoding = ep.payload.as_ref().map(|p| &p.encoding);
        let payload_encrypted = ep.payload.as_ref().is_some_and(|p| p.encrypted);

        let response_type = ep.response.inner_type.as_ref();
        let response_encoding = &ep.response.encoding;
        let response_encrypted = ep.response.encrypted;

        let payload_fn_args: Vec<TokenStream> = if has_payload {
            let pt = payload_type.unwrap();
            match payload_encoding.unwrap() {
                PayloadEncoding::Raw => {
                    let raw_ident = crate::endpoint::last_segment_ident(pt);
                    match raw_ident.as_deref() {
                        Some("String") => vec![quote! { payload: &str }],
                        _ => vec![quote! { payload: &[u8] }],
                    }
                }
                _ => vec![quote! { payload: &#pt }],
            }
        } else {
            vec![]
        };

        let serialize_payload = if has_payload {
            let pt = payload_type.unwrap();
            match payload_encoding.unwrap() {
                PayloadEncoding::Json | PayloadEncoding::Serde => {
                    quote! {
                        let __body = #nats_micro::__macros::serialize_serde_payload(payload)
                            .map_err(#nats_micro::ClientError::<#error_type>::serialize)?;
                    }
                }
                PayloadEncoding::Proto => {
                    quote! {
                        let __body = #nats_micro::__macros::serialize_proto_payload(payload)
                            .map_err(#nats_micro::ClientError::<#error_type>::serialize)?;
                    }
                }
                PayloadEncoding::Raw => {
                    quote! { let __body = #nats_micro::Bytes::copy_from_slice(payload.as_ref()); }
                }
            }
        } else {
            quote! { let __body = #nats_micro::Bytes::new(); }
        };

        let return_type: TokenStream = match response_encoding {
            ResponseEncoding::Unit => quote! { () },
            ResponseEncoding::Raw => {
                if let Some(rt) = response_type {
                    let ident = crate::endpoint::last_segment_ident(rt);
                    let is_str_ref = matches!(rt, Type::Reference(r) if matches!(&*r.elem, Type::Path(p) if p.path.is_ident("str")));
                    match ident.as_deref() {
                        Some("String") => quote! { String },
                        _ if is_str_ref => quote! { String },
                        _ => quote! { Vec<u8> },
                    }
                } else {
                    quote! { Vec<u8> }
                }
            }
            _ => {
                if let Some(rt) = response_type {
                    quote! { #rt }
                } else {
                    quote! { () }
                }
            }
        };

        let deserialize_response = match response_encoding {
            ResponseEncoding::Unit => {
                quote! { #nats_micro::__macros::deserialize_unit_response::<#error_type>(__response_headers, &__response_payload) }
            }
            ResponseEncoding::Json | ResponseEncoding::Serde => {
                quote! { #nats_micro::__macros::deserialize_response::<#return_type, #error_type>(__response_headers, &__response_payload) }
            }
            ResponseEncoding::Proto => {
                quote! { #nats_micro::__macros::deserialize_proto_response::<#return_type, #error_type>(__response_headers, &__response_payload) }
            }
            ResponseEncoding::Raw => {
                if let Some(rt) = response_type {
                    let ident = crate::endpoint::last_segment_ident(rt);
                    let is_str_ref = matches!(rt, Type::Reference(r) if matches!(&*r.elem, Type::Path(p) if p.path.is_ident("str")));
                    match ident.as_deref() {
                        Some("String") => {
                            quote! { #nats_micro::__macros::raw_response_to_string::<#error_type>(__response_headers, &__response_payload) }
                        }
                        _ if is_str_ref => {
                            quote! { #nats_micro::__macros::raw_response_to_string::<#error_type>(__response_headers, &__response_payload) }
                        }
                        _ => {
                            quote! { #nats_micro::__macros::raw_response_to_bytes::<#error_type>(__response_headers, &__response_payload) }
                        }
                    }
                } else {
                    quote! { #nats_micro::__macros::raw_response_to_bytes::<#error_type>(__response_headers, &__response_payload) }
                }
            }
        };

        let all_with_args: Vec<TokenStream> = subject_param_args
            .iter()
            .chain(payload_fn_args.iter())
            .cloned()
            .collect();

        let forward_args: Vec<TokenStream> = ep
            .subject_params
            .iter()
            .map(|sp| {
                let name = format_ident!("{}", sp.name);
                quote! { #name }
            })
            .chain(if has_payload {
                vec![quote! { payload }]
            } else {
                vec![]
            })
            .collect();

        let with_body = if payload_encrypted || response_encrypted {
            if payload_encrypted && response_encrypted {
                quote! {
                    #serialize_payload
                    let __subject = self.build_subject(#group, &#endpoint_subject_expr);
                    let options = self.apply_encryption_recipient(options);
                    let (__msg, __eph_ctx) = options
                        .into_encrypted_request(&self.client, __subject, __body.to_vec())
                        .await
                        .map_err(#nats_micro::ClientError::<#error_type>::request)?;
                    let __response_headers = __msg.headers.as_ref();
                    let __response_payload = #nats_micro::__macros::decrypt_client_response::<#error_type>(
                        __response_headers,
                        &__eph_ctx,
                        &__msg.payload,
                    )?;
                    #deserialize_response
                }
            } else if payload_encrypted {
                quote! {
                    #serialize_payload
                    let __subject = self.build_subject(#group, &#endpoint_subject_expr);
                    let options = self.apply_encryption_recipient(options);
                    let (__msg, _) = options
                        .into_encrypted_request(&self.client, __subject, __body.to_vec())
                        .await
                        .map_err(#nats_micro::ClientError::<#error_type>::request)?;
                    let __response_headers = __msg.headers.as_ref();
                    let __response_payload = __msg.payload.to_vec();
                    #deserialize_response
                }
            } else {
                quote! {
                    #serialize_payload
                    let __subject = self.build_subject(#group, &#endpoint_subject_expr);
                    let __msg = options
                        .into_request(&self.client, __subject, __body)
                        .await
                        .map_err(#nats_micro::ClientError::<#error_type>::request)?;
                    let __response_headers = __msg.headers.as_ref();
                    let __response_payload = __msg.payload.to_vec();
                    #deserialize_response
                }
            }
        } else {
            quote! {
                #serialize_payload
                let __subject = self.build_subject(#group, &#endpoint_subject_expr);
                let __msg = options
                    .into_request(&self.client, __subject, __body)
                    .await
                    .map_err(#nats_micro::ClientError::<#error_type>::request)?;
                let __response_headers = __msg.headers.as_ref();
                let __response_payload = __msg.payload.to_vec();
                #deserialize_response
            }
        };

        methods.push(quote! {
            #(#attrs)*
            pub async fn #fn_ident(
                &self,
                #(#all_with_args,)*
            ) -> Result<#return_type, #nats_micro::ClientError<#error_type>> {
                self.#fn_with_ident(#(#forward_args,)* #nats_micro::ClientCallOptions::new()).await
            }

            #(#attrs)*
            pub async fn #fn_with_ident(
                &self,
                #(#all_with_args,)*
                options: #nats_micro::ClientCallOptions,
            ) -> Result<#return_type, #nats_micro::ClientError<#error_type>> {
                #with_body
            }
        });
    }
    let recipient_field = if encryption_enabled {
        quote! {
            recipient: #nats_micro::ServiceRecipient,
        }
    } else {
        quote! {}
    };

    let new_fn = if encryption_enabled {
        quote! {
            pub fn new(
                client: #nats_micro::async_nats::Client,
                recipient: impl Into<#nats_micro::ServiceRecipient>,
            ) -> Self {
                Self {
                    client,
                    prefix: #struct_ident::__nats_micro_service_meta().subject_prefix,
                    recipient: recipient.into(),
                }
            }
        }
    } else {
        quote! {
            pub fn new(client: #nats_micro::async_nats::Client) -> Self {
                Self {
                    client,
                    prefix: #struct_ident::__nats_micro_service_meta().subject_prefix,
                }
            }
        }
    };

    let with_prefix_fn = if encryption_enabled {
        quote! {
            pub fn with_prefix(
                client: #nats_micro::async_nats::Client,
                prefix: impl Into<String>,
                recipient: impl Into<#nats_micro::ServiceRecipient>,
            ) -> Self {
                Self {
                    client,
                    prefix: Some(prefix.into()),
                    recipient: recipient.into(),
                }
            }
        }
    } else {
        quote! {
            pub fn with_prefix(
                client: #nats_micro::async_nats::Client,
                prefix: impl Into<String>,
            ) -> Self {
                Self {
                    client,
                    prefix: Some(prefix.into()),
                }
            }
        }
    };

    let with_recipient_fn = if encryption_enabled {
        quote! {
            pub fn with_recipient(
                mut self,
                recipient: impl Into<#nats_micro::ServiceRecipient>,
            ) -> Self {
                self.recipient = recipient.into();
                self
            }
        }
    } else {
        quote! {}
    };

    let apply_encryption_recipient_fn = if encryption_enabled {
        quote! {
            fn apply_encryption_recipient(
                &self,
                options: #nats_micro::ClientCallOptions,
            ) -> #nats_micro::ClientCallOptions {
                options.with_default_recipient(self.recipient.clone())
            }
        }
    } else {
        quote! {
            fn apply_encryption_recipient(
                &self,
                options: #nats_micro::ClientCallOptions,
            ) -> #nats_micro::ClientCallOptions {
                options
            }
        }
    };

    quote! {
        pub mod #module_name {
            use super::*;

            pub struct #client_struct_name {
                client: #nats_micro::async_nats::Client,
                prefix: Option<String>,
                #recipient_field
            }

            impl #client_struct_name {
                #new_fn

                #[doc(hidden)]
                #with_prefix_fn

                #with_recipient_fn

                fn build_subject(&self, group: &str, endpoint: &str) -> String {
                    #nats_micro::__macros::build_subject(self.prefix.as_deref(), group, endpoint)
                }

                #apply_encryption_recipient_fn

                #(#methods)*
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ServiceArgs, expand_service, generate_client_module, process_consumer_method,
        process_endpoint_method,
    };
    use syn::{ImplItemFn, parse_quote};

    #[test]
    fn service_metadata_includes_prefix_when_present() {
        let tokens = expand_service(
            ServiceArgs {
                name: Some("demo".to_string()),
                version: Some("1.0.0".to_string()),
                description: Some("test".to_string()),
                prefix: Some("api".to_string()),
            },
            parse_quote! {
                struct DemoService;
            },
        );

        let expanded = tokens.to_string();
        assert!(expanded.contains("ServiceMetadata :: new"));
        assert!(expanded.contains("Some (\"api\" . to_string ())"));
    }

    #[test]
    fn generated_client_uses_service_metadata_prefix() {
        let struct_ident = parse_quote!(DemoService);
        let tokens = generate_client_module(&struct_ident, "DemoService", &[]);
        let expanded = tokens.to_string();

        assert!(expanded.contains("DemoService :: __nats_micro_service_meta () . subject_prefix"));
        assert!(
            expanded.contains("build_subject (self . prefix . as_deref () , group , endpoint)")
        );
        if cfg!(feature = "macros_encryption_feature") {
            assert!(expanded.contains("with_default_recipient (self . recipient . clone ())"));
            assert!(!expanded.contains("# [cfg (feature = \"encryption\") ]"));
        }
        assert!(!expanded.contains("# [cfg (feature = \"macros_encryption_feature\") ]"));
    }

    #[test]
    fn generated_endpoint_handlers_only_enable_shutdown_support_when_requested() {
        let struct_ident = parse_quote!(DemoService);
        let idle_method: ImplItemFn = parse_quote! {
            #[endpoint(subject = "status", group = "demo")]
            async fn status() -> Result<&'static str, nats_micro::NatsErrorResponse> {
                Ok("ok")
            }
        };
        let idle_attr = idle_method
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("endpoint"))
            .unwrap();
        let (idle_result, _) =
            process_endpoint_method(&struct_ident, &idle_method, idle_attr).unwrap();
        let idle_tokens = idle_result.def_fn.to_string();

        assert!(idle_tokens.contains("new_with_shutdown_signal_support (false"));

        let shutdown_method: ImplItemFn = parse_quote! {
            #[endpoint(subject = "cleanup", group = "demo")]
            async fn cleanup(
                mut shutdown: nats_micro::ShutdownSignal,
            ) -> Result<&'static str, nats_micro::NatsErrorResponse> {
                shutdown.wait_for_shutdown().await;
                Ok("ok")
            }
        };
        let shutdown_attr = shutdown_method
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("endpoint"))
            .unwrap();
        let (shutdown_result, _) =
            process_endpoint_method(&struct_ident, &shutdown_method, shutdown_attr).unwrap();
        let shutdown_tokens = shutdown_result.def_fn.to_string();

        assert!(shutdown_tokens.contains("new_with_shutdown_signal_support (true"));
    }

    #[test]
    fn generated_consumer_handlers_only_enable_shutdown_support_when_requested() {
        let struct_ident = parse_quote!(DemoService);
        let idle_method: ImplItemFn = parse_quote! {
            #[consumer(stream = "DEMO", durable = "jobs")]
            async fn jobs() -> Result<(), nats_micro::NatsErrorResponse> {
                Ok(())
            }
        };
        let idle_attr = idle_method
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("consumer"))
            .unwrap();
        let idle_result = process_consumer_method(&struct_ident, &idle_method, idle_attr).unwrap();
        let idle_tokens = idle_result.def_fn.to_string();

        assert!(idle_tokens.contains("new_with_shutdown_signal_support (false"));

        let shutdown_method: ImplItemFn = parse_quote! {
            #[consumer(stream = "DEMO", durable = "cleanup-jobs")]
            async fn cleanup_jobs(
                mut shutdown: nats_micro::ShutdownSignal,
            ) -> Result<(), nats_micro::NatsErrorResponse> {
                shutdown.wait_for_shutdown().await;
                Ok(())
            }
        };
        let shutdown_attr = shutdown_method
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("consumer"))
            .unwrap();
        let shutdown_result =
            process_consumer_method(&struct_ident, &shutdown_method, shutdown_attr).unwrap();
        let shutdown_tokens = shutdown_result.def_fn.to_string();

        assert!(shutdown_tokens.contains("new_with_shutdown_signal_support (true"));
    }
}
