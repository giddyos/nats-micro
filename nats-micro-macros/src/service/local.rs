use proc_macro2::TokenStream;
use quote::quote;

use super::{ArgumentKind, MethodModel, OperationKind, ServiceModel, metadata};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let service = &model.service_ident;
    let state = &model.state_type;
    let routes: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind == OperationKind::Request)
        .map(|operation| {
            let index = operation.operation_index.expect("operation index");
            let endpoint = metadata::operation_type(model, operation);
            if operation_is_encrypted(operation) {
                quote! {
                    #index => Err(#nats_micro::ErrorReply::framework(
                        #nats_micro::FrameworkError::NoEncryptionKey,
                        "encrypted local dispatch requires TestApp with an encryption key",
                        request.request_id().existing(),
                    )),
                }
            } else {
                quote! {
                    #index => <#endpoint as #nats_micro::RequestEndpoint<#state>>
                        ::call(state, request).await,
                }
            }
        })
        .collect();
    quote! {
        impl #nats_micro::LocalService<#state> for #service {
            async fn dispatch_local<'__request>(
                state: &'__request #state,
                operation: usize,
                request: #nats_micro::Request<'__request>,
            ) -> #nats_micro::DispatchResult {
                match operation {
                    #(#routes)*
                    _ => Err(#nats_micro::ErrorReply::framework(
                        #nats_micro::FrameworkError::InvalidResponse,
                        "operation is not a local request route",
                        request.request_id().existing(),
                    )),
                }
            }
        }
    }
}

fn operation_is_encrypted(operation: &super::OperationModel) -> bool {
    operation.response.encrypted
        || operation.arguments.iter().any(|argument| {
            matches!(&argument.kind, ArgumentKind::Payload(payload) if payload.encrypted)
        })
}

pub(crate) fn generate_service_method(model: &ServiceModel) -> TokenStream {
    if !cfg!(feature = "macros_test_util_feature") {
        return TokenStream::new();
    }

    let nats_micro = nats_micro_path();
    let state = &model.state_type;
    let routes = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind == OperationKind::Request)
        .map(|operation| {
            let subject = &operation.subject.pattern;
            let endpoint = metadata::operation_type(model, operation);
            if operation_is_encrypted(operation) {
                quote! {
                    if #nats_micro::subject_matches(#subject, request.subject()) {
                        return #nats_micro::testing::dispatch_encrypted::<#state, #endpoint>(
                            state,
                            keypair,
                            request,
                        )
                        .await;
                    }
                }
            } else {
                quote! {
                    if #nats_micro::subject_matches(#subject, request.subject()) {
                        return #nats_micro::testing::dispatch::<#state, #endpoint>(
                            state,
                            request,
                        )
                        .await;
                    }
                }
            }
        });
    let encryption_parameter = cfg!(feature = "macros_encryption_feature")
        .then(|| quote!(keypair: Option<&'__request #nats_micro::ServiceKeyPair>,));

    quote! {
        fn dispatch_local<'__request>(
            self,
            state: &'__request #state,
            #encryption_parameter
            request: #nats_micro::testing::LocalRequest,
        ) -> impl ::std::future::Future<
            Output = #nats_micro::testing::LocalDispatch
        > + Send + '__request {
            async move {
                #(#routes)*
                #nats_micro::testing::LocalDispatch::NotMatched(request)
            }
        }
    }
}

pub(crate) fn generate_consumer_service_method(model: &ServiceModel) -> TokenStream {
    if !cfg!(feature = "macros_test_jetstream_feature") {
        return TokenStream::new();
    }

    let nats_micro = nats_micro_path();
    let state = &model.state_type;
    let routes = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind == OperationKind::Consumer)
        .map(|operation| {
            let durable = operation.options.durable.as_deref().unwrap_or_default();
            let consumer = metadata::operation_type(model, operation);
            quote! {
                if durable == #durable {
                    return #nats_micro::testing::dispatch_consumer::<#state, #consumer>(
                        state,
                        request,
                    )
                    .await;
                }
            }
        });

    quote! {
        fn dispatch_consumer_local<'__request>(
            self,
            state: &'__request #state,
            durable: &'__request str,
            request: #nats_micro::testing::LocalRequest,
        ) -> impl ::std::future::Future<
            Output = #nats_micro::testing::LocalConsumerDispatch
        > + Send + '__request {
            async move {
                #(#routes)*
                #nats_micro::testing::LocalConsumerDispatch::NotMatched(request)
            }
        }
    }
}
