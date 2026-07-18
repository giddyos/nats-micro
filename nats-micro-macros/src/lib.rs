use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod app_state;
mod application;
mod message;
mod object;
mod runtime_main;
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
    runtime_main::expand(args.into(), &input.into()).into()
}

#[proc_macro_attribute]
pub fn test(args: TokenStream, input: TokenStream) -> TokenStream {
    test::expand_test(args.into(), input.into()).into()
}

#[proc_macro_attribute]
pub fn live_test(args: TokenStream, input: TokenStream) -> TokenStream {
    test::expand_live_test(args.into(), &input.into()).into()
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
