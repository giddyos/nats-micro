use std::collections::BTreeSet;

use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use nats_micro_shared::{FrameworkError, TransportError as SharedTransportError};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::{LitStr, Type, spanned::Spanned};

use crate::{
    client::{ClientEndpointSpec, ClientPayloadShape, ClientResponseShape, RawValueKind},
    endpoint::SubjectParamMeta,
    utils::nats_micro_path,
};

fn dto_payload_type(endpoint: &ClientEndpointSpec) -> Option<Type> {
    let payload = endpoint.payload.as_ref()?;
    match &payload.shape {
        ClientPayloadShape::Json(ty)
        | ClientPayloadShape::Proto(ty)
        | ClientPayloadShape::Serde(ty) => Some(ty.clone()),
        ClientPayloadShape::Raw(_) => None,
    }
}

fn dto_response_type(endpoint: &ClientEndpointSpec) -> Option<Type> {
    match &endpoint.response.shape {
        ClientResponseShape::Json(ty)
        | ClientResponseShape::Proto(ty)
        | ClientResponseShape::Serde(ty) => Some(ty.clone()),
        ClientResponseShape::Unit | ClientResponseShape::Raw(_) => None,
    }
}

#[derive(Debug, Clone)]
struct NapiServiceErrorSpec {
    error_type: Type,
    js_enum_name: String,
}

fn last_type_ident(ty: &Type) -> Option<syn::Ident> {
    let Type::Path(type_path) = ty else {
        return None;
    };

    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.clone())
}

fn is_generic_napi_error_type(ty: &Type) -> bool {
    matches!(
        last_type_ident(ty)
            .as_ref()
            .map(std::string::ToString::to_string)
            .as_deref(),
        Some("NatsErrorResponse" | "NatsError")
    )
}

fn napi_service_error_spec(ty: &Type) -> Option<NapiServiceErrorSpec> {
    if is_generic_napi_error_type(ty) {
        return None;
    }

    let ident = last_type_ident(ty)?;
    Some(NapiServiceErrorSpec {
        error_type: ty.clone(),
        js_enum_name: format!("Js{ident}"),
    })
}

fn collect_napi_service_error_specs(endpoints: &[ClientEndpointSpec]) -> Vec<NapiServiceErrorSpec> {
    let mut seen = BTreeSet::new();
    let mut specs = Vec::new();

    for endpoint in endpoints {
        let Some(spec) = napi_service_error_spec(&endpoint.error_type) else {
            continue;
        };

        let error_type = &spec.error_type;
        if seen.insert(quote! { #error_type }.to_string()) {
            specs.push(spec);
        }
    }

    specs
}

fn client_error_base_name(service_name: &str) -> String {
    let normalized = service_name.to_upper_camel_case();
    let base = normalized.strip_suffix("Service").unwrap_or(&normalized);

    if base.ends_with("Client") {
        base.to_string()
    } else {
        format!("{base}Client")
    }
}

fn framework_error_kind_values() -> Vec<&'static str> {
    FrameworkError::ALL
        .iter()
        .copied()
        .filter(|error| {
            cfg!(feature = "macros_encryption_feature")
                || !matches!(
                    error,
                    FrameworkError::DecryptError
                        | FrameworkError::DecryptFailed
                        | FrameworkError::EncryptFailed
                        | FrameworkError::EncryptRequired
                        | FrameworkError::MissingRecipientPubkey
                        | FrameworkError::NoEncryptionKey
                        | FrameworkError::SignatureInvalid
                        | FrameworkError::SignatureMissing
                )
        })
        .map(FrameworkError::as_code)
        .collect()
}

fn transport_error_kind_values() -> Vec<&'static str> {
    SharedTransportError::ALL
        .iter()
        .copied()
        .map(SharedTransportError::as_code)
        .collect()
}

fn render_string_enum(
    enum_ident: &syn::Ident,
    kinds: &[&str],
    nats_micro: &syn::Path,
) -> TokenStream {
    let variants: Vec<_> = kinds.iter().map(|kind| format_ident!("{}", kind)).collect();
    let as_ref_arms = variants.iter().zip(kinds.iter()).map(|(variant, kind)| {
        quote! {
            Self::#variant => #kind,
        }
    });

    quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[#nats_micro::napi_derive::napi(string_enum)]
        pub enum #enum_ident {
            #(#variants,)*
        }

        impl ::std::convert::AsRef<str> for #enum_ident {
            fn as_ref(&self) -> &str {
                match self {
                    #(#as_ref_arms)*
                }
            }
        }
    }
}

fn render_flagged_error_interface(
    rust_name: &syn::Ident,
    js_name: &str,
    flag_field: &syn::Ident,
    kind_ts_type: &str,
    nats_micro: &syn::Path,
) -> TokenStream {
    quote! {
        #[#nats_micro::napi_derive::napi(object, js_name = #js_name)]
        pub struct #rust_name {
            #[#nats_micro::napi_derive::napi(ts_type = "true")]
            pub #flag_field: bool,
            #[#nats_micro::napi_derive::napi(ts_type = #kind_ts_type)]
            pub kind: String,
            pub code: String,
            pub message: String,
            pub request_id: String,
            pub status_code: u16,
            pub details: Option<String>,
        }
    }
}

fn render_framework_and_transport_error_items(
    service_name: &str,
    nats_micro: &syn::Path,
) -> TokenStream {
    let client_base_name = client_error_base_name(service_name);

    let framework_enum_ident = format_ident!("Js{}FrameworkError", client_base_name);
    let framework_struct_ident = format_ident!("Napi{}FrameworkError", client_base_name);
    let framework_js_name = format!("{client_base_name}FrameworkError");
    let framework_flag_field = format_ident!("is_framework_error");
    let framework_kind_ts_type = format!("{framework_enum_ident} | string");
    let framework_kinds = framework_error_kind_values();

    let transport_enum_ident = format_ident!("Js{}TransportError", client_base_name);
    let transport_struct_ident = format_ident!("Napi{}TransportError", client_base_name);
    let transport_js_name = format!("{client_base_name}TransportError");
    let transport_flag_field = format_ident!("is_transport_error");
    let transport_kind_ts_type = format!("{transport_enum_ident} | string");
    let transport_kinds = transport_error_kind_values();

    let framework_enum = render_string_enum(&framework_enum_ident, &framework_kinds, nats_micro);
    let framework_interface = render_flagged_error_interface(
        &framework_struct_ident,
        &framework_js_name,
        &framework_flag_field,
        &framework_kind_ts_type,
        nats_micro,
    );
    let transport_enum = render_string_enum(&transport_enum_ident, &transport_kinds, nats_micro);
    let transport_interface = render_flagged_error_interface(
        &transport_struct_ident,
        &transport_js_name,
        &transport_flag_field,
        &transport_kind_ts_type,
        nats_micro,
    );

    quote! {
        #framework_enum
        #framework_interface
        #transport_enum
        #transport_interface
    }
}

fn gen_napi_service_error_assert(ty: &Type) -> TokenStream {
    let nats_micro = nats_micro_path();
    quote_spanned! { ty.span() =>
        const _: fn() = || {
            #nats_micro::__private::assert_napi_service_error::<#ty>();
        };
    }
}

fn render_service_error_interface(
    service_name: &str,
    service_error_specs: &[NapiServiceErrorSpec],
    nats_micro: &syn::Path,
) -> TokenStream {
    if service_error_specs.is_empty() {
        return quote! {};
    }

    let client_base_name = client_error_base_name(service_name);

    let rust_name = format_ident!("Napi{}Error", service_name.to_upper_camel_case());
    let js_name = format!("{client_base_name}Error");
    let flag_field = format_ident!("is_service_error");
    let kind_ts_type = service_error_specs
        .iter()
        .map(|spec| spec.js_enum_name.as_str())
        .collect::<Vec<_>>()
        .join(" | ");

    render_flagged_error_interface(&rust_name, &js_name, &flag_field, &kind_ts_type, nats_micro)
}

fn gen_napi_assert(ty: &Type) -> TokenStream {
    let nats_micro = nats_micro_path();
    quote_spanned! { ty.span() =>
        const _: fn() = || {
            #nats_micro::__private::assert_napi_object::<#ty>();
        };
    }
}

pub(crate) fn gen_napi_asserts(endpoints: &[ClientEndpointSpec]) -> TokenStream {
    let mut asserts = Vec::new();

    for endpoint in endpoints {
        if let Some(ty) = dto_payload_type(endpoint) {
            let attrs = &endpoint.attrs;
            let assert = gen_napi_assert(&ty);
            asserts.push(quote! {
                #(#attrs)*
                #assert
            });
        }

        if let Some(ty) = dto_response_type(endpoint) {
            let attrs = &endpoint.attrs;
            let assert = gen_napi_assert(&ty);
            asserts.push(quote! {
                #(#attrs)*
                #assert
            });
        }
    }

    quote! {
        #(#asserts)*
    }
}

fn connect_options_names(service_name: &str) -> (syn::Ident, syn::Ident) {
    (
        format_ident!("{}ClientAuthOptions", service_name.to_upper_camel_case()),
        format_ident!("{}ClientConnectOptions", service_name.to_upper_camel_case()),
    )
}

fn header_type_name(service_name: &str) -> syn::Ident {
    format_ident!("{}ClientHeader", service_name.to_upper_camel_case())
}

fn args_struct_name(service_name: &str, fn_name: &syn::Ident) -> syn::Ident {
    format_ident!(
        "{}{}Args",
        service_name.to_upper_camel_case(),
        fn_name.to_string().to_upper_camel_case()
    )
}

fn payload_arg_type(
    payload: &crate::client::ClientPayloadSpec,
    nats_micro: &syn::Path,
) -> TokenStream {
    let inner = match &payload.shape {
        ClientPayloadShape::Json(ty)
        | ClientPayloadShape::Proto(ty)
        | ClientPayloadShape::Serde(ty) => quote! { #ty },
        ClientPayloadShape::Raw(RawValueKind::String) => quote! { String },
        ClientPayloadShape::Raw(RawValueKind::Bytes) => {
            quote! { #nats_micro::napi::bindgen_prelude::Buffer }
        }
    };

    if payload.optional {
        quote! { Option<#inner> }
    } else {
        inner
    }
}

fn payload_forward_expr(
    payload: &crate::client::ClientPayloadSpec,
    binding: &TokenStream,
) -> TokenStream {
    match (&payload.shape, payload.optional) {
        (
            ClientPayloadShape::Json(_)
            | ClientPayloadShape::Proto(_)
            | ClientPayloadShape::Serde(_),
            true,
        )
        | (ClientPayloadShape::Raw(RawValueKind::Bytes), false) => quote! { #binding.as_ref() },
        (
            ClientPayloadShape::Json(_)
            | ClientPayloadShape::Proto(_)
            | ClientPayloadShape::Serde(_)
            | ClientPayloadShape::Raw(RawValueKind::String),
            false,
        ) => quote! { &#binding },
        (ClientPayloadShape::Raw(RawValueKind::String | RawValueKind::Bytes), true) => {
            quote! { #binding.as_deref() }
        }
    }
}

fn response_return_type(
    response: &crate::client::ClientResponseSpec,
    nats_micro: &syn::Path,
) -> TokenStream {
    let inner = match &response.shape {
        ClientResponseShape::Unit => quote! { () },
        ClientResponseShape::Json(ty)
        | ClientResponseShape::Proto(ty)
        | ClientResponseShape::Serde(ty) => quote! { #ty },
        ClientResponseShape::Raw(RawValueKind::String) => quote! { String },
        ClientResponseShape::Raw(RawValueKind::Bytes) => {
            quote! { #nats_micro::napi::bindgen_prelude::Buffer }
        }
    };

    if response.optional {
        quote! { Option<#inner> }
    } else {
        inner
    }
}

fn response_map_expr(
    response: &crate::client::ClientResponseSpec,
    binding: TokenStream,
    nats_micro: &syn::Path,
) -> TokenStream {
    match (&response.shape, response.optional) {
        (ClientResponseShape::Raw(RawValueKind::Bytes), true) => {
            quote! { #binding.map(#nats_micro::napi::bindgen_prelude::Buffer::from) }
        }
        (ClientResponseShape::Raw(RawValueKind::Bytes), false) => {
            quote! { #nats_micro::napi::bindgen_prelude::Buffer::from(#binding) }
        }
        _ => binding,
    }
}

fn gen_args_struct(
    service_name: &str,
    fn_name: &syn::Ident,
    attrs: &[syn::Attribute],
    fields: &[(syn::Ident, TokenStream)],
    nats_micro: &syn::Path,
) -> (syn::Ident, TokenStream) {
    let struct_name = args_struct_name(service_name, fn_name);
    let field_tokens = fields.iter().map(|(name, ty)| {
        quote! { pub #name: #ty }
    });

    (
        struct_name.clone(),
        quote! {
            #(#attrs)*
            #[#nats_micro::napi_derive::napi(object)]
            pub struct #struct_name {
                #(#field_tokens,)*
            }
        },
    )
}

#[allow(clippy::too_many_lines)]
fn render_connect_support(service_name: &str, nats_micro: &syn::Path) -> TokenStream {
    let (auth_options_name, connect_options_name) = connect_options_names(service_name);
    let header_name = header_type_name(service_name);

    let encrypted_field = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            pub encrypted: bool,
        }
    } else {
        quote! {}
    };

    let encrypted_conversion = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            encrypted: value.encrypted,
        }
    } else {
        quote! {}
    };

    let recipient_field = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            pub recipient_public_key: Option<#nats_micro::napi::bindgen_prelude::Buffer>,
        }
    } else {
        quote! {}
    };

    let recipient_conversion = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            recipient_public_key: value
                .recipient_public_key
                .map(|recipient_public_key| recipient_public_key.to_vec()),
        }
    } else {
        quote! {}
    };

    quote! {
        #[derive(Clone)]
        #[#nats_micro::napi_derive::napi(object, js_name = "Header")]
        pub struct #header_name {
            pub name: String,
            pub value: String,
            #encrypted_field
        }

        impl From<#header_name> for #nats_micro::__napi::NapiClientHeaderValue {
            fn from(value: #header_name) -> Self {
                Self {
                    name: value.name,
                    value: value.value,
                    #encrypted_conversion
                }
            }
        }

        #[derive(Default)]
        #[#nats_micro::napi_derive::napi(object)]
        pub struct #auth_options_name {
            pub token: Option<String>,
            pub username: Option<String>,
            pub password: Option<String>,
            pub nkey: Option<String>,
        }

        impl From<#auth_options_name> for #nats_micro::__napi::NapiAuthOptions {
            fn from(value: #auth_options_name) -> Self {
                Self {
                    token: value.token,
                    username: value.username,
                    password: value.password,
                    nkey: value.nkey,
                }
            }
        }

        #[derive(Default)]
        #[#nats_micro::napi_derive::napi(object)]
        pub struct #connect_options_name {
            pub name: Option<String>,
            pub no_echo: Option<bool>,
            pub max_reconnects: Option<u32>,
            pub connection_timeout_ms: Option<u32>,
            pub auth: Option<#auth_options_name>,
            pub tls_required: Option<bool>,
            pub tls_first: Option<bool>,
            pub certificates: Option<Vec<String>>,
            pub client_cert: Option<String>,
            pub client_key: Option<String>,
            pub ping_interval_ms: Option<u32>,
            pub subscription_capacity: Option<u32>,
            pub sender_capacity: Option<u32>,
            pub inbox_prefix: Option<String>,
            pub request_timeout_ms: Option<u32>,
            pub retry_on_initial_connect: Option<bool>,
            pub ignore_discovered_servers: Option<bool>,
            pub retain_servers_order: Option<bool>,
            pub read_buffer_capacity: Option<u16>,
            pub subject_prefix: Option<String>,
            pub headers: Option<Vec<#header_name>>,
            #recipient_field
        }

        impl From<#connect_options_name> for #nats_micro::__napi::NapiConnectOptions {
            fn from(value: #connect_options_name) -> Self {
                Self {
                    name: value.name,
                    no_echo: value.no_echo,
                    max_reconnects: value.max_reconnects,
                    connection_timeout_ms: value.connection_timeout_ms,
                    auth: value.auth.map(Into::into),
                    tls_required: value.tls_required,
                    tls_first: value.tls_first,
                    certificates: value.certificates,
                    client_cert: value.client_cert,
                    client_key: value.client_key,
                    ping_interval_ms: value.ping_interval_ms,
                    subscription_capacity: value.subscription_capacity,
                    sender_capacity: value.sender_capacity,
                    inbox_prefix: value.inbox_prefix,
                    request_timeout_ms: value.request_timeout_ms,
                    retry_on_initial_connect: value.retry_on_initial_connect,
                    ignore_discovered_servers: value.ignore_discovered_servers,
                    retain_servers_order: value.retain_servers_order,
                    read_buffer_capacity: value.read_buffer_capacity,
                    subject_prefix: value.subject_prefix,
                    #recipient_conversion
                }
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn render_method(
    service_name: &str,
    endpoint: &ClientEndpointSpec,
    nats_micro: &syn::Path,
    build_call_options_fn: &syn::Ident,
    map_service_error_fn: &syn::Ident,
    map_generic_error_fn: &syn::Ident,
    map_connect_error_fn: &syn::Ident,
) -> (Option<TokenStream>, TokenStream, TokenStream) {
    let attrs = &endpoint.attrs;
    let fn_ident = &endpoint.fn_name;
    let fn_with_ident = format_ident!("{}_with", fn_ident);
    let fn_with_headers_ident = format_ident!("{}_with_headers", fn_ident);
    let header_name = header_type_name(service_name);
    let js_name = fn_ident.to_string().to_lower_camel_case();
    let error_type = &endpoint.error_type;

    let map_client_error_fn = if napi_service_error_spec(error_type).is_some() {
        map_service_error_fn
    } else {
        map_generic_error_fn
    };

    let return_type = response_return_type(&endpoint.response, nats_micro);
    let response_map = response_map_expr(&endpoint.response, quote! { result }, nats_micro);

    let payload = endpoint.payload.as_ref();
    let has_subject_params = !endpoint.subject.params.is_empty();
    let has_inputs = has_subject_params || payload.is_some();

    let (helper_struct, method_args, public_forward_args, inner_forward_args) =
        if has_subject_params {
            let mut fields: Vec<(syn::Ident, TokenStream)> =
                Vec::with_capacity(endpoint.subject.params.len() + usize::from(payload.is_some()));

            fields.extend(endpoint.subject.params.iter().map(
                |SubjectParamMeta { name, inner_type }| {
                    (format_ident!("{}", name), quote! { #inner_type })
                },
            ));

            if let Some(payload) = payload {
                fields.push((
                    format_ident!("payload"),
                    payload_arg_type(payload, nats_micro),
                ));
            }

            let (args_name, args_tokens) =
                gen_args_struct(service_name, fn_ident, attrs, &fields, nats_micro);

            let subject_forward = endpoint.subject.params.iter().map(|param| {
                let name = format_ident!("{}", param.name);
                quote! { &args.#name }
            });

            let forward_args = if let Some(payload) = payload {
                let payload_binding = quote! { args.payload };
                let payload_forward = payload_forward_expr(payload, &payload_binding);
                quote! { #(#subject_forward,)* #payload_forward }
            } else {
                quote! { #(#subject_forward),* }
            };

            (
                Some(args_tokens),
                quote! { args: #args_name },
                quote! { args },
                forward_args,
            )
        } else if let Some(payload) = payload {
            let payload_ty = payload_arg_type(payload, nats_micro);
            let payload_binding = quote! { payload };
            let payload_forward = payload_forward_expr(payload, &payload_binding);

            (
                None,
                quote! { payload: #payload_ty },
                quote! { payload },
                quote! { #payload_forward },
            )
        } else {
            (None, quote! {}, quote! {}, quote! {})
        };

    let with_headers_method_args = if has_inputs {
        quote! { #method_args, headers: Option<Vec<#header_name>> }
    } else {
        quote! { headers: Option<Vec<#header_name>> }
    };

    let wrapper_forward_args = if has_inputs {
        quote! { #public_forward_args, None }
    } else {
        quote! { None }
    };

    let rust_call = if has_inputs {
        quote! { self.inner.#fn_with_ident(#inner_forward_args, options).await }
    } else {
        quote! { self.inner.#fn_with_ident(options).await }
    };

    let js_call = if has_inputs {
        quote! { inner.#fn_with_ident(#inner_forward_args, options).await }
    } else {
        quote! { inner.#fn_with_ident(options).await }
    };

    let rust_build_call_options = if cfg!(feature = "macros_encryption_feature") {
        quote! { #build_call_options_fn(&self.default_headers, headers, self.has_default_recipient) }
    } else {
        quote! { #build_call_options_fn(&self.default_headers, headers) }
    };

    let js_has_default_recipient = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            let has_default_recipient = self.has_default_recipient;
        }
    } else {
        quote! {}
    };

    let js_build_call_options = if cfg!(feature = "macros_encryption_feature") {
        quote! { #build_call_options_fn(&default_headers, headers, has_default_recipient) }
    } else {
        quote! { #build_call_options_fn(&default_headers, headers) }
    };

    let exported_fn_ident = format_ident!("__napi_{}", fn_ident);

    let rust_method = quote! {
        #(#attrs)*
        pub async fn #fn_with_headers_ident(
            &self,
            #with_headers_method_args
        ) -> #nats_micro::napi::Result<#return_type, String> {
            let options = #rust_build_call_options
                .map_err(|error| #map_connect_error_fn(error).into_rust_napi_error())?;
            let result = #rust_call
                .map_err(|err| #map_client_error_fn::<#error_type>(err).into_rust_napi_error())?;
            Ok(#response_map)
        }

        #(#attrs)*
        pub async fn #fn_ident(
            &self,
            #method_args
        ) -> #nats_micro::napi::Result<#return_type, String> {
            self.#fn_with_headers_ident(#wrapper_forward_args).await
        }
    };

    let js_method = quote! {
        #(#attrs)*
        #[#nats_micro::napi_derive::napi(js_name = #js_name)]
        pub fn #exported_fn_ident(
            &self,
            env: &Env,
            #with_headers_method_args
        ) -> #nats_micro::napi::Result<
            #nats_micro::napi::bindgen_prelude::PromiseRaw<'static, #return_type>
        > {
            let inner = self.inner.clone();
            let default_headers = self.default_headers.clone();
            #js_has_default_recipient

            let promise = env.spawn_future_with_callback(
                async move {
                    let result = match #js_build_call_options {
                        Ok(options) => #js_call.map_err(#map_client_error_fn::<#error_type>),
                        Err(error) => Err(#map_connect_error_fn(error)),
                    };

                    Ok(result)
                },
                move |env, result| match result {
                    Ok(value) => {
                        let result = value;
                        Ok(#response_map)
                    }
                    Err(error) => Err(error.into_napi_error(*env)),
                },
            )?;

            Ok(unsafe {
                ::std::mem::transmute::<
                    #nats_micro::napi::bindgen_prelude::PromiseRaw<'_, #return_type>,
                    #nats_micro::napi::bindgen_prelude::PromiseRaw<'static, #return_type>,
                >(promise)
            })
        }
    };

    (helper_struct, rust_method, js_method)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn generate_client_napi_module(
    _struct_ident: &syn::Ident,
    service_name: &str,
    endpoints: &[ClientEndpointSpec],
) -> TokenStream {
    let client_base_name = client_error_base_name(service_name);
    let header_name = header_type_name(service_name);
    let nats_micro = nats_micro_path();
    let rust_client_module = format_ident!("{}_client", service_name.to_snake_case());
    let rust_client_struct = format_ident!("{}Client", service_name.to_upper_camel_case());
    let wrapper_client_struct = format_ident!("Js{}Client", service_name.to_upper_camel_case());
    let js_client_name = format!("{}Client", service_name.to_upper_camel_case());
    let map_service_error_fn =
        format_ident!("__{}_service_napi_error", service_name.to_snake_case());
    let map_generic_error_fn =
        format_ident!("__{}_generic_napi_error", service_name.to_snake_case());
    let map_connect_error_fn =
        format_ident!("__{}_connect_napi_error", service_name.to_snake_case());
    let build_call_options_fn =
        format_ident!("__{}_build_call_options", service_name.to_snake_case());
    let framework_and_transport_error_items =
        render_framework_and_transport_error_items(service_name, &nats_micro);
    let service_error_specs = collect_napi_service_error_specs(endpoints);
    let service_error_asserts: Vec<_> = service_error_specs
        .iter()
        .map(|spec| gen_napi_service_error_assert(&spec.error_type))
        .collect();
    let service_error_interface =
        render_service_error_interface(service_name, &service_error_specs, &nats_micro);
    let connect_support = render_connect_support(service_name, &nats_micro);
    let mut helper_structs = Vec::new();
    let mut rust_methods = Vec::new();
    let mut js_methods = Vec::new();

    for endpoint in endpoints {
        let (helper_struct, rust_method, js_method) = render_method(
            service_name,
            endpoint,
            &nats_micro,
            &build_call_options_fn,
            &map_service_error_fn,
            &map_generic_error_fn,
            &map_connect_error_fn,
        );
        if let Some(helper_struct) = helper_struct {
            helper_structs.push(helper_struct);
        }
        rust_methods.push(rust_method);
        js_methods.push(js_method);
    }

    let connect_inner = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            let options = options.unwrap_or_default();
            let has_default_recipient = options.recipient_public_key.is_some();
            let default_headers = options
                .headers
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect::<Vec<#nats_micro::__napi::NapiClientHeaderValue>>();
            let _ = #nats_micro::__napi::client_call_options_from_headers(
                default_headers.clone(),
                has_default_recipient,
            )?;

            let #nats_micro::__napi::ConnectedClient {
                client,
                subject_prefix,
                recipient,
            } = #nats_micro::__napi::connect(server, options.into()).await?;

            let recipient_public_key = recipient.as_ref().map(#nats_micro::ServiceRecipient::to_bytes);
            let inner = match subject_prefix {
                Some(subject_prefix) => {
                    #rust_client_module::#rust_client_struct::with_prefix(
                        client,
                        subject_prefix,
                        recipient_public_key,
                    )
                }
                None => #rust_client_module::#rust_client_struct::new(
                    client,
                    recipient_public_key,
                ),
            };

            Ok(Self {
                inner,
                default_headers,
                has_default_recipient,
            })
        }
    } else {
        quote! {
            let options = options.unwrap_or_default();
            let default_headers = options
                .headers
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect::<Vec<#nats_micro::__napi::NapiClientHeaderValue>>();
            let _ = #nats_micro::__napi::client_call_options_from_headers(default_headers.clone(), false)?;

            let #nats_micro::__napi::ConnectedClient {
                client,
                subject_prefix,
            } = #nats_micro::__napi::connect(server, options.into()).await?;

            let inner = match subject_prefix {
                Some(subject_prefix) => {
                    #rust_client_module::#rust_client_struct::with_prefix(client, subject_prefix)
                }
                None => #rust_client_module::#rust_client_struct::new(client),
            };

            Ok(Self {
                inner,
                default_headers,
            })
        }
    };

    let (_, connect_options_name) = connect_options_names(service_name);

    let build_call_options_args = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            default_headers: &[#nats_micro::__napi::NapiClientHeaderValue],
            headers: Option<Vec<#header_name>>,
            has_default_recipient: bool,
        }
    } else {
        quote! {
            default_headers: &[#nats_micro::__napi::NapiClientHeaderValue],
            headers: Option<Vec<#header_name>>,
        }
    };

    let build_call_options_result = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            #nats_micro::__napi::client_call_options_from_headers(merged, has_default_recipient)
        }
    } else {
        quote! {
            #nats_micro::__napi::client_call_options_from_headers(merged, false)
        }
    };

    let wrapper_recipient_field = if cfg!(feature = "macros_encryption_feature") {
        quote! {
            has_default_recipient: bool,
        }
    } else {
        quote! {}
    };

    let framework_error_attrs = {
        let framework_enum_ident = format_ident!("{}FrameworkError", client_base_name);

        let return_type = format!("err is {framework_enum_ident}");
        let return_type_lit = LitStr::new(&return_type, Span::call_site());

        quote! {
            #[#nats_micro::napi_derive::napi(js_name = "isFrameworkError", ts_return_type = #return_type_lit)]
        }
    };

    let transport_error_attrs = {
        let transport_enum_ident = format_ident!("{}TransportError", client_base_name);

        let return_type = format!("err is {transport_enum_ident}");
        let return_type_lit = LitStr::new(&return_type, Span::call_site());

        quote! {
            #[#nats_micro::napi_derive::napi(
                js_name = "isTransportError",
                ts_return_type = #return_type_lit
            )]
        }
    };

    let service_error_attrs = {
        let service_enum_ident = format_ident!("{}Error", client_base_name);

        let return_type = format!("err is {service_enum_ident}");
        let return_type_lit = LitStr::new(&return_type, Span::call_site());

        quote! {
            #[#nats_micro::napi_derive::napi(js_name = "isServiceError", ts_return_type = #return_type_lit)]
        }
    };

    quote! {
        use #nats_micro::napi::Env;

        #connect_support
        #framework_and_transport_error_items
        #(#service_error_asserts)*
        #service_error_interface
        #(#helper_structs)*

        fn #build_call_options_fn(
            #build_call_options_args
        ) -> ::std::result::Result<#nats_micro::ClientCallOptions, #nats_micro::NatsErrorResponse> {
            let mut merged = default_headers.to_vec();

            if let Some(headers) = headers {
                merged.extend(headers.into_iter().map(Into::into));
            }

            #build_call_options_result
        }

        fn #map_service_error_fn<E>(
            err: #nats_micro::ClientError<E>
        ) -> #nats_micro::__napi::NapiClientError
        where
            E: #nats_micro::__private::NapiServiceError
                + ::std::fmt::Debug
                + ::std::fmt::Display
                + 'static,
        {
            match err {
                #nats_micro::ClientError::Service { response, .. } => {
                    #nats_micro::__napi::NapiClientError::service(response)
                }
                #nats_micro::ClientError::ServiceResponse(response) => {
                    #nats_micro::__napi::NapiClientError::from_response(response)
                }
                #nats_micro::ClientError::Transport(error) => {
                    #nats_micro::__napi::NapiClientError::from_response(
                        error.as_nats_error_response().clone()
                    )
                }
            }
        }

        fn #map_generic_error_fn<E>(
            err: #nats_micro::ClientError<E>
        ) -> #nats_micro::__napi::NapiClientError
        where
            E: ::std::fmt::Debug + ::std::fmt::Display + 'static,
        {
            #nats_micro::__napi::NapiClientError::from_response(err.into_nats_error_response())
        }

        fn #map_connect_error_fn(
            err: #nats_micro::NatsErrorResponse
        ) -> #nats_micro::__napi::NapiClientError {
            #nats_micro::__napi::NapiClientError::from_response(err)
        }

        #[#nats_micro::napi_derive::napi(js_name = #js_client_name)]
        pub struct #wrapper_client_struct {
            inner: #rust_client_module::#rust_client_struct,
            default_headers: Vec<#nats_micro::__napi::NapiClientHeaderValue>,
            #wrapper_recipient_field
        }

        impl #wrapper_client_struct {
            async fn connect_inner(
                server: String,
                options: Option<#connect_options_name>,
            ) -> ::std::result::Result<Self, #nats_micro::NatsErrorResponse> {
                #connect_inner
            }

            pub fn set_headers(
                &mut self,
                headers: Option<Vec<#header_name>>,
            ) {
                self.default_headers = headers
                    .unwrap_or_default()
                    .into_iter()
                    .map(Into::into)
                    .collect();
            }

            pub async fn connect(
                server: String,
                options: Option<#connect_options_name>,
            ) -> #nats_micro::napi::Result<Self, String> {
                Self::connect_inner(server, options)
                    .await
                    .map_err(|error| #map_connect_error_fn(error).into_rust_napi_error())
            }

            #(#rust_methods)*
        }

        #[#nats_micro::napi_derive::napi]
        impl #wrapper_client_struct {
            #[#nats_micro::napi_derive::napi(js_name = "connect")]
            pub fn __napi_connect(
                env: &Env,
                server: String,
                options: Option<#connect_options_name>,
            ) -> #nats_micro::napi::Result<
                #nats_micro::napi::bindgen_prelude::PromiseRaw<'static, #wrapper_client_struct>
            > {
                let promise = env.spawn_future_with_callback(
                    async move {
                        Ok(
                            Self::connect_inner(server, options)
                                .await
                                .map_err(#map_connect_error_fn)
                        )
                    },
                    move |env, result| match result {
                        Ok(value) => Ok(value),
                        Err(error) => Err(error.into_napi_error(*env)),
                    },
                )?;

                Ok(unsafe {
                    ::std::mem::transmute::<
                        #nats_micro::napi::bindgen_prelude::PromiseRaw<'_, #wrapper_client_struct>,
                        #nats_micro::napi::bindgen_prelude::PromiseRaw<'static, #wrapper_client_struct>,
                    >(promise)
                })
            }

            #[#nats_micro::napi_derive::napi(js_name = "setDefaultHeaders")]
            pub fn __napi_set_headers(&mut self, headers: Option<Vec<#header_name>>) {
                self.set_headers(headers);
            }

            #framework_error_attrs
            pub fn __napi_is_framework_error(err: #nats_micro::serde_json::Value) -> bool {
                if let Some(is_framework_error) = err.get("isFrameworkError") {
                    return is_framework_error.as_bool() == Some(true);
                }

                false
            }

            #transport_error_attrs
            pub fn __napi_is_transport_error(err: #nats_micro::serde_json::Value) -> bool {
                  if let Some(is_transport_error) = err.get("isTransportError") {
                      return is_transport_error.as_bool() == Some(true);
                  }

                  false
            }

            #service_error_attrs
            pub fn __napi_is_service_error(err: #nats_micro::serde_json::Value) -> bool {
                if let Some(is_service_error) = err.get("isServiceError") {
                    return is_service_error.as_bool() == Some(true);
                }

                false
            }

            #(#js_methods)*
        }
    }
}

#[cfg(test)]
#[path = "tests/napi_tests.rs"]
mod tests;
