use heck::ToLowerCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Type, spanned::Spanned};

use super::{
    ArgumentKind, MethodModel, OperationKind, OperationModel, ServiceModel, WireCodec, Wrapper,
    classify, dispatch,
};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    if !model.args.napi {
        return TokenStream::new();
    }

    let nats_micro = nats_micro_path();
    let service = &model.service_ident;
    let rust_client = format_ident!("{}Client", service);
    let napi_client = format_ident!("Js{}Client", service);
    let js_client_name = format!("{service}Client");
    let error_mapper = format_ident!(
        "__{}_napi_client_error",
        heck::ToSnakeCase::to_snake_case(service.to_string().as_str())
    );
    let operations: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| {
            matches!(
                operation.kind,
                OperationKind::Request | OperationKind::Publish
            )
        })
        .map(|operation| operation_method(operation, &error_mapper))
        .collect();
    let assertions: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .flat_map(napi_assertions)
        .collect();
    let encrypted = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .any(operation_is_encrypted);
    let connect_parameters = encrypted.then(
        || quote!(, recipient_public_key: Option<#nats_micro::napi::bindgen_prelude::Buffer>),
    );
    let configure_recipient = encrypted.then(|| {
        quote! {
            let inner = if let Some(recipient_public_key) = recipient_public_key {
                let actual = recipient_public_key.len();
                let public_key: [u8; 32] = recipient_public_key
                    .to_vec()
                    .try_into()
                    .map_err(|_| #nats_micro::napi::Error::from_reason(format!(
                        "MISSING_RECIPIENT_PUBKEY: expected 32 bytes, received {actual}"
                    )))?;
                inner.with_recipient(#nats_micro::ServiceRecipient::from_bytes(public_key))
            } else {
                inner
            };
        }
    });

    quote! {
        #(#assertions)*

        fn #error_mapper<E>(
            error: #nats_micro::ClientError<E>,
        ) -> #nats_micro::napi::Error
        where
            E: #nats_micro::FromNatsErrorResponse
                + ::std::fmt::Debug
                + ::std::fmt::Display
                + 'static,
        {
            let response = error
                .as_nats_error_response()
                .cloned()
                .unwrap_or_else(|| #nats_micro::NatsErrorResponse::internal(
                    "NAPI_CLIENT_ERROR",
                    error.to_string(),
                ));
            let reason = #nats_micro::serde_json::to_string(&response)
                .unwrap_or_else(|_| response.to_string());
            #nats_micro::napi::Error::from_reason(reason)
        }

        #[#nats_micro::napi_derive::napi(js_name = #js_client_name)]
        pub struct #napi_client {
            inner: #rust_client<#nats_micro::NatsTransport>,
        }

        #[#nats_micro::napi_derive::napi]
        impl #napi_client {
            #[#nats_micro::napi_derive::napi(factory)]
            pub async fn connect(
                server: String
                #connect_parameters
            ) -> #nats_micro::napi::Result<Self> {
                let client = #nats_micro::async_nats::connect(server)
                    .await
                    .map_err(|error| #nats_micro::napi::Error::from_reason(format!(
                        "CLIENT_CONNECT_FAILED: {error}"
                    )))?;
                let inner = #rust_client::new(#nats_micro::NatsTransport::new(client));
                #configure_recipient
                Ok(Self { inner })
            }

            #(#operations)*
        }
    }
}

fn operation_method(operation: &OperationModel, error_mapper: &syn::Ident) -> TokenStream {
    let nats_micro = nats_micro_path();
    let method = &operation.method.sig.ident;
    let js_name = method.to_string().to_lower_camel_case();
    let attrs = dispatch::conditional_attrs(&operation.method);
    let parameters: Vec<_> = operation
        .arguments
        .iter()
        .filter_map(napi_parameter)
        .collect();
    let declarations = parameters.iter().map(|parameter| {
        let ident = &parameter.ident;
        let ty = &parameter.napi_type;
        quote!(#ident: #ty)
    });
    let forwards = parameters.iter().map(|parameter| &parameter.forward);
    let response = napi_response_type(operation);
    let map_response = napi_response_map(operation);

    match operation.kind {
        OperationKind::Request => quote! {
            #(#attrs)*
            #[#nats_micro::napi_derive::napi(js_name = #js_name)]
            pub async fn #method(
                &self,
                #(#declarations),*
            ) -> #nats_micro::napi::Result<#response> {
                let response = self.inner.#method(#(#forwards),*)
                    .await
                    .map_err(#error_mapper)?;
                Ok(#map_response)
            }
        },
        OperationKind::Publish => quote! {
            #(#attrs)*
            #[#nats_micro::napi_derive::napi(js_name = #js_name)]
            pub async fn #method(
                &self,
                #(#declarations),*
            ) -> #nats_micro::napi::Result<()> {
                self.inner.#method(#(#forwards),*)
                    .await
                    .map_err(#error_mapper)
            }
        },
        OperationKind::Subscribe | OperationKind::Consumer => TokenStream::new(),
    }
}

struct NapiParameter {
    ident: syn::Ident,
    napi_type: Type,
    forward: TokenStream,
}

fn napi_parameter(argument: &super::ArgumentModel) -> Option<NapiParameter> {
    let nats_micro = nats_micro_path();
    let ident = argument.ident.clone();
    match &argument.kind {
        ArgumentKind::Subject { .. } => {
            let (napi_type, forward) = if matches!(&argument.ty, Type::Reference(reference) if classify::last_ident(&reference.elem).as_deref() == Some("str"))
            {
                (syn::parse_quote!(String), quote!(&#ident))
            } else {
                (argument.ty.clone(), quote!(#ident))
            };
            Some(NapiParameter {
                ident,
                napi_type,
                forward,
            })
        }
        ArgumentKind::Payload(payload) => {
            let value = payload.decoded_type.as_ref().unwrap_or(&argument.ty);
            let napi_type: Type = match payload.wrapper {
                Wrapper::Body => {
                    syn::parse_quote!(#nats_micro::napi::bindgen_prelude::Buffer)
                }
                Wrapper::Text => syn::parse_quote!(String),
                Wrapper::Json | Wrapper::Proto | Wrapper::Direct => value.clone(),
                Wrapper::Response => argument.ty.clone(),
            };
            let forward = match (payload.wrapper, payload.optional) {
                (Wrapper::Body | Wrapper::Text, true) => quote!(#ident.as_deref()),
                (Wrapper::Json | Wrapper::Proto | Wrapper::Direct, true) => quote!(#ident.as_ref()),
                (
                    Wrapper::Body
                    | Wrapper::Text
                    | Wrapper::Json
                    | Wrapper::Proto
                    | Wrapper::Direct,
                    false,
                ) => quote!(&#ident),
                (Wrapper::Response, _) => quote!(#ident),
            };
            let napi_type = if payload.optional {
                syn::parse_quote!(Option<#napi_type>)
            } else {
                napi_type
            };
            Some(NapiParameter {
                ident,
                napi_type,
                forward,
            })
        }
        ArgumentKind::State
        | ArgumentKind::StateRef(_)
        | ArgumentKind::Header { .. }
        | ArgumentKind::Headers
        | ArgumentKind::RequestMeta
        | ArgumentKind::Auth { .. } => None,
    }
}

fn napi_response_type(operation: &OperationModel) -> Type {
    let nats_micro = nats_micro_path();
    let response = &operation.response;
    let value: Type = match response.codec {
        WireCodec::Raw
            if matches!(
                classify::last_ident(&response.wire_type).as_deref(),
                Some("Bytes" | "Vec")
            ) =>
        {
            syn::parse_quote!(#nats_micro::napi::bindgen_prelude::Buffer)
        }
        _ => response.wire_type.clone(),
    };
    if response.optional {
        syn::parse_quote!(Option<#value>)
    } else {
        value
    }
}

fn napi_response_map(operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let response = &operation.response;
    if response.codec == WireCodec::Raw
        && matches!(
            classify::last_ident(&response.wire_type).as_deref(),
            Some("Bytes" | "Vec")
        )
    {
        if response.optional {
            quote!(response.map(#nats_micro::napi::bindgen_prelude::Buffer::from))
        } else {
            quote!(#nats_micro::napi::bindgen_prelude::Buffer::from(response))
        }
    } else {
        quote!(response)
    }
}

fn napi_assertions(operation: &OperationModel) -> Vec<TokenStream> {
    if !matches!(
        operation.kind,
        OperationKind::Request | OperationKind::Publish
    ) {
        return Vec::new();
    }
    let nats_micro = nats_micro_path();
    let attrs = dispatch::conditional_attrs(&operation.method);
    let mut assertions = Vec::new();
    for argument in &operation.arguments {
        let ArgumentKind::Payload(payload) = &argument.kind else {
            continue;
        };
        if matches!(payload.wrapper, Wrapper::Json | Wrapper::Proto)
            || (payload.wrapper == Wrapper::Direct
                && matches!(payload.codec, WireCodec::Json | WireCodec::Protobuf))
        {
            let ty = payload.decoded_type.as_ref().unwrap_or(&argument.ty);
            assertions.push(quote_spanned! {ty.span()=>
                #(#attrs)*
                const _: fn() = || {
                    #nats_micro::__private::assert_napi_object::<#ty>();
                };
            });
        }
    }
    if matches!(
        operation.response.codec,
        WireCodec::Json | WireCodec::Protobuf
    ) && !classify::is_unit(&operation.response.wire_type)
    {
        let ty = &operation.response.wire_type;
        assertions.push(quote_spanned! {ty.span()=>
            #(#attrs)*
            const _: fn() = || {
                #nats_micro::__private::assert_napi_object::<#ty>();
            };
        });
    }
    assertions
}

fn operation_is_encrypted(operation: &OperationModel) -> bool {
    operation.response.encrypted
        || operation.arguments.iter().any(|argument| {
            matches!(&argument.kind, ArgumentKind::Payload(payload) if payload.encrypted)
        })
}
