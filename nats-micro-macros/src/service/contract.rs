use proc_macro2::TokenStream;
use quote::quote;

use super::ServiceModel;
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let service = &model.service_ident;
    let napi_hook = model.args.napi.then(|| {
        quote! {
            #[doc(hidden)]
            pub const __NAPI_OPERATION_MODELS: &'static [#nats_micro::OperationSpec] =
                Self::SPEC.operations;
        }
    });
    quote! {
        impl #service {
            #[must_use]
            pub const fn service_spec() -> &'static #nats_micro::ServiceSpec {
                &Self::SPEC
            }

            pub fn contract_json() -> ::std::result::Result<
                String,
                #nats_micro::serde_json::Error,
            > {
                #nats_micro::serde_json::to_string_pretty(&Self::SPEC)
            }

            #napi_hook
        }
    }
}
