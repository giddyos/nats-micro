use darling::FromMeta;
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ImplItem, ImplItemFn, ItemImpl, ItemStruct, Type};

use crate::client::{ClientEndpointSpec, generate_client_module};
use crate::consumer::process_consumer_method;
use crate::endpoint::process_endpoint_method;
use crate::napi::{gen_napi_asserts, generate_client_napi_module};
use crate::utils::nats_micro_path;

#[derive(Debug, FromMeta)]
pub struct ServiceArgs {
    name: String,
    version: String,
    description: Option<String>,
    prefix: Option<String>,
    #[cfg(feature = "macros_napi_feature")]
    #[darling(default)]
    napi: bool,
}

pub(crate) fn validate_service_args(
    args: &ServiceArgs,
    item_struct: &ItemStruct,
) -> syn::Result<()> {
    is_valid_service_version(&args.version)
        .then_some(())
        .ok_or_else(|| {
            syn::Error::new(
                item_struct.ident.span(),
                "service `version` must match x.x.x where each x is one or more ASCII digits",
            )
        })
}

fn is_valid_service_version(version: &str) -> bool {
    let mut parts = version.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(major), Some(minor), Some(patch), None)
            if [major, minor, patch]
                .into_iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    )
}

pub fn expand_service(args: ServiceArgs, item_struct: &ItemStruct) -> TokenStream {
    let nats_micro = nats_micro_path();
    let ident = &item_struct.ident;
    #[cfg(feature = "macros_napi_feature")]
    let ServiceArgs {
        name,
        version,
        description,
        prefix,
        napi,
    } = args;
    #[cfg(not(feature = "macros_napi_feature"))]
    let ServiceArgs {
        name,
        version,
        description,
        prefix,
    } = args;

    let service_name = name;
    let description = description.unwrap_or_default();
    let service_config_module = service_config_module_ident(ident);
    let prefix = if let Some(prefix) = prefix {
        quote! { Some(#prefix.to_string()) }
    } else {
        quote! { None }
    };

    #[cfg(feature = "macros_napi_feature")]
    let napi_enabled = napi;
    #[cfg(not(feature = "macros_napi_feature"))]
    let napi_enabled = false;

    let emit_napi_items = emit_napi_items_tokens(napi_enabled);

    quote! {
        #item_struct

        impl #ident {
            pub fn __nats_micro_service_meta() -> #nats_micro::ServiceMetadata {
                #nats_micro::ServiceMetadata::new(
                    #service_name,
                    #version,
                    #description,
                    #prefix,
                )
            }
        }

        #[doc(hidden)]
        mod #service_config_module {
            #emit_napi_items
        }
    }
}

pub(crate) struct GeneratedHandlerItem {
    pub attrs: Vec<syn::Attribute>,
    pub accessor_fn: TokenStream,
    pub accessor_call: TokenStream,
    pub info_expr: TokenStream,
}

#[derive(Default)]
struct ServiceExpansion {
    cleaned_items: Vec<ImplItem>,
    endpoint_fns: Vec<TokenStream>,
    endpoint_calls: Vec<TokenStream>,
    endpoint_info_exprs: Vec<TokenStream>,
    consumer_fns: Vec<TokenStream>,
    consumer_calls: Vec<TokenStream>,
    consumer_info_exprs: Vec<TokenStream>,
    client_endpoints: Vec<ClientEndpointSpec>,
}

impl ServiceExpansion {
    fn push_endpoint(
        &mut self,
        method: &ImplItemFn,
        attr_index: usize,
        generated: GeneratedHandlerItem,
        client_endpoint: ClientEndpointSpec,
    ) {
        self.cleaned_items.push(cleaned_method(method, attr_index));
        push_generated_handler(
            &mut self.endpoint_fns,
            &mut self.endpoint_calls,
            &mut self.endpoint_info_exprs,
            generated,
        );
        self.client_endpoints.push(client_endpoint);
    }

    fn push_consumer(
        &mut self,
        method: &ImplItemFn,
        attr_index: usize,
        generated: GeneratedHandlerItem,
    ) {
        self.cleaned_items.push(cleaned_method(method, attr_index));
        push_generated_handler(
            &mut self.consumer_fns,
            &mut self.consumer_calls,
            &mut self.consumer_info_exprs,
            generated,
        );
    }
}

pub fn expand_service_handlers(item_impl: &ItemImpl) -> TokenStream {
    let nats_micro = nats_micro_path();
    let struct_ty = &item_impl.self_ty;
    let Some(struct_ident) = extract_ident_from_type(struct_ty) else {
        return syn::Error::new_spanned(struct_ty, "expected a named struct type")
            .to_compile_error();
    };

    let expansion = match collect_service_items(&struct_ident, item_impl) {
        Ok(expansion) => expansion,
        Err(err) => return err,
    };
    let ServiceExpansion {
        cleaned_items,
        endpoint_fns,
        endpoint_calls,
        endpoint_info_exprs,
        consumer_fns,
        consumer_calls,
        consumer_info_exprs,
        client_endpoints,
    } = expansion;
    let mut cleaned_impl = item_impl.clone();
    cleaned_impl.items = cleaned_items;
    let client_module = render_client_module(&struct_ident, &client_endpoints);
    let napi_items = render_napi_items(&struct_ident, &client_endpoints);

    quote! {
        #cleaned_impl

        impl #struct_ident {
            #(#endpoint_fns)*
            #(#consumer_fns)*
        }

        impl #nats_micro::__macros::NatsService for #struct_ident {
            fn definition() -> #nats_micro::__macros::ServiceDefinition {
                #nats_micro::__macros::ServiceDefinition {
                    metadata: #struct_ident::__nats_micro_service_meta(),
                    endpoints: vec![#(#endpoint_calls),*],
                    consumers: vec![#(#consumer_calls),*],
                    endpoint_info: vec![#(#endpoint_info_exprs),*],
                    consumer_info: vec![#(#consumer_info_exprs),*],
                }
            }
        }

        #nats_micro::__macros::inventory::submit! {
            #nats_micro::__macros::ServiceRegistration {
                constructor: <#struct_ident as #nats_micro::__macros::NatsService>::definition,
            }
        }

        #client_module
        #napi_items
    }
}

fn extract_ident_from_type(ty: &Type) -> Option<syn::Ident> {
    if let Type::Path(type_path) = ty {
        type_path.path.segments.last().map(|s| s.ident.clone())
    } else {
        None
    }
}

fn service_config_module_ident(service_ident: &syn::Ident) -> syn::Ident {
    format_ident!(
        "__nats_micro_service_config_{}",
        service_ident.to_string().to_snake_case()
    )
}

fn emit_napi_items_tokens(napi_enabled: bool) -> TokenStream {
    if napi_enabled {
        quote! {
            #[allow(unused_macros)]
            macro_rules! emit_napi_items {
                ($($item:item)*) => {
                    $($item)*
                };
            }

            pub(crate) use emit_napi_items;
        }
    } else {
        quote! {
            #[allow(unused_macros)]
            macro_rules! emit_napi_items {
                ($($item:item)*) => {};
            }

            pub(crate) use emit_napi_items;
        }
    }
}

fn collect_service_items(
    struct_ident: &syn::Ident,
    item_impl: &ItemImpl,
) -> Result<ServiceExpansion, TokenStream> {
    let mut expansion = ServiceExpansion::default();

    for item in &item_impl.items {
        let ImplItem::Fn(method) = item else {
            expansion.cleaned_items.push(item.clone());
            continue;
        };

        if let Some(attr_index) = method
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident("endpoint"))
        {
            let attr = &method.attrs[attr_index];
            let (generated, client_endpoint) = process_endpoint_method(struct_ident, method, attr)?;
            expansion.push_endpoint(method, attr_index, generated, client_endpoint);
            continue;
        }

        if let Some(attr_index) = method
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident("consumer"))
        {
            let attr = &method.attrs[attr_index];
            let generated = process_consumer_method(struct_ident, method, attr)?;
            expansion.push_consumer(method, attr_index, generated);
            continue;
        }

        expansion.cleaned_items.push(item.clone());
    }

    Ok(expansion)
}

fn cleaned_method(method: &ImplItemFn, attr_index: usize) -> ImplItem {
    let mut cleaned = method.clone();
    cleaned.attrs.remove(attr_index);
    ImplItem::Fn(cleaned)
}

fn push_generated_handler(
    fns: &mut Vec<TokenStream>,
    calls: &mut Vec<TokenStream>,
    info_exprs: &mut Vec<TokenStream>,
    generated: GeneratedHandlerItem,
) {
    let GeneratedHandlerItem {
        attrs,
        accessor_fn,
        accessor_call,
        info_expr,
    } = generated;

    fns.push(quote! {
        #(#attrs)*
        #accessor_fn
    });
    calls.push(quote! {
        #(#attrs)*
        #accessor_call
    });
    info_exprs.push(quote! {
        #(#attrs)*
        #info_expr
    });
}

fn render_client_module(
    struct_ident: &syn::Ident,
    client_endpoints: &[ClientEndpointSpec],
) -> TokenStream {
    let service_name = struct_ident.to_string();
    if cfg!(feature = "macros_client_feature") {
        generate_client_module(struct_ident, &service_name, client_endpoints)
    } else {
        quote! {}
    }
}

fn render_napi_items(
    struct_ident: &syn::Ident,
    client_endpoints: &[ClientEndpointSpec],
) -> TokenStream {
    if !cfg!(feature = "macros_napi_feature") {
        return quote! {};
    }

    let service_name = struct_ident.to_string();
    let service_config_module = service_config_module_ident(struct_ident);
    let napi_asserts = gen_napi_asserts(client_endpoints);
    let napi_client_module =
        generate_client_napi_module(struct_ident, &service_name, client_endpoints);

    quote! {
        #service_config_module::emit_napi_items! {
            #napi_asserts
            #napi_client_module
        }
    }
}

#[cfg(test)]
#[path = "tests/service_tests.rs"]
mod tests;
