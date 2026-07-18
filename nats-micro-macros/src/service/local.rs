use proc_macro2::TokenStream;
use quote::quote;

use super::{MethodModel, OperationKind, ServiceModel, metadata};
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
            quote! {
                #index => <#endpoint as #nats_micro::RequestEndpoint<#state>>
                    ::call(state, request).await,
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
            quote! {
                if #nats_micro::subject_matches(#subject, request.subject()) {
                    return #nats_micro::testing::dispatch::<#state, #endpoint>(
                        state,
                        request,
                    )
                    .await;
                }
            }
        });

    quote! {
        fn dispatch_local(
            self,
            state: &#state,
            request: #nats_micro::testing::LocalRequest,
        ) -> impl ::std::future::Future<
            Output = #nats_micro::testing::LocalDispatch
        > + Send + '_ {
            async move {
                #(#routes)*
                #nats_micro::testing::LocalDispatch::NotMatched(request)
            }
        }
    }
}
