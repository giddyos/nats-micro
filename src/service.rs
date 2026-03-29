use darling::FromMeta;
use darling::ast::NestedMeta;
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ImplItem, ImplItemFn, ItemImpl, ItemStruct, Meta, Type};

use crate::consumer::ConsumerArgs;
use crate::endpoint::{
    EndpointArgs, PayloadEncoding, PayloadMeta, ResponseEncoding, ResponseMeta, build_handler_body,
    classify_return_type, extract_client_meta, extract_param_info, parse_subject_template,
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
    let client_module = if cfg!(feature = "client") {
        generate_client_module(&service_name_str, &client_endpoints)
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
    let client_meta = extract_client_meta(&method.sig)?;

    let payload_meta_tokens = match &client_meta.payload {
        Some(pm) => {
            let enc = payload_encoding_tokens(&pm.encoding);
            let encrypted = pm.encrypted;
            let inner_type = &pm.inner_type;
            let inner_type_str = quote!(#inner_type).to_string();
            quote! {
                Some(::nats_micro::__macros::PayloadMeta {
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
        ::nats_micro::__macros::ResponseMeta {
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
            pub fn #def_method_name() -> ::nats_micro::EndpointDefinition {
                let __meta = Self::__nats_micro_service_meta();
                ::nats_micro::EndpointDefinition {
                    service_name: __meta.name,
                    service_version: __meta.version,
                    service_description: __meta.description,
                    fn_name: #fn_name_str.to_string(),
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
                payload_meta: #payload_meta_tokens,
                response_meta: #response_meta_tokens,
            }
        },
    };

    let client_data = ClientEndpointData {
        attrs,
        fn_name: fn_name.clone(),
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
    let attrs = conditional_attrs(method);

    Ok(HandlerResult {
        attrs,
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

fn payload_encoding_tokens(enc: &PayloadEncoding) -> TokenStream {
    match enc {
        PayloadEncoding::Json => quote! { ::nats_micro::__macros::PayloadEncoding::Json },
        PayloadEncoding::Proto => quote! { ::nats_micro::__macros::PayloadEncoding::Proto },
        PayloadEncoding::Serde => quote! { ::nats_micro::__macros::PayloadEncoding::Serde },
        PayloadEncoding::Raw => quote! { ::nats_micro::__macros::PayloadEncoding::Raw },
    }
}

fn response_encoding_tokens(enc: &ResponseEncoding) -> TokenStream {
    match enc {
        ResponseEncoding::Json => quote! { ::nats_micro::__macros::ResponseEncoding::Json },
        ResponseEncoding::Proto => quote! { ::nats_micro::__macros::ResponseEncoding::Proto },
        ResponseEncoding::Serde => quote! { ::nats_micro::__macros::ResponseEncoding::Serde },
        ResponseEncoding::Raw => quote! { ::nats_micro::__macros::ResponseEncoding::Raw },
        ResponseEncoding::Unit => quote! { ::nats_micro::__macros::ResponseEncoding::Unit },
    }
}

fn generate_client_module(service_name_str: &str, endpoints: &[ClientEndpointData]) -> TokenStream {
    let module_name = format_ident!("{}_client", service_name_str.to_snake_case());
    let client_struct_name = format_ident!("{}Client", service_name_str);

    let mut methods = Vec::new();

    for ep in endpoints {
        let attrs = &ep.attrs;
        let fn_ident = &ep.fn_name;
        let fn_with_ident = format_ident!("{}_with", fn_ident);
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
                    quote! { let __body = ::nats_micro::__macros::serialize_serde_payload(payload)?; }
                }
                PayloadEncoding::Proto => {
                    quote! { let __body = ::nats_micro::__macros::serialize_proto_payload(payload)?; }
                }
                PayloadEncoding::Raw => {
                    quote! { let __body = ::nats_micro::Bytes::copy_from_slice(payload.as_ref()); }
                }
            }
        } else {
            quote! { let __body = ::nats_micro::Bytes::new(); }
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
            ResponseEncoding::Unit => quote! { Ok(()) },
            ResponseEncoding::Json | ResponseEncoding::Serde => {
                quote! { ::nats_micro::__macros::deserialize_response(&__response_payload) }
            }
            ResponseEncoding::Proto => {
                quote! { ::nats_micro::__macros::deserialize_proto_response(&__response_payload) }
            }
            ResponseEncoding::Raw => {
                if let Some(rt) = response_type {
                    let ident = crate::endpoint::last_segment_ident(rt);
                    let is_str_ref = matches!(rt, Type::Reference(r) if matches!(&*r.elem, Type::Path(p) if p.path.is_ident("str")));
                    match ident.as_deref() {
                        Some("String") => {
                            quote! { ::nats_micro::__macros::raw_response_to_string(&__response_payload) }
                        }
                        _ if is_str_ref => {
                            quote! { ::nats_micro::__macros::raw_response_to_string(&__response_payload) }
                        }
                        _ => {
                            quote! { ::nats_micro::__macros::raw_response_to_bytes(&__response_payload) }
                        }
                    }
                } else {
                    quote! { ::nats_micro::__macros::raw_response_to_bytes(&__response_payload) }
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
                    let (__msg, __eph_ctx) = options
                        .into_encrypted_request(&self.client, __subject, __body.to_vec())
                        .await?;
                    let __response_payload = __eph_ctx.decrypt_response(&__msg.payload).map_err(|e| {
                        ::nats_micro::NatsErrorResponse::internal("DECRYPT_ERROR", e.to_string())
                    })?;
                    #deserialize_response
                }
            } else if payload_encrypted {
                quote! {
                    #serialize_payload
                    let __subject = self.build_subject(#group, &#endpoint_subject_expr);
                    let (__msg, _) = options
                        .into_encrypted_request(&self.client, __subject, __body.to_vec())
                        .await?;
                    let __response_payload = __msg.payload.to_vec();
                    #deserialize_response
                }
            } else {
                quote! {
                    #serialize_payload
                    let __subject = self.build_subject(#group, &#endpoint_subject_expr);
                    let __msg = options
                        .into_request(&self.client, __subject, __body)
                        .await?;
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
                    .await?;
                let __response_payload = __msg.payload.to_vec();
                #deserialize_response
            }
        };

        methods.push(quote! {
            #(#attrs)*
            pub async fn #fn_ident(
                &self,
                #(#all_with_args,)*
            ) -> Result<#return_type, ::nats_micro::NatsErrorResponse> {
                self.#fn_with_ident(#(#forward_args,)* ::nats_micro::ClientCallOptions::new()).await
            }

            #(#attrs)*
            pub async fn #fn_with_ident(
                &self,
                #(#all_with_args,)*
                options: ::nats_micro::ClientCallOptions,
            ) -> Result<#return_type, ::nats_micro::NatsErrorResponse> {
                #with_body
            }
        });
    }

    let service_name_lit = service_name_str;

    quote! {
        pub mod #module_name {
            use super::*;

            pub struct #client_struct_name {
                client: ::nats_micro::async_nats::Client,
                prefix: String,
            }

            impl #client_struct_name {
                pub fn new(client: ::nats_micro::async_nats::Client) -> Self {
                    Self {
                        client,
                        prefix: #service_name_lit.to_string(),
                    }
                }

                pub fn with_prefix(
                    client: ::nats_micro::async_nats::Client,
                    prefix: impl Into<String>,
                ) -> Self {
                    Self {
                        client,
                        prefix: prefix.into(),
                    }
                }

                fn build_subject(&self, group: &str, endpoint: &str) -> String {
                    if group.is_empty() {
                        format!("{}.{}", self.prefix, endpoint)
                    } else {
                        format!("{}.{}.{}", self.prefix, group, endpoint)
                    }
                }

                #(#methods)*
            }
        }
    }
}
