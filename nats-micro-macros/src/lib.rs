#![allow(special_module_name)]

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod app_state;
mod application;
mod main;
mod message;
mod object;
mod service;
mod service_error;
mod test;
mod util;

#[proc_macro_attribute]
pub fn service(args: TokenStream, input: TokenStream) -> TokenStream {
    service::expand(args.into(), input.into()).into()
}

#[proc_macro_attribute]
pub fn message(args: TokenStream, input: TokenStream) -> TokenStream {
    message::expand(args.into(), input.into()).into()
}

#[proc_macro_attribute]
pub fn service_error(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    service_error::expand_service_error(input).into()
}

#[proc_macro_derive(AppState, attributes(state))]
pub fn app_state(input: TokenStream) -> TokenStream {
    app_state::expand(parse_macro_input!(input as DeriveInput)).into()
}

#[proc_macro_attribute]
pub fn main(args: TokenStream, input: TokenStream) -> TokenStream {
    main::expand(args.into(), &input.into()).into()
}

#[proc_macro_attribute]
pub fn test(args: TokenStream, input: TokenStream) -> TokenStream {
    test::expand(args.into(), &input.into()).into()
}

#[proc_macro_attribute]
pub fn live_test(args: TokenStream, input: TokenStream) -> TokenStream {
    test::expand(args.into(), &input.into()).into()
}

#[proc_macro]
pub fn application(input: TokenStream) -> TokenStream {
    application::expand(input.into()).into()
}

#[proc_macro_attribute]
pub fn object(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemStruct);
    object::expand_object(&input).into()
}

#[proc_macro_attribute]
pub fn service_handlers(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input: proc_macro2::TokenStream = input.into();
    let error = syn::Error::new_spanned(
        &input,
        "#[service_handlers] was removed in v2; place #[service(...)] on the impl block",
    )
    .to_compile_error();
    quote::quote!(#error #input).into()
}
