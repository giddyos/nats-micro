use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};

pub fn nats_micro_path() -> syn::Path {
    match crate_name("nats-micro") {
        Ok(FoundCrate::Itself) | Err(_) => syn::parse_quote!(::nats_micro),
        Ok(FoundCrate::Name(name)) => {
            syn::parse_str(&format!("::{name}")).unwrap_or_else(|_| syn::parse_quote!(::nats_micro))
        }
    }
}

pub fn nats_micro_serde_path() -> syn::LitStr {
    let value = match crate_name("nats-micro") {
        Ok(FoundCrate::Name(name)) => format!("{name}::serde"),
        Ok(FoundCrate::Itself) | Err(_) => "nats_micro::serde".to_owned(),
    };
    syn::LitStr::new(&value, Span::call_site())
}

pub fn error_stream<T: ToTokens>(span: Span, msg: &str, original: T) -> TokenStream {
    let compile_error = syn::Error::new(span, msg).to_compile_error();
    quote! {
        #compile_error
        #original
    }
}
