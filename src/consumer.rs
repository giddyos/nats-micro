use darling::FromMeta;
use proc_macro2::TokenStream;
use syn::ItemFn;

#[derive(Debug, FromMeta)]
pub(crate) struct ConsumerArgs {
    pub stream: Option<String>,
    pub durable: Option<String>,
    pub filter_subject: Option<String>,
    #[darling(default)]
    pub ack_on_success: bool,
}

pub fn expand_consumer(args: ConsumerArgs, func: ItemFn) -> TokenStream {
    let _ = args;
    crate::utils::error_stream(
        func.sig.ident.span(),
        "#[consumer] may only be used on methods inside a #[service_handlers] impl block",
        func,
    )
}
