use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;

use super::{MethodModel, OperationKind, ServiceModel, classify, dispatch, metadata};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let state_type = &model.state_type;
    let items = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind == OperationKind::Consumer)
        .map(|operation| {
            let operation_type = metadata::operation_type(model, operation);
            let spec = metadata::consumer_spec(operation, &model.args.name, &model.args.version);
            let bindings = dispatch::argument_bindings(model, operation);
            let call = dispatch::handler_call(model, operation);
            let ok_type = &operation.response.ok_type;
            let action = if classify::is_unit(ok_type) {
                quote!(#nats_micro::ConsumerAction::Ack)
            } else if classify::last_ident(ok_type).as_deref() == Some("ConsumerAction") {
                quote!(__response)
            } else {
                quote_spanned! {ok_type.span()=>
                    {
                        const _: () = {
                            fn assert_consumer_action<T: Into<#nats_micro::ConsumerAction>>() {}
                            let _ = assert_consumer_action::<#ok_type>;
                        };
                        __response.into()
                    }
                }
            };
            let attrs = dispatch::conditional_attrs(&operation.method);
            quote! {
                #(#attrs)*
                #[doc(hidden)]
                pub struct #operation_type;

                impl #nats_micro::ConsumerHandler<#state_type> for #operation_type {
                    const SPEC: #nats_micro::ConsumerSpec = #spec;

                    async fn call<'__request>(
                        state: &'__request #state_type,
                        request: #nats_micro::Request<'__request>,
                    ) -> ::std::result::Result<
                        #nats_micro::ConsumerAction,
                        #nats_micro::ErrorReply,
                    > {
                        #bindings
                        let __response = #call;
                        Ok(#action)
                    }
                }
            }
        });
    quote!(#(#items)*)
}
