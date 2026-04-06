use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ImplItemFn, Type};

use crate::endpoint::{
    EndpointArgs, PayloadEncoding, PayloadMeta, ResponseEncoding, ResponseMeta, SubjectParamMeta,
    extract_client_meta, extract_result_types, last_segment_ident, parse_subject_template,
    validate_template_bindings,
};
use crate::utils::{nats_micro_path, spanned_trait_assertion};

#[derive(Debug, Clone)]
pub(crate) struct ClientModuleSpec {
    pub module_name: syn::Ident,
    pub client_struct_name: syn::Ident,
    pub service_ident: syn::Ident,
    pub encryption_enabled: bool,
    pub endpoints: Vec<ClientEndpointSpec>,
}

impl ClientModuleSpec {
    fn render_rust_tokens(&self) -> TokenStream {
        let nats_micro = nats_micro_path();
        let module_name = &self.module_name;
        let client_struct_name = &self.client_struct_name;
        let methods: Vec<_> = self.endpoints.iter().map(render_client_method).collect();
        let recipient_field = self.recipient_field_tokens(&nats_micro);
        let new_fn = self.new_fn_tokens(&nats_micro);
        let with_prefix_fn = self.with_prefix_fn_tokens(&nats_micro);
        let with_recipient_fn = self.with_recipient_fn_tokens(&nats_micro);
        let apply_client_defaults_fn = self.apply_client_defaults_fn_tokens(&nats_micro);

        quote! {
            pub mod #module_name {
                use super::*;

                #[derive(Clone)]
                pub struct #client_struct_name {
                    client: #nats_micro::async_nats::Client,
                    prefix: Option<String>,
                    service_version: String,
                    #recipient_field
                }

                impl #client_struct_name {
                    #new_fn

                    #[doc(hidden)]
                    #with_prefix_fn

                    #with_recipient_fn

                    fn build_subject(&self, group: &str, endpoint: &str) -> String {
                        #nats_micro::__macros::build_subject(
                            self.prefix.as_deref(),
                            &self.service_version,
                            group,
                            endpoint,
                        )
                    }

                    #apply_client_defaults_fn

                    #(#methods)*
                }
            }
        }
    }

    fn recipient_field_tokens(&self, nats_micro: &syn::Path) -> TokenStream {
        if self.encryption_enabled {
            quote! {
                recipient: Option<#nats_micro::ServiceRecipient>,
            }
        } else {
            quote! {}
        }
    }

    fn new_fn_tokens(&self, nats_micro: &syn::Path) -> TokenStream {
        let service_ident = &self.service_ident;
        if self.encryption_enabled {
            quote! {
                pub fn new(
                    client: #nats_micro::async_nats::Client,
                    recipient_public_key: Option<[u8; 32]>,
                ) -> Self {
                    let service_meta = #service_ident::__nats_micro_service_meta();
                    Self {
                        client,
                        prefix: service_meta.subject_prefix,
                        service_version: service_meta.version,
                        recipient: recipient_public_key.map(#nats_micro::ServiceRecipient::from_bytes),
                    }
                }
            }
        } else {
            quote! {
                pub fn new(client: #nats_micro::async_nats::Client) -> Self {
                    let service_meta = #service_ident::__nats_micro_service_meta();
                    Self {
                        client,
                        prefix: service_meta.subject_prefix,
                        service_version: service_meta.version,
                    }
                }
            }
        }
    }

    fn with_prefix_fn_tokens(&self, nats_micro: &syn::Path) -> TokenStream {
        let service_ident = &self.service_ident;
        if self.encryption_enabled {
            quote! {
                pub fn with_prefix(
                    client: #nats_micro::async_nats::Client,
                    prefix: impl Into<String>,
                    recipient_public_key: Option<[u8; 32]>,
                ) -> Self {
                    let service_meta = #service_ident::__nats_micro_service_meta();
                    Self {
                        client,
                        prefix: Some(prefix.into()),
                        service_version: service_meta.version,
                        recipient: recipient_public_key.map(#nats_micro::ServiceRecipient::from_bytes),
                    }
                }
            }
        } else {
            quote! {
                pub fn with_prefix(
                    client: #nats_micro::async_nats::Client,
                    prefix: impl Into<String>,
                ) -> Self {
                    let service_meta = #service_ident::__nats_micro_service_meta();
                    Self {
                        client,
                        prefix: Some(prefix.into()),
                        service_version: service_meta.version,
                    }
                }
            }
        }
    }

    fn with_recipient_fn_tokens(&self, nats_micro: &syn::Path) -> TokenStream {
        if self.encryption_enabled {
            quote! {
                pub fn with_recipient(
                    mut self,
                    recipient: impl Into<#nats_micro::ServiceRecipient>,
                ) -> Self {
                    self.recipient = Some(recipient.into());
                    self
                }
            }
        } else {
            quote! {}
        }
    }

    fn apply_client_defaults_fn_tokens(&self, nats_micro: &syn::Path) -> TokenStream {
        if self.encryption_enabled {
            quote! {
                fn apply_client_defaults(
                    &self,
                    options: #nats_micro::ClientCallOptions,
                ) -> #nats_micro::ClientCallOptions {
                    let options = match &self.recipient {
                        Some(recipient) => options.with_default_recipient(recipient.clone()),
                        None => options,
                    };

                    options.with_required_header(
                        #nats_micro::X_CLIENT_VERSION_HEADER,
                        self.service_version.clone(),
                    )
                }
            }
        } else {
            quote! {
                fn apply_client_defaults(
                    &self,
                    options: #nats_micro::ClientCallOptions,
                ) -> #nats_micro::ClientCallOptions {
                    options.with_required_header(
                        #nats_micro::X_CLIENT_VERSION_HEADER,
                        self.service_version.clone(),
                    )
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ClientEndpointSpec {
    pub attrs: Vec<syn::Attribute>,
    pub fn_name: syn::Ident,
    pub error_type: Type,
    pub group: String,
    pub subject: ClientSubjectSpec,
    pub payload_meta: Option<PayloadMeta>,
    pub response_meta: ResponseMeta,
    pub payload: Option<ClientPayloadSpec>,
    pub response: ClientResponseSpec,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientSubjectSpec {
    pub template: String,
    pub pattern: String,
    pub params: Vec<SubjectParamMeta>,
}

impl ClientSubjectSpec {
    pub(crate) fn is_templated(&self) -> bool {
        !self.params.is_empty()
    }

    fn parameter_args(&self) -> Vec<TokenStream> {
        self.params
            .iter()
            .map(|param| {
                let name = format_ident!("{}", param.name);
                let ty = &param.inner_type;
                quote! { #name: &#ty }
            })
            .collect()
    }

    fn parameter_idents(&self) -> Vec<syn::Ident> {
        self.params
            .iter()
            .map(|param| format_ident!("{}", param.name))
            .collect()
    }

    fn endpoint_expr(&self) -> TokenStream {
        if self.params.is_empty() {
            let pattern = &self.pattern;
            return quote! { #pattern.to_string() };
        }

        let mut fmt_str = String::new();
        let mut fmt_args = Vec::new();
        for segment in self.template.split('.') {
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawValueKind {
    Bytes,
    String,
}

#[derive(Debug, Clone)]
pub(crate) enum ClientPayloadShape {
    Json(Type),
    Proto(Type),
    Serde(Type),
    Raw(RawValueKind),
}

#[derive(Debug, Clone)]
pub(crate) struct ClientPayloadSpec {
    pub shape: ClientPayloadShape,
    pub optional: bool,
    pub encrypted: bool,
}

impl ClientPayloadSpec {
    fn from_meta(meta: &PayloadMeta) -> Self {
        let shape = match meta.encoding {
            PayloadEncoding::Json => ClientPayloadShape::Json(meta.inner_type.clone()),
            PayloadEncoding::Proto => ClientPayloadShape::Proto(meta.inner_type.clone()),
            PayloadEncoding::Serde => ClientPayloadShape::Serde(meta.inner_type.clone()),
            PayloadEncoding::Raw => ClientPayloadShape::Raw(raw_value_kind(&meta.inner_type)),
        };

        Self {
            shape,
            optional: meta.optional,
            encrypted: meta.encrypted,
        }
    }

    fn fn_args(&self) -> Vec<TokenStream> {
        match &self.shape {
            ClientPayloadShape::Raw(RawValueKind::String) if self.optional => {
                vec![quote! { payload: Option<&str> }]
            }
            ClientPayloadShape::Raw(RawValueKind::String) => vec![quote! { payload: &str }],
            ClientPayloadShape::Raw(RawValueKind::Bytes) if self.optional => {
                vec![quote! { payload: Option<&[u8]> }]
            }
            ClientPayloadShape::Raw(RawValueKind::Bytes) => vec![quote! { payload: &[u8] }],
            ClientPayloadShape::Json(ty)
            | ClientPayloadShape::Proto(ty)
            | ClientPayloadShape::Serde(ty)
                if self.optional =>
            {
                vec![quote! { payload: Option<&#ty> }]
            }
            ClientPayloadShape::Json(ty)
            | ClientPayloadShape::Proto(ty)
            | ClientPayloadShape::Serde(ty) => vec![quote! { payload: &#ty }],
        }
    }

    fn serialize_tokens(&self, error_type: &Type) -> TokenStream {
        let nats_micro = nats_micro_path();
        match &self.shape {
            ClientPayloadShape::Json(_) | ClientPayloadShape::Serde(_) => {
                quote! {
                    let __body = #nats_micro::__macros::serialize_serde_payload(payload)
                        .map_err(#nats_micro::ClientError::<#error_type>::serialize)?;
                }
            }
            ClientPayloadShape::Proto(_) => {
                quote! {
                    let __body = #nats_micro::__macros::serialize_proto_payload(payload)
                        .map_err(#nats_micro::ClientError::<#error_type>::serialize)?;
                }
            }
            ClientPayloadShape::Raw(RawValueKind::String) => {
                quote! {
                    let __body = #nats_micro::Bytes::copy_from_slice(payload.as_bytes());
                }
            }
            ClientPayloadShape::Raw(RawValueKind::Bytes) => {
                quote! {
                    let __body = #nats_micro::Bytes::copy_from_slice(payload.as_ref());
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ClientResponseShape {
    Unit,
    Json(Type),
    Proto(Type),
    Serde(Type),
    Raw(RawValueKind),
}

#[derive(Debug, Clone)]
pub(crate) struct ClientResponseSpec {
    pub shape: ClientResponseShape,
    pub optional: bool,
    pub encrypted: bool,
}

impl ClientResponseSpec {
    fn from_meta(meta: &ResponseMeta) -> Self {
        let shape = match meta.encoding {
            ResponseEncoding::Unit => ClientResponseShape::Unit,
            ResponseEncoding::Json => ClientResponseShape::Json(
                meta.inner_type
                    .clone()
                    .expect("json responses always carry an inner type"),
            ),
            ResponseEncoding::Proto => ClientResponseShape::Proto(
                meta.inner_type
                    .clone()
                    .expect("proto responses always carry an inner type"),
            ),
            ResponseEncoding::Serde => ClientResponseShape::Serde(
                meta.inner_type
                    .clone()
                    .expect("serde responses always carry an inner type"),
            ),
            ResponseEncoding::Raw => ClientResponseShape::Raw(raw_value_kind(
                meta.inner_type
                    .as_ref()
                    .expect("raw responses always carry an inner type"),
            )),
        };

        Self {
            shape,
            optional: meta.optional,
            encrypted: meta.encrypted,
        }
    }

    fn return_type_tokens(&self) -> TokenStream {
        let inner = match &self.shape {
            ClientResponseShape::Unit => quote! { () },
            ClientResponseShape::Json(ty)
            | ClientResponseShape::Proto(ty)
            | ClientResponseShape::Serde(ty) => quote! { #ty },
            ClientResponseShape::Raw(RawValueKind::String) => quote! { String },
            ClientResponseShape::Raw(RawValueKind::Bytes) => quote! { Vec<u8> },
        };

        if self.optional {
            quote! { Option<#inner> }
        } else {
            inner
        }
    }

    fn deserialize_tokens(&self, error_type: &Type) -> TokenStream {
        let nats_micro = nats_micro_path();

        match (&self.shape, self.optional) {
            (ClientResponseShape::Unit, true) => {
                quote! {
                    #nats_micro::__macros::deserialize_optional_unit_response::<#error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Unit, false) => {
                quote! {
                    #nats_micro::__macros::deserialize_unit_response::<#error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Json(ty) | ClientResponseShape::Serde(ty), true) => {
                quote! {
                    #nats_micro::__macros::deserialize_optional_response::<#ty, #error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Json(ty) | ClientResponseShape::Serde(ty), false) => {
                quote! {
                    #nats_micro::__macros::deserialize_response::<#ty, #error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Proto(ty), true) => {
                quote! {
                    #nats_micro::__macros::deserialize_optional_proto_response::<#ty, #error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Proto(ty), false) => {
                quote! {
                    #nats_micro::__macros::deserialize_proto_response::<#ty, #error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Raw(RawValueKind::String), true) => {
                quote! {
                    #nats_micro::__macros::raw_response_to_optional_string::<#error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Raw(RawValueKind::String), false) => {
                quote! {
                    #nats_micro::__macros::raw_response_to_string::<#error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Raw(RawValueKind::Bytes), true) => {
                quote! {
                    #nats_micro::__macros::raw_response_to_optional_bytes::<#error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
            (ClientResponseShape::Raw(RawValueKind::Bytes), false) => {
                quote! {
                    #nats_micro::__macros::raw_response_to_bytes::<#error_type>(
                        __response_headers,
                        &__response_payload,
                    )
                }
            }
        }
    }
}

fn render_client_assertions(endpoint: &ClientEndpointSpec) -> Vec<TokenStream> {
    let nats_micro = nats_micro_path();
    let mut assertions = vec![spanned_trait_assertion(
        &endpoint.error_type,
        &quote!(#nats_micro::FromNatsErrorResponse + ::std::fmt::Debug + ::std::fmt::Display + 'static),
    )];

    if let Some(payload) = endpoint.payload.as_ref() {
        match &payload.shape {
            ClientPayloadShape::Json(ty) | ClientPayloadShape::Serde(ty) => {
                assertions.push(spanned_trait_assertion(
                    ty,
                    &quote!(#nats_micro::serde::Serialize),
                ));
            }
            ClientPayloadShape::Proto(ty) => {
                assertions.push(spanned_trait_assertion(
                    ty,
                    &quote!(#nats_micro::prost::Message),
                ));
            }
            ClientPayloadShape::Raw(_) => {}
        }
    }

    match &endpoint.response.shape {
        ClientResponseShape::Json(ty) | ClientResponseShape::Serde(ty) => {
            assertions.push(spanned_trait_assertion(
                ty,
                &quote!(#nats_micro::serde::de::DeserializeOwned),
            ));
        }
        ClientResponseShape::Proto(ty) => {
            assertions.push(spanned_trait_assertion(
                ty,
                &quote!(#nats_micro::prost::Message + ::std::default::Default),
            ));
        }
        ClientResponseShape::Unit | ClientResponseShape::Raw(_) => {}
    }

    assertions
}

pub(crate) fn build_endpoint_client_spec(
    method: &ImplItemFn,
    args: &EndpointArgs,
    attrs: Vec<syn::Attribute>,
) -> Result<ClientEndpointSpec, TokenStream> {
    let (pattern, template_params) = parse_subject_template(&args.subject, &method.sig.ident)?;
    validate_template_bindings(&method.sig, &template_params)?;

    let (_, error_type) =
        extract_result_types(&method.sig).map_err(|error| error.to_compile_error())?;
    let client_meta = extract_client_meta(&method.sig)?;
    let payload_meta = client_meta.payload;
    let response_meta = client_meta.response;
    let payload = payload_meta.as_ref().map(ClientPayloadSpec::from_meta);
    let response = ClientResponseSpec::from_meta(&response_meta);

    Ok(ClientEndpointSpec {
        attrs,
        fn_name: method.sig.ident.clone(),
        error_type: error_type.clone(),
        group: args.group.clone().unwrap_or_default(),
        subject: ClientSubjectSpec {
            template: args.subject.clone(),
            pattern,
            params: client_meta.subject_params,
        },
        payload_meta,
        response_meta,
        payload,
        response,
    })
}

pub(crate) fn build_client_module_spec(
    struct_ident: &syn::Ident,
    service_name_str: &str,
    endpoints: &[ClientEndpointSpec],
) -> ClientModuleSpec {
    ClientModuleSpec {
        module_name: format_ident!("{}_client", service_name_str.to_snake_case()),
        client_struct_name: format_ident!("{}Client", service_name_str),
        service_ident: struct_ident.clone(),
        encryption_enabled: cfg!(feature = "macros_encryption_feature"),
        endpoints: endpoints.to_vec(),
    }
}

pub(crate) fn generate_client_module(
    struct_ident: &syn::Ident,
    service_name_str: &str,
    endpoints: &[ClientEndpointSpec],
) -> TokenStream {
    build_client_module_spec(struct_ident, service_name_str, endpoints).render_rust_tokens()
}

fn raw_value_kind(ty: &Type) -> RawValueKind {
    let is_str_ref = matches!(ty, Type::Reference(reference) if matches!(&*reference.elem, Type::Path(path) if path.path.is_ident("str")));
    match last_segment_ident(ty).as_deref() {
        Some("String") => RawValueKind::String,
        _ if is_str_ref => RawValueKind::String,
        _ => RawValueKind::Bytes,
    }
}

fn empty_body_expr() -> TokenStream {
    let nats_micro = nats_micro_path();
    quote! { #nats_micro::Bytes::new() }
}

fn render_client_method(endpoint: &ClientEndpointSpec) -> TokenStream {
    let nats_micro = nats_micro_path();
    let attrs = &endpoint.attrs;
    let fn_ident = &endpoint.fn_name;
    let fn_with_ident = format_ident!("{}_with", fn_ident);
    let error_type = &endpoint.error_type;
    let return_type = endpoint.response.return_type_tokens();
    let assertions = render_client_assertions(endpoint);

    let subject_param_args = endpoint.subject.parameter_args();
    let payload_fn_args = endpoint
        .payload
        .as_ref()
        .map_or_else(Vec::new, ClientPayloadSpec::fn_args);
    let all_with_args: Vec<_> = subject_param_args
        .iter()
        .cloned()
        .chain(payload_fn_args.iter().cloned())
        .collect();

    let mut forward_args: Vec<TokenStream> = endpoint
        .subject
        .parameter_idents()
        .into_iter()
        .map(|ident| quote! { #ident })
        .collect();
    if endpoint.payload.is_some() {
        forward_args.push(quote! { payload });
    }

    let with_body = render_with_body(endpoint);

    quote! {
        #(#attrs)*
        pub async fn #fn_ident(
            &self,
            #(#all_with_args,)*
        ) -> Result<#return_type, #nats_micro::ClientError<#error_type>> {
            self.#fn_with_ident(#(#forward_args,)* #nats_micro::ClientCallOptions::new())
                .await
        }

        #(#attrs)*
        pub async fn #fn_with_ident(
            &self,
            #(#all_with_args,)*
            options: #nats_micro::ClientCallOptions,
        ) -> Result<#return_type, #nats_micro::ClientError<#error_type>> {
            #(#assertions)*
            #with_body
        }
    }
}

fn render_with_body(endpoint: &ClientEndpointSpec) -> TokenStream {
    let error_type = &endpoint.error_type;

    match endpoint.payload.as_ref() {
        Some(payload) if payload.optional => {
            let serialize_payload = payload.serialize_tokens(error_type);
            let some_body = quote! { __body };
            let none_body = empty_body_expr();
            let some_request = render_request_execution(endpoint, &some_body, payload.encrypted);
            let none_request = render_request_execution(endpoint, &none_body, false);
            quote! {
                if let Some(payload) = payload {
                    #serialize_payload
                    #some_request
                } else {
                    #none_request
                }
            }
        }
        Some(payload) => {
            let serialize_payload = payload.serialize_tokens(error_type);
            let body = quote! { __body };
            let request = render_request_execution(endpoint, &body, payload.encrypted);
            quote! {
                #serialize_payload
                #request
            }
        }
        None => {
            let body = empty_body_expr();
            render_request_execution(endpoint, &body, false)
        }
    }
}

fn render_request_execution(
    endpoint: &ClientEndpointSpec,
    body_expr: &TokenStream,
    request_encrypted: bool,
) -> TokenStream {
    let nats_micro = nats_micro_path();
    let error_type = &endpoint.error_type;
    let group = &endpoint.group;
    let endpoint_subject_expr = endpoint.subject.endpoint_expr();
    let deserialize_response = endpoint.response.deserialize_tokens(error_type);
    let response_encrypted = endpoint.response.encrypted;
    let encrypted_body_expr = quote! { (#body_expr).to_vec() };

    match (request_encrypted, response_encrypted) {
        (true, true) => quote! {
            let __subject = self.build_subject(#group, &#endpoint_subject_expr);
            let options = self.apply_client_defaults(options);
            let (__msg, __eph_ctx) = options
                .into_encrypted_request(&self.client, __subject, #encrypted_body_expr)
                .await
                .map_err(#nats_micro::ClientError::<#error_type>::request)?;
            let __response_headers = __msg.headers.as_ref();
            let __response_payload = #nats_micro::__macros::decrypt_client_response::<#error_type>(
                __response_headers,
                &__eph_ctx,
                &__msg.payload,
            )?;
            #deserialize_response
        },
        (true, false) => quote! {
            let __subject = self.build_subject(#group, &#endpoint_subject_expr);
            let options = self.apply_client_defaults(options);
            let (__msg, _) = options
                .into_encrypted_request(&self.client, __subject, #encrypted_body_expr)
                .await
                .map_err(#nats_micro::ClientError::<#error_type>::request)?;
            let __response_headers = __msg.headers.as_ref();
            let __response_payload = __msg.payload.to_vec();
            #deserialize_response
        },
        (false, true) => quote! {
            let __subject = self.build_subject(#group, &#endpoint_subject_expr);
            let options = self.apply_client_defaults(options);
            let (__msg, __eph_ctx) = options
                .into_request_with_context(&self.client, __subject, #body_expr)
                .await
                .map_err(#nats_micro::ClientError::<#error_type>::request)?;
            let __response_headers = __msg.headers.as_ref();
            let __eph_ctx = __eph_ctx.as_ref().ok_or_else(|| {
                #nats_micro::ClientError::<#error_type>::invalid_response(
                    #nats_micro::__macros::invalid_response(
                        "encrypted response requested without encryption context",
                    ),
                )
            })?;
            let __response_payload = #nats_micro::__macros::decrypt_client_response::<#error_type>(
                __response_headers,
                __eph_ctx,
                &__msg.payload,
            )?;
            #deserialize_response
        },
        (false, false) => quote! {
            let __subject = self.build_subject(#group, &#endpoint_subject_expr);
            let options = self.apply_client_defaults(options);
            let __msg = options
                .into_request(&self.client, __subject, #body_expr)
                .await
                .map_err(#nats_micro::ClientError::<#error_type>::request)?;
            let __response_headers = __msg.headers.as_ref();
            let __response_payload = __msg.payload.to_vec();
            #deserialize_response
        },
    }
}

#[cfg(test)]
#[path = "tests/client_tests.rs"]
mod tests;
