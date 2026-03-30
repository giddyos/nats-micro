use darling::FromMeta;
use proc_macro2::TokenStream;
use syn::{Expr, ItemFn};

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

pub fn expand_consumer(args: ConsumerArgs, func: ItemFn) -> TokenStream {
    let _ = args;
    crate::utils::error_stream(
        func.sig.ident.span(),
        "#[consumer] may only be used on methods inside a #[service_handlers] impl block",
        func,
    )
}
