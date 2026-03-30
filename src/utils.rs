use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};

pub fn nats_micro_path() -> syn::Path {
    match crate_name("nats-micro") {
        Ok(FoundCrate::Itself) => syn::parse_quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            syn::parse_str(&format!("::{name}")).unwrap_or_else(|_| syn::parse_quote!(::nats_micro))
        }
        Err(_) => syn::parse_quote!(::nats_micro),
    }
}

pub fn error_stream<T: ToTokens>(span: Span, msg: &str, original: T) -> TokenStream {
    let err = syn::Error::new(span, msg);
    let compile_error = err.to_compile_error();
    quote! {
        #compile_error
        #original
    }
}
