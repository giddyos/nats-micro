use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{MethodModel, OperationKind, ServiceModel};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let client_ident = format_ident!("{}Client", model.service_ident);
    let service_ident = &model.service_ident;
    let operation_models: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| {
            matches!(
                operation.kind,
                OperationKind::Request | OperationKind::Publish
            )
        })
        .map(|operation| {
            let marker = format_ident!(
                "{}{}Operation",
                service_ident,
                operation.method.sig.ident.to_string().to_upper_camel_case()
            );
            let index = operation.operation_index.expect("operation index");
            quote! {
                #[derive(Debug, Clone, Copy, Default)]
                pub struct #marker;

                impl #marker {
                    pub const SPEC: &'static #nats_micro::OperationSpec =
                        &#service_ident::SPEC.operations[#index];
                }
            }
        })
        .collect();

    quote! {
        #[derive(Debug, Clone)]
        pub struct #client_ident<T> {
            transport: T,
        }

        impl<T> #client_ident<T> {
            #[must_use]
            pub const fn new(transport: T) -> Self {
                Self { transport }
            }

            #[must_use]
            pub const fn transport(&self) -> &T {
                &self.transport
            }

            #[must_use]
            pub fn into_transport(self) -> T {
                self.transport
            }
        }

        #(#operation_models)*
    }
}
