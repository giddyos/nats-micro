use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemImpl, ItemStruct};

#[derive(Debug, FromMeta)]
pub struct ServiceArgs {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
}

pub fn expand_service(args: ServiceArgs, item_struct: ItemStruct) -> TokenStream {
    let ident = &item_struct.ident;
    let service_name = args.name.unwrap_or_else(|| item_struct.ident.to_string());
    let version = args.version.unwrap_or_else(|| "0.1.0".to_string());
    let description = args.description.unwrap_or_default();

    quote! {
        #item_struct

        impl #ident {
            pub fn __nats_micro_service_meta() -> ::nats_micro::ServiceMetadata {
                ::nats_micro::ServiceMetadata::new(
                    #service_name,
                    #version,
                    #description,
                )
            }
        }

        ::nats_micro::__private::inventory::submit! {
            ::nats_micro::__private::ServiceRegistration {
                constructor: #ident::__nats_micro_service_meta,
            }
        }
    }
}

pub fn expand_service_handlers(item_impl: ItemImpl) -> TokenStream {
    quote! { #item_impl }
}
