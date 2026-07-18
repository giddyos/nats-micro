use proc_macro2::TokenStream;
use quote::quote;

use crate::util::nats_micro_path;

pub(crate) fn expand(args: TokenStream, input: &TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new_spanned(args, "#[nats_micro::main] takes no arguments")
            .to_compile_error();
    }
    let nats_micro = nats_micro_path();
    quote! {
        #[#nats_micro::tokio::main]
        #input
    }
}
