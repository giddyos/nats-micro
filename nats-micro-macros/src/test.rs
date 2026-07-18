use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, ItemFn, Lit, Meta, Token, parse::Parser, punctuated::Punctuated};

use crate::util::nats_micro_path;

pub(crate) fn expand_test(args: TokenStream, input: TokenStream) -> TokenStream {
    if !cfg!(feature = "macros_test_util_feature") {
        return syn::Error::new_spanned(
            input,
            "#[nats_micro::test] requires the `test-util` feature",
        )
        .to_compile_error();
    }
    match expand_test_result(args, input) {
        Ok(output) => output,
        Err(error) => error.to_compile_error(),
    }
}

#[allow(clippy::too_many_lines)]
fn expand_test_result(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let mut function: ItemFn = syn::parse2(input)?;
    if function.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "#[nats_micro::test] requires an async function",
        ));
    }

    let options = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(args)?;
    let mut flavor = None;
    let mut worker_threads = None;
    let mut start_paused = None;
    for option in options {
        let name = option.path().get_ident().map(ToString::to_string);
        let Meta::NameValue(option) = option else {
            return Err(syn::Error::new_spanned(
                option,
                "test options use `name = value` syntax",
            ));
        };
        match name.as_deref() {
            Some("flavor") => {
                if flavor.is_some() {
                    return Err(syn::Error::new_spanned(option, "duplicate `flavor`"));
                }
                let Expr::Lit(expression) = option.value else {
                    return Err(syn::Error::new_spanned(option, "flavor requires a string"));
                };
                let Lit::Str(value) = expression.lit else {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "flavor requires a string",
                    ));
                };
                if !matches!(value.value().as_str(), "current_thread" | "multi_thread") {
                    return Err(syn::Error::new_spanned(
                        value,
                        "flavor must be `current_thread` or `multi_thread`",
                    ));
                }
                flavor = Some(value);
            }
            Some("worker_threads") => {
                if worker_threads.is_some() {
                    return Err(syn::Error::new_spanned(
                        option,
                        "duplicate `worker_threads`",
                    ));
                }
                let Expr::Lit(expression) = option.value else {
                    return Err(syn::Error::new_spanned(
                        option,
                        "worker_threads requires an integer",
                    ));
                };
                let Lit::Int(value) = expression.lit else {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "worker_threads requires an integer",
                    ));
                };
                if value.base10_parse::<usize>()? == 0 {
                    return Err(syn::Error::new_spanned(
                        value,
                        "worker_threads must be greater than zero",
                    ));
                }
                worker_threads = Some(value);
            }
            Some("start_paused") => {
                if start_paused.is_some() {
                    return Err(syn::Error::new_spanned(option, "duplicate `start_paused`"));
                }
                let Expr::Lit(expression) = option.value else {
                    return Err(syn::Error::new_spanned(
                        option,
                        "start_paused requires a boolean",
                    ));
                };
                let Lit::Bool(value) = expression.lit else {
                    return Err(syn::Error::new_spanned(
                        expression,
                        "start_paused requires a boolean",
                    ));
                };
                start_paused = Some(value);
            }
            _ => {
                return Err(syn::Error::new_spanned(option, "unknown test option"));
            }
        }
    }

    let flavor = flavor.unwrap_or_else(|| syn::parse_quote!("current_thread"));
    if worker_threads.is_some() && flavor.value() != "multi_thread" {
        return Err(syn::Error::new_spanned(
            worker_threads,
            "worker_threads requires `flavor = \"multi_thread\"`",
        ));
    }
    let nats_micro = nats_micro_path();
    let statements = function.block.stmts;
    function.block = syn::parse_quote!({
        #nats_micro::testing::init_test_tracing();
        #(#statements)*
    });
    let worker_threads = worker_threads.map(|value| quote!(, worker_threads = #value));
    let start_paused = start_paused.map(|value| quote!(, start_paused = #value));

    Ok(quote! {
        #[#nats_micro::tokio::test(
            flavor = #flavor
            #worker_threads
            #start_paused
        )]
        #function
    })
}

pub(crate) fn expand_live_test(args: TokenStream, input: &TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new_spanned(args, "live_test macro takes no arguments")
            .to_compile_error();
    }
    let nats_micro = nats_micro_path();
    quote! {
        #[#nats_micro::tokio::test]
        #input
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::expand_test_result;

    #[test]
    fn current_thread_is_the_default_and_tracing_is_initialized() {
        let output = expand_test_result(
            quote!(),
            quote! {
                async fn example() -> anyhow::Result<()> {
                    Ok(())
                }
            },
        )
        .unwrap()
        .to_string();
        assert!(output.contains("flavor = \"current_thread\""));
        assert!(output.contains("init_test_tracing"));
    }

    #[test]
    fn multi_thread_options_are_forwarded() {
        let output = expand_test_result(
            quote!(flavor = "multi_thread", worker_threads = 2),
            quote!(
                async fn example() {}
            ),
        )
        .unwrap()
        .to_string();
        assert!(output.contains("flavor = \"multi_thread\""));
        assert!(output.contains("worker_threads = 2"));
    }

    #[test]
    fn worker_threads_requires_multi_thread() {
        let error = expand_test_result(
            quote!(worker_threads = 2),
            quote!(
                async fn example() {}
            ),
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires"));
    }
}
