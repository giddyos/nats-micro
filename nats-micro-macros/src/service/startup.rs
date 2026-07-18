use proc_macro2::TokenStream;
use quote::quote;

use super::{MethodModel, OperationKind, ServiceModel, metadata};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let service_ident = &model.service_ident;
    let state_type = &model.state_type;
    let starters = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter_map(|operation| {
            let operation_type = metadata::operation_type(model, operation);
            match operation.kind {
                OperationKind::Request => Some(quote! {
                    runtime.spawn_request::<#operation_type>().await?;
                }),
                OperationKind::Subscribe => Some(quote! {
                    runtime.spawn_subscription::<#operation_type>().await?;
                }),
                OperationKind::Consumer => Some(quote! {
                    runtime.spawn_consumer::<#operation_type>().await?;
                }),
                OperationKind::Publish => None,
            }
        })
        .collect::<Vec<_>>();
    let client = quote::format_ident!("{}Client", service_ident);
    let local_dispatch = super::local::generate_service_method(model);
    let local_consumer_dispatch = super::local::generate_consumer_service_method(model);

    quote! {
        impl #nats_micro::Service<#state_type> for #service_ident {
            const SPEC: #nats_micro::ServiceSpec =
                <Self as #nats_micro::StaticService<#state_type>>::SPEC;

            type Client<T>
                = #client<T>
            where
                T: #nats_micro::ClientTransport;

            fn client<T>(transport: T) -> Self::Client<T>
            where
                T: #nats_micro::ClientTransport,
            {
                #client::new(transport)
            }

            fn start(
                self,
                runtime: &mut #nats_micro::Runtime<#state_type>,
            ) -> impl ::std::future::Future<
                Output = ::std::result::Result<(), #nats_micro::StartError>
            > + Send {
                async move {
                    runtime.start_service(Self::SPEC).await?;
                    #(#starters)*
                    Ok(())
                }
            }

            #local_dispatch
            #local_consumer_dispatch
        }
    }
}
