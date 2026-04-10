use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, ImplItemFn};

use crate::{
    endpoint::{build_handler_body, extract_param_info, requires_auth},
    service::GeneratedHandlerItem,
    utils::{conditional_attrs, nats_micro_path, parse_attr},
};

#[derive(Debug, Clone)]
pub(crate) struct ConsumerConfigExpr(pub Expr);

impl FromMeta for ConsumerConfigExpr {
    fn from_expr(value: &Expr) -> darling::Result<Self> {
        Ok(Self(value.clone()))
    }
}

#[derive(Debug, FromMeta)]
pub(crate) struct ConsumerArgs {
    pub stream: Option<String>,
    pub durable: Option<String>,
    pub concurrency_limit: Option<u64>,
    pub config: Option<ConsumerConfigExpr>,
}

pub(crate) fn process_consumer_method(
    struct_ident: &syn::Ident,
    method: &ImplItemFn,
    attr: &syn::Attribute,
) -> Result<GeneratedHandlerItem, TokenStream> {
    let nats_micro = nats_micro_path();
    let args = parse_attr::<ConsumerArgs>(attr)?;
    let fn_name = &method.sig.ident;
    let stream = args.stream.as_deref().unwrap_or("DEFAULT");
    let durable = args.durable.unwrap_or_else(|| fn_name.to_string());
    let auth_required = requires_auth(&method.sig);
    let concurrency_limit = optional_u64_tokens(args.concurrency_limit);
    let config = consumer_config_tokens(args.config, &nats_micro);
    let fn_path = quote! { #struct_ident::#fn_name };
    let handler = build_handler_body(&fn_path, &method.sig)?;
    let param_infos = extract_param_info(&method.sig)?;
    let attrs = conditional_attrs(method);
    let def_method_name = format_ident!("__con_{}", fn_name);
    let accessor_name = format_ident!("{}_consumer", fn_name);
    let fn_name_str = fn_name.to_string();

    Ok(GeneratedHandlerItem {
        attrs,
        def_fn: quote! {
            #[doc(hidden)]
            pub fn #def_method_name() -> #nats_micro::ConsumerDefinition {
                #nats_micro::ConsumerDefinition {
                    stream: #stream.to_string(),
                    durable: #durable.to_string(),
                    auth_required: #auth_required,
                    concurrency_limit: #concurrency_limit,
                    config: #config,
                    handler: #handler,
                }
            }
        },
        accessor_fn: quote! {
            pub fn #accessor_name() -> #nats_micro::ConsumerDefinition {
                Self::#def_method_name()
            }
        },
        def_call: quote! { Self::#def_method_name() },
        info_expr: quote! {
            #nats_micro::__macros::ConsumerInfo {
                fn_name: #fn_name_str.to_string(),
                stream: #stream.to_string(),
                durable: #durable.to_string(),
                auth_required: #auth_required,
                concurrency_limit: #concurrency_limit,
                params: vec![#(#param_infos),*],
            }
        },
    })
}

fn optional_u64_tokens(value: Option<u64>) -> TokenStream {
    if let Some(value) = value {
        quote! { Some(#value) }
    } else {
        quote! { None }
    }
}

fn consumer_config_tokens(
    config: Option<ConsumerConfigExpr>,
    nats_micro: &syn::Path,
) -> TokenStream {
    if let Some(ConsumerConfigExpr(expr)) = config {
        quote! {{
            let __config: #nats_micro::__macros::ConsumerConfig = (#expr);
            __config
        }}
    } else {
        quote! { #nats_micro::__macros::ConsumerConfig::default() }
    }
}

#[cfg(test)]
#[path = "tests/consumer_tests.rs"]
mod tests;
