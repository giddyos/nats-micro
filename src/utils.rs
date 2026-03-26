use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};

pub fn error_stream<T: ToTokens>(span: Span, msg: &str, original: T) -> TokenStream {
    let err = syn::Error::new(span, msg);
    let compile_error = err.to_compile_error();
    quote! {
        #compile_error
        #original
    }
}
