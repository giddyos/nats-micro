use proc_macro2::TokenStream;
use quote::quote;
use syn::{Token, Type, parse::Parser, punctuated::Punctuated};

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    match Punctuated::<Type, Token![,]>::parse_terminated.parse2(input) {
        Ok(services) => {
            let services = services.iter();
            quote!((#(<#services as Default>::default()),*))
        }
        Err(error) => error.to_compile_error(),
    }
}
