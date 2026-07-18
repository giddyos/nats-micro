use heck::{ToShoutySnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{
    ArgumentKind, AuthIntent, MethodModel, OperationKind, OperationModel, ServiceModel, WireCodec,
    classify,
};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let service_ident = &model.service_ident;
    let state_type = &model.state_type;
    let name = &model.args.name;
    let version = &model.args.version;
    let description = &model.args.description;
    let prefix = &model.args.prefix;
    let operations: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind != OperationKind::Consumer)
        .map(|operation| operation_spec(operation, name, version))
        .collect();
    let consumers: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind == OperationKind::Consumer)
        .map(|operation| consumer_spec(operation, name, version))
        .collect();
    let markers: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind != OperationKind::Consumer)
        .map(|operation| {
            let marker = format_ident!(
                "{}",
                operation
                    .method
                    .sig
                    .ident
                    .to_string()
                    .to_shouty_snake_case()
            );
            let index = operation.operation_index.expect("operation index");
            quote! {
                pub const #marker: #nats_micro::OperationMarker =
                    #nats_micro::OperationMarker {
                        index: #index,
                        spec: &Self::SPEC.operations[#index],
                    };
            }
        })
        .collect();

    quote! {
        impl #nats_micro::StaticService<#state_type> for #service_ident {
            const SPEC: #nats_micro::ServiceSpec = #nats_micro::ServiceSpec {
                name: #name,
                version: #version,
                description: #description,
                prefix: #prefix,
                operations: &[#(#operations),*],
                consumers: &[#(#consumers),*],
            };
        }

        impl #service_ident {
            pub const SPEC: #nats_micro::ServiceSpec =
                <Self as #nats_micro::StaticService<#state_type>>::SPEC;

            #(#markers)*
        }
    }
}

pub(crate) fn operation_type(model: &ServiceModel, operation: &OperationModel) -> syn::Ident {
    format_ident!(
        "__{}{}",
        model.service_ident,
        operation.method.sig.ident.to_string().to_upper_camel_case()
    )
}

pub(crate) fn operation_spec(
    operation: &OperationModel,
    service_name: &str,
    service_version: &str,
) -> TokenStream {
    let nats_micro = nats_micro_path();
    let rust_name = operation.method.sig.ident.to_string();
    let kind = match operation.kind {
        OperationKind::Request => quote!(#nats_micro::OperationKind::Request),
        OperationKind::Publish => quote!(#nats_micro::OperationKind::Publish),
        OperationKind::Subscribe => quote!(#nats_micro::OperationKind::Subscribe),
        OperationKind::Consumer => quote!(#nats_micro::OperationKind::Consumer),
    };
    let subject = &operation.subject.pattern;
    let template = &operation.subject.template;
    let queue = operation.options.queue.as_deref();
    let queue = if let Some(queue) = queue {
        quote!(Some(#queue))
    } else {
        quote!(None)
    };
    let request_codec = operation
        .arguments
        .iter()
        .find_map(|argument| match &argument.kind {
            ArgumentKind::Payload(payload) => Some(codec_tokens(payload.codec)),
            _ => None,
        })
        .unwrap_or_else(|| codec_tokens(WireCodec::Empty));
    let response_codec = codec_tokens(operation.response.codec);
    let request_encrypted = operation.arguments.iter().any(
        |argument| matches!(&argument.kind, ArgumentKind::Payload(payload) if payload.encrypted),
    );
    let response_encrypted = operation.response.encrypted;
    let request_type = operation
        .arguments
        .iter()
        .find_map(|argument| match &argument.kind {
            ArgumentKind::Payload(_) => Some(type_option(&argument.ty)),
            _ => None,
        })
        .unwrap_or_else(|| quote!(None));
    let response_type = if classify::is_unit(&operation.response.wire_type) {
        quote!(None)
    } else {
        type_option(&operation.response.wire_type)
    };
    let error_type = operation
        .response
        .error_type
        .as_ref()
        .map_or_else(|| quote!(None), type_option);
    let auth = auth_policy(operation);
    let concurrency = operation.options.concurrency.unwrap_or(256);
    let params: Vec<_> = operation
        .arguments
        .iter()
        .filter_map(|argument| {
            let ArgumentKind::Subject { name, segment } = &argument.kind else {
                return None;
            };
            let ty = &argument.ty;
            let rust_type = quote!(#ty).to_string();
            Some(quote! {
                #nats_micro::ParamSpec {
                    name: #name,
                    segment: #segment as u16,
                    rust_type: #rust_type,
                }
            })
        })
        .collect();

    quote! {
        #nats_micro::OperationSpec {
            service_name: #service_name,
            service_version: #service_version,
            rust_name: #rust_name,
            kind: #kind,
            subject: #subject,
            subject_template: #template,
            queue_group: #queue,
            request_codec: #request_codec,
            response_codec: #response_codec,
            request_encrypted: #request_encrypted,
            response_encrypted: #response_encrypted,
            request_type: #request_type,
            response_type: #response_type,
            error_type: #error_type,
            auth: #auth,
            concurrency: #concurrency,
            params: &[#(#params),*],
        }
    }
}

pub(crate) fn consumer_spec(
    operation: &OperationModel,
    service_name: &str,
    service_version: &str,
) -> TokenStream {
    let nats_micro = nats_micro_path();
    let rust_name = operation.method.sig.ident.to_string();
    let stream = operation.options.stream.as_deref().unwrap_or_default();
    let durable = operation.options.durable.as_deref().unwrap_or_default();
    let filter = &operation.subject.pattern;
    let concurrency = operation.options.concurrency.unwrap_or(256);
    let ack_wait = operation.options.ack_wait_ms.unwrap_or(30_000);
    let max_deliver = operation.options.max_deliver.unwrap_or(5);
    let backoff = &operation.options.backoff_ms;
    quote! {
        #nats_micro::ConsumerSpec {
            service_name: #service_name,
            service_version: #service_version,
            rust_name: #rust_name,
            stream: #stream,
            durable: #durable,
            filter_subject: #filter,
            concurrency: #concurrency,
            ack_wait_ms: #ack_wait,
            max_deliver: #max_deliver,
            backoff_ms: &[#(#backoff),*],
        }
    }
}

pub(crate) fn codec_tokens(codec: WireCodec) -> TokenStream {
    let nats_micro = nats_micro_path();
    match codec {
        WireCodec::Json => quote!(#nats_micro::Codec::Json),
        WireCodec::Protobuf => quote!(#nats_micro::Codec::Protobuf),
        WireCodec::Raw => quote!(#nats_micro::Codec::Raw),
        WireCodec::Utf8 => quote!(#nats_micro::Codec::Utf8),
        WireCodec::Empty => quote!(#nats_micro::Codec::Empty),
    }
}

fn auth_policy(operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let inferred = if operation.arguments.iter().any(|argument| {
        matches!(
            argument.kind,
            ArgumentKind::Auth {
                optional: false,
                ..
            }
        )
    }) {
        AuthIntent::Required
    } else if operation
        .arguments
        .iter()
        .any(|argument| matches!(argument.kind, ArgumentKind::Auth { optional: true, .. }))
    {
        AuthIntent::Optional
    } else {
        AuthIntent::None
    };
    match operation.options.auth.unwrap_or(inferred) {
        AuthIntent::None => quote!(#nats_micro::AuthPolicy::None),
        AuthIntent::Optional => quote!(#nats_micro::AuthPolicy::Optional),
        AuthIntent::Required => quote!(#nats_micro::AuthPolicy::Required),
    }
}

fn type_option(ty: &syn::Type) -> TokenStream {
    let value = quote!(#ty).to_string();
    quote!(Some(#value))
}
