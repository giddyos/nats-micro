#![allow(unused)]

use darling::{FromMeta, ast::NestedMeta};
use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, ItemImpl, ItemStruct, parse_macro_input};

mod client;
mod consumer;
mod endpoint;
mod napi;
mod object;
mod service;
mod service_error;
mod utils;

fn parse_attr_args(attr: TokenStream) -> Result<Vec<NestedMeta>, TokenStream> {
    NestedMeta::parse_meta_list(attr.into())
        .map_err(|e| TokenStream::from(darling::Error::from(e).write_errors()))
}

#[proc_macro_attribute]
pub fn endpoint(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_args = match parse_attr_args(attr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let func = parse_macro_input!(item as ItemFn);
    match endpoint::EndpointArgs::from_list(&attr_args) {
        Ok(args) => endpoint::expand_endpoint(args, func).into(),
        Err(e) => e.write_errors().into(),
    }
}

#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_args = match parse_attr_args(attr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let item_struct = parse_macro_input!(item as ItemStruct);
    match service::ServiceArgs::from_list(&attr_args) {
        Ok(args) => service::expand_service(args, item_struct).into(),
        Err(e) => e.write_errors().into(),
    }
}

#[proc_macro_attribute]
pub fn object(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_struct = parse_macro_input!(item as ItemStruct);
    object::expand_object(item_struct).into()
}

#[proc_macro_attribute]
pub fn service_handlers(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_impl = parse_macro_input!(item as ItemImpl);
    service::expand_service_handlers(item_impl).into()
}

#[proc_macro_attribute]
pub fn service_error(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    service_error::expand_service_error(input).into()
}

#[proc_macro_attribute]
pub fn consumer(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr_args = match parse_attr_args(attr) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let func = parse_macro_input!(item as ItemFn);
    match consumer::ConsumerArgs::from_list(&attr_args) {
        Ok(args) => consumer::expand_consumer(args, func).into(),
        Err(e) => e.write_errors().into(),
    }
}
