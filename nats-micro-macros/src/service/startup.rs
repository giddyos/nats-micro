use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{MethodModel, OperationKind, ServiceModel, metadata};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let service_ident = &model.service_ident;
    let state_type = &model.state_type;
    let requests: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind == OperationKind::Request)
        .collect();

    let registrations = requests.iter().enumerate().map(|(index, operation)| {
        let endpoint = format_ident!("__endpoint_{index}");
        let operation_type = metadata::operation_type(model, operation);
        quote! {
            let mut __builder = __service
                .endpoint_builder()
                .name(<#operation_type as #nats_micro::RequestEndpoint<#state_type>>::SPEC.rust_name);
            if let Some(queue) =
                <#operation_type as #nats_micro::RequestEndpoint<#state_type>>::SPEC.queue_group
            {
                __builder = __builder.queue_group(queue);
            }
            let #endpoint = __builder
                .add(<#operation_type as #nats_micro::RequestEndpoint<#state_type>>::SPEC.subject)
                .await
                .map_err(|error| #nats_micro::anyhow::anyhow!(error.to_string()))?;
        }
    });
    let workers: Vec<_> = requests
        .iter()
        .enumerate()
        .map(|(index, operation)| {
            let endpoint = format_ident!("__endpoint_{index}");
            let operation_type = metadata::operation_type(model, operation);
            quote! {
                #nats_micro::runtime::run_request_endpoint::<#state_type, #operation_type>(
                    ::std::sync::Arc::clone(&state),
                    #endpoint,
                    <#operation_type as #nats_micro::RequestEndpoint<#state_type>>::SPEC.concurrency,
                    shutdown.clone(),
                )
            }
        })
        .collect();

    let drive = if workers.is_empty() {
        quote! {
            let mut shutdown = shutdown;
            while !shutdown.borrow().is_requested() && shutdown.changed().await.is_ok() {}
        }
    } else {
        quote! {
            #nats_micro::tokio::try_join!(#(#workers),*)?;
        }
    };

    quote! {
        impl #nats_micro::RunnableService<#state_type> for #service_ident {
            fn run_requests(
                self,
                state: ::std::sync::Arc<#state_type>,
                client: #nats_micro::NatsClient,
                shutdown: #nats_micro::tokio::sync::watch::Receiver<#nats_micro::ShutdownState>,
            ) -> impl ::std::future::Future<Output = #nats_micro::anyhow::Result<()>> + Send {
                async move {
                    use #nats_micro::async_nats::service::ServiceExt as _;

                    let __service = client
                        .service_builder()
                        .description(Self::SPEC.description)
                        .start(Self::SPEC.name, Self::SPEC.version)
                        .await
                        .map_err(|error| #nats_micro::anyhow::anyhow!(error.to_string()))?;
                    #(#registrations)*
                    #drive
                    drop(__service);
                    Ok(())
                }
            }
        }
    }
}
